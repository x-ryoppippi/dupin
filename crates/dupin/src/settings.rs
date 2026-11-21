//! Where `config.toml` lives and what happens when it's missing or broken.

use std::path::{Path, PathBuf};

use dupin_core::Config;

/// `$XDG_CONFIG_HOME/dupin/config.toml`, else `~/.config/dupin/config.toml`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("dupin/config.toml"))
}

/// Whether config.toml exists but can't be written, as when home-manager links it into the
/// Nix store. Settings then shows the values without offering to change them.
pub fn read_only() -> bool {
    path().is_some_and(|path| is_read_only(&path))
}

fn is_read_only(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly())
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

/// Write `config` to config.toml on a thread of its own. Comments in a hand-written file are
/// not kept.
pub fn save(config: &dupin_core::Config) {
    let toml = match toml::to_string(config) {
        Ok(toml) => toml,
        Err(e) => return crate::log::warn(&format!("couldn't serialize the settings: {e}")),
    };
    std::thread::spawn(move || {
        let written = path()
            .ok_or_else(|| std::io::Error::other("HOME is unset"))
            .and_then(|path| write(&path, &toml));
        if let Err(e) = written {
            crate::log::warn(&format!("couldn't save the settings: {e}"));
        }
    });
}

/// Rename a new file over the old one so a crash leaves the old one whole. A symlinked
/// config.toml is written where it points, so the link, and whoever manages it, stays.
fn write(path: &Path, toml: &str) -> std::io::Result<()> {
    if is_read_only(path) {
        return Err(std::io::Error::other(format!("{} is read-only", path.display())));
    }
    let target = match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.to_owned(),
        Err(e) => return Err(e),
    };
    let dir = target
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = target.with_extension("toml.tmp");
    std::fs::write(&tmp, toml)?;
    std::fs::rename(tmp, target)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dupin-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creates_a_missing_file() {
        let dir = scratch("missing");
        let path = dir.join("dupin/config.toml");
        write(&path, "a = 1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\n");
    }

    #[test]
    fn writes_through_a_symlink() {
        let dir = scratch("symlink");
        let real = dir.join("dotfiles.toml");
        std::fs::write(&real, "a = 1\n").unwrap();
        let path = dir.join("config.toml");
        symlink(&real, &path).unwrap();

        write(&path, "a = 2\n").unwrap();

        assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "a = 2\n");
    }

    #[test]
    fn leaves_a_read_only_file_alone() {
        let dir = scratch("read-only");
        let real = dir.join("store.toml");
        std::fs::write(&real, "a = 1\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o444)).unwrap();
        let path = dir.join("config.toml");
        symlink(&real, &path).unwrap();

        assert!(is_read_only(&path));
        assert!(write(&path, "a = 2\n").is_err());
        assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "a = 1\n");
    }

    #[test]
    fn a_missing_file_is_writable() {
        assert!(!is_read_only(&scratch("absent").join("config.toml")));
    }
}
