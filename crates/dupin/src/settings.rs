//! Where `config.toml` lives and what happens when it's missing or broken.

use std::path::PathBuf;

use dupin_core::Config;

/// `$XDG_CONFIG_HOME/dupin/config.toml`, else `~/.config/dupin/config.toml`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("dupin/config.toml"))
}

/// The config, or the defaults with a message saying why the file wasn't used.
pub fn load() -> (Config, Option<String>) {
    let Some(path) = path() else {
        return (Config::default(), None);
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(src) => src,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Config::default(), None),
        Err(e) => {
            return problem(format!(
                "couldn't read {}: {e}; using the defaults",
                path.display()
            ));
        }
    };
    match Config::parse(&src) {
        Ok(config) => (config, None),
        Err(e) => problem(format!(
            "{} is invalid, using the defaults: {}",
            path.display(),
            e.message()
        )),
    }
}

fn problem(message: String) -> (Config, Option<String>) {
    crate::log::warn(&message);
    (Config::default(), Some(message))
}

/// Write `config` to config.toml on a thread of its own, renaming over the file so a crash
/// leaves the old one whole. Comments in a hand-written file are not kept.
pub fn save(config: &dupin_core::Config) {
    let toml = match toml::to_string(config) {
        Ok(toml) => toml,
        Err(e) => return crate::log::warn(&format!("couldn't serialize the settings: {e}")),
    };
    std::thread::spawn(move || {
        let write = || -> std::io::Result<()> {
            let path = path().ok_or_else(|| std::io::Error::other("HOME is unset"))?;
            let dir = path
                .parent()
                .ok_or_else(|| std::io::Error::other("no parent directory"))?;
            std::fs::create_dir_all(dir)?;
            let tmp = path.with_extension("toml.tmp");
            std::fs::write(&tmp, toml)?;
            std::fs::rename(tmp, path)
        };
        if let Err(e) = write() {
            crate::log::warn(&format!("couldn't save the settings: {e}"));
        }
    });
}
