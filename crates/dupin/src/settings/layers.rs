use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use dupin_core::Config;
use toml::Table;

pub type LockedKeys = BTreeSet<String>;

#[derive(Clone, Default)]
pub struct Paths {
    pub base: Option<PathBuf>,
    pub overlay: Option<PathBuf>,
}

impl Paths {
    pub fn discover() -> Self {
        Self {
            base: super::path(),
            overlay: crate::store::data_dir().map(|dir| dir.join("settings.toml")),
        }
    }

    pub(super) fn read(&self) -> Result<Layers, String> {
        let base_read_only = self.base.as_ref().is_some_and(|path| !super::writable(path));
        let base = read_layer(self.base.as_deref(), "config.toml")?;
        let overlay = if base_read_only {
            read_layer(self.overlay.as_deref(), "settings.toml")?
        } else {
            None
        };
        Ok(Layers {
            base_read_only,
            base,
            overlay,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Layers {
    pub base_read_only: bool,
    base: Option<String>,
    overlay: Option<String>,
}

pub(super) struct Effective {
    pub config: Config,
    pub locked_keys: LockedKeys,
    pub overlay: Table,
}

impl Layers {
    pub fn effective(&self) -> Result<Effective, String> {
        let base = self.base.as_deref().unwrap_or_default();
        let mut config = Config::parse(base).map_err(|e| parse_problem("config.toml", base, &e))?;
        let mut locked_keys = LockedKeys::new();
        let mut overlay = Table::new();
        if self.base_read_only {
            let base = top_level(base, "config.toml")?;
            locked_keys.extend(base.keys().cloned());
            if let Some(source) = &self.overlay {
                Config::parse(source).map_err(|e| parse_problem("settings.toml", source, &e))?;
                overlay = top_level(source, "settings.toml")?;
                let mut merged = overlay.clone();
                merged.extend(base);
                config = merged.try_into().map_err(|_| "could not merge settings.toml")?;
            }
        }
        Ok(Effective {
            config,
            locked_keys,
            overlay,
        })
    }
}

pub fn load(paths: &Paths) -> (Config, LockedKeys, Option<String>) {
    match paths.read().and_then(|layers| layers.effective()) {
        Ok(effective) => (effective.config, effective.locked_keys, None),
        Err(error) => {
            let problem = format!("{error}; using the defaults");
            crate::log::warn(&problem);
            (Config::default(), LockedKeys::new(), Some(problem))
        }
    }
}

pub fn read_layer(path: Option<&Path>, name: &str) -> Result<Option<String>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    match super::read_source(path) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not read {name} ({:?})", error.kind())),
    }
}

pub fn top_level(source: &str, name: &str) -> Result<Table, String> {
    source.parse().map_err(|e| parse_problem(name, source, &e))
}

fn parse_problem(name: &str, source: &str, error: &toml::de::Error) -> String {
    let Some(span) = error.span() else {
        return format!("{name} is invalid (location unavailable)");
    };
    let (line, column) = super::location(source, span.start);
    format!("{name} is invalid at line {line}, column {column}")
}
