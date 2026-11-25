//! Configuration files and their live settings service.

use std::path::{Path, PathBuf};

use dupin_core::Config;

mod layers;
mod live;
pub mod overlay;
pub use layers::{LockedKeys, Paths, load, read_layer, read_overlay, top_level};
pub use live::{Service, Update, check_locked};

/// `$XDG_CONFIG_HOME/dupin/config.toml`, else `~/.config/dupin/config.toml`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("dupin/config.toml"))
}

/// Whether `write` can replace the file `path` resolves to: the file itself, if it exists,
/// and the directory the new file is renamed in, or the nearest one that exists to make it in.
pub fn writable(path: &Path) -> bool {
    let Ok(target) = resolve(path) else {
        return false;
    };
    if target.exists() && !can_write(&target) {
        return false;
    }
    target
        .ancestors()
        .skip(1)
        .find(|dir| dir.exists())
        .is_some_and(can_write)
}

/// Whether this process may write `path`, by its effective user and groups; the mode bits
/// alone say nothing about who owns the file.
fn can_write(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
}

/// Follow `path` through any symlinks to the file they name, which need not exist yet.
fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    resolve_with(path, |_| {})
}

pub(super) fn resolve_with(path: &Path, mut visit: impl FnMut(&Path)) -> std::io::Result<PathBuf> {
    let mut path = path.to_owned();
    'links: for _ in 0..40 {
        visit(&path);
        let mut prefix = PathBuf::new();
        let mut components = path.components();
        while let Some(component) = components.next() {
            prefix.push(component.as_os_str());
            match std::fs::read_link(&prefix) {
                Ok(target) => {
                    visit(&prefix);
                    let target = match prefix.parent() {
                        Some(dir) => dir.join(target),
                        None => target,
                    };
                    path = if components.as_path().as_os_str().is_empty() {
                        target
                    } else {
                        target.join(components.as_path())
                    };
                    continue 'links;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidInput
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        return Ok(path);
    }
    Err(std::io::Error::other(format!(
        "{} has too many links",
        path.display()
    )))
}

fn read_source(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "configuration must be a regular file",
        ));
    }
    let mut source = String::new();
    file.read_to_string(&mut source)?;
    Ok(source)
}

/// Whether `source` would be accepted as config.toml: it parses, and its hotkey is one the
/// launcher can register.
pub fn check(source: &str) -> Result<Config, String> {
    let config = Config::parse(source).map_err(|error| match error.span() {
        Some(span) => {
            let (line, column) = location(source, span.start);
            format!("line {line}, column {column}: {}", error.message())
        }
        None => error.message().to_owned(),
    })?;
    crate::composition::check(&config.advanced)?;
    crate::hotkey::parse(&config.hotkey).map_err(|_| format!("the hotkey {:?} is invalid", config.hotkey))?;
    crate::hotkey::parse(&config.selection_hotkey)
        .map_err(|_| format!("the selection hotkey {:?} is invalid", config.selection_hotkey))?;
    Ok(config)
}

/// The one-based line and character column of byte `offset` in `source`.
fn location(source: &str, offset: usize) -> (usize, usize) {
    let mut start = offset.min(source.len());
    while !source.is_char_boundary(start) {
        start -= 1;
    }
    let prefix = &source[..start];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map_or(1, |line| line.chars().count() + 1);
    (line, column)
}

/// Rename a new file over the old one so a crash leaves the old one whole. A symlinked
/// config.toml is written where it points, so the link, and whoever manages it, stays. That
/// folder may be one other users can write, so the new file takes a name no one can have
/// put a link at. It keeps the old file's mode, so a file the user made private stays so.
fn write(path: &Path, toml: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    if !writable(path) {
        return Err(std::io::Error::other(format!("{} is read-only", path.display())));
    }
    let target = resolve(path)?;
    let dir = target
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent directory"))?;
    std::fs::create_dir_all(dir)?;
    let mut staged = tempfile::Builder::new()
        .prefix(&format!(
            ".{}.",
            target.file_name().unwrap_or_default().to_string_lossy()
        ))
        .suffix(".tmp")
        .tempfile_in(dir)?;
    match std::fs::symlink_metadata(&target) {
        Ok(old) => staged
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(old.permissions().mode() & 0o777))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    staged.write_all(toml.as_bytes())?;
    staged.persist(target).map(drop).map_err(|error| error.error)
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
    fn loads_config_with_a_missing_script_binding_at_startup() {
        let dir = scratch("missing-script");
        let path = dir.join("config.toml");
        let source = r#"
file_limit = 37
[bindings."alt+cmd+t"]
target = "script"
script = "missing.sh"
"#;
        std::fs::write(&path, source).unwrap();

        let (config, _, problem) = load(&Paths {
            base: Some(path),
            overlay: None,
        });
        assert_eq!(config, Config::parse(source).unwrap());
        assert_eq!(problem, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_composition_naming_what_dupin_lacks_is_invalid_where_it_is_read() {
        let dir = scratch("invalid-composition");
        let path = dir.join("config.toml");
        let source = "file_limit = 37\n[advanced.sources.apps]\nmatcher = \"builtin.prefx\"\n";
        std::fs::write(&path, source).unwrap();
        let (config, _, problem) = load(&Paths {
            base: Some(path),
            overlay: None,
        });
        assert_eq!(config, Config::default());
        let problem = problem.unwrap();
        assert!(
            problem.starts_with(
                "config.toml is invalid: [advanced.sources.apps]: `builtin.prefx` isn't a matcher Dupin has"
            ),
            "{problem}"
        );
        assert!(
            check(source)
                .unwrap_err()
                .contains("`builtin.prefx` isn't a matcher")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_only_regular_files_including_through_links() {
        use std::os::unix::ffi::OsStrExt;

        let dir = scratch("regular-source");
        let real = dir.join("regular.toml");
        std::fs::write(&real, "file_limit = 30\n").unwrap();
        let link = dir.join("config.toml");
        symlink(&real, &link).unwrap();
        assert_eq!(read_source(&link).unwrap(), "file_limit = 30\n");
        assert!(read_source(&dir).is_err());
        let fifo = dir.join("pipe");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(
            read_source(&fifo).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn check_locates_problems_and_rejects_unusable_hotkeys() {
        assert_eq!(check("file_limit = 30\n").unwrap().file_limit, 30);
        let problem = check("file_limit = 30\nfile_limit = \"é\"\n").unwrap_err();
        assert!(problem.starts_with("line 2, column 1: "), "{problem}");
        assert!(check("hotkey = \"Cmd+Nope\"\n").unwrap_err().contains("hotkey"));
        assert!(check("hotkey = \"\"\n").is_ok());
        assert_eq!(location("é\nab", 4), (2, 2));
        assert_eq!(location("é", 1), (1, 1));
    }

    #[test]
    fn creates_a_missing_file() {
        let dir = scratch("missing");
        let path = dir.join("dupin/config.toml");
        write(&path, "a = 1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\n");
    }

    #[test]
    fn saving_config_preserves_custom_settings() {
        let config =
            dupin_core::Config::parse("dictionary_keywords = [\"word\", \"lookup\"]\ndisplay = \"active\"")
                .unwrap();
        let toml = toml::to_string(&config).unwrap();
        let path = scratch("dictionary-keywords").join("config.toml");
        write(&path, &toml).unwrap();

        let loaded = dupin_core::Config::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(loaded, config);
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
    fn keeps_the_mode_of_the_file_it_replaces() {
        let dir = scratch("mode");
        let fresh = dir.join("fresh/config.toml");
        write(&fresh, "a = 1\n").unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&fresh), 0o600);

        let real = dir.join("dotfiles.toml");
        let path = dir.join("config.toml");
        std::fs::write(&real, "a = 1\n").unwrap();
        symlink(&real, &path).unwrap();
        for kept in [0o600, 0o640, 0o644] {
            std::fs::set_permissions(&real, std::fs::Permissions::from_mode(kept)).unwrap();
            write(&path, "a = 2\n").unwrap();
            assert_eq!(mode(&real), kept);
        }
    }

    #[test]
    fn never_writes_through_a_link_beside_the_target() {
        let dir = scratch("planted-link");
        let real = dir.join("dotfiles.toml");
        std::fs::write(&real, "a = 1\n").unwrap();
        let path = dir.join("config.toml");
        symlink(&real, &path).unwrap();
        let victim = dir.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        let planted = real.with_extension("toml.tmp");
        symlink(&victim, &planted).unwrap();

        write(&path, "a = 2\n").unwrap();

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert!(std::fs::symlink_metadata(&planted).unwrap().is_symlink());
        assert!(std::fs::symlink_metadata(&real).unwrap().is_file());
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

        assert!(!writable(&path));
        assert!(write(&path, "a = 2\n").is_err());
        assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "a = 1\n");
    }

    #[test]
    fn a_missing_file_is_writable() {
        assert!(writable(&scratch("absent").join("dupin/config.toml")));
    }

    #[test]
    fn creates_the_target_of_a_dangling_relative_symlink() {
        let dir = scratch("dangling");
        std::fs::create_dir(dir.join("dotfiles")).unwrap();
        let path = dir.join("config.toml");
        symlink("dotfiles/dupin.toml", &path).unwrap();

        write(&path, "a = 1\n").unwrap();

        assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
        assert_eq!(
            std::fs::read_to_string(dir.join("dotfiles/dupin.toml")).unwrap(),
            "a = 1\n"
        );
    }

    #[test]
    fn a_writable_file_in_a_locked_directory_is_read_only() {
        let dir = scratch("locked-dir");
        let locked = dir.join("locked");
        std::fs::create_dir(&locked).unwrap();
        let path = locked.join("config.toml");
        std::fs::write(&path, "a = 1\n").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();

        let result = writable(&path);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!result);
    }
}
