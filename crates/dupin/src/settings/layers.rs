use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dupin_core::Config;
use rune::Shortcut;
use toml::Table;

pub type LockedKeys = BTreeSet<String>;

#[derive(Clone, Default)]
pub struct Paths {
    pub base: Option<PathBuf>,
    /// Where Settings saves over a read-only base.
    pub overlay: Option<Arc<super::overlay::Overlay>>,
}

impl Paths {
    pub fn discover() -> Self {
        Self {
            base: super::path(),
            overlay: super::overlay::shared(),
        }
    }

    pub(super) fn read(&self) -> Result<Layers, String> {
        let base_read_only = self.base.as_ref().is_some_and(|path| !super::writable(path));
        let base = read_layer(self.base.as_deref(), "config.toml")?;
        let overlay = match &self.overlay {
            Some(overlay) if base_read_only => read_overlay(overlay)?,
            _ => None,
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
    /// config.toml as it was read, empty when there is none.
    pub fn base_source(&self) -> &str {
        self.base.as_deref().unwrap_or_default()
    }

    pub fn effective(&self) -> Result<Effective, String> {
        let base = self.base_source();
        let mut config = Config::parse(base).map_err(|e| parse_problem("config.toml", base, &e))?;
        let mut locked_keys = LockedKeys::new();
        let mut overlay = Table::new();
        if self.base_read_only {
            let base = top_level(base, "config.toml")?;
            locked_keys.extend(base.keys().cloned());
            if let Some(source) = &self.overlay {
                Config::parse(source).map_err(|e| parse_problem("the stored settings", source, &e))?;
                overlay = top_level(source, "the stored settings")?;
                let mut merged = overlay.clone();
                merged.extend(base);
                config = merged
                    .try_into()
                    .map_err(|_| "could not merge the stored settings")?;
            }
        }
        check_navigation(&config).map_err(|problem| format!("config.toml is invalid: {problem}"))?;
        crate::composition::check(&config.advanced)
            .map_err(|problem| format!("config.toml is invalid: {problem}"))?;
        Ok(Effective {
            config,
            locked_keys,
            overlay,
        })
    }
}

fn check_navigation(config: &Config) -> Result<(), String> {
    let parse = |key: &str, value: &str| {
        if value.trim().is_empty() {
            Ok(None)
        } else {
            value
                .parse::<Shortcut>()
                .map(Some)
                .map_err(|_| format!("`{key}` must be a valid key or key chord"))
        }
    };
    let up = parse("navigation_up", &config.navigation_up)?;
    let down = parse("navigation_down", &config.navigation_down)?;
    if up.iter().chain(down.iter()).any(|shortcut| {
        matches!(
            shortcut.physical_key(),
            rune::PhysicalKeyCode::ArrowUp | rune::PhysicalKeyCode::ArrowDown
        )
    }) {
        return Err("`navigation_up` and `navigation_down` cannot override the arrow keys".into());
    }
    if up.is_some() && up == down {
        return Err("`navigation_up` and `navigation_down` must use different keys".into());
    }
    Ok(())
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

/// The stored overlay as TOML, from the running launcher when it holds the overlay, as it does
/// whenever this is another process, such as `dupin ext install`.
pub fn read_overlay(overlay: &super::overlay::Overlay) -> Result<Option<String>, String> {
    match overlay.read() {
        Ok(source) => Ok(source),
        Err(dupin_storage::Error::Busy) => {
            crate::control::exchange(crate::control::Command::ConfigOverlay {})
                .and_then(|source| serde_json::from_value(source).map_err(|error| error.to_string()))
                .map_err(|error| format!("could not read the stored settings Dupin holds ({error})"))
        }
        Err(error) => Err(format!("could not read the stored settings ({error})")),
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

#[cfg(test)]
mod tests {
    use super::check_navigation;
    use dupin_core::Config;

    #[test]
    fn navigation_accepts_the_physical_shortcut_format_recorded_by_settings() {
        let config = Config {
            navigation_down: "ctrl+KeyJ".into(),
            ..Config::default()
        };
        assert!(check_navigation(&config).is_ok());
    }

    #[test]
    fn navigation_does_not_allow_overriding_native_arrow_keys() {
        let config = Config {
            navigation_up: "down".into(),
            ..Config::default()
        };
        assert!(check_navigation(&config).is_err());
    }
}
