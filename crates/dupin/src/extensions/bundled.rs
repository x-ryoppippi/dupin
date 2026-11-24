//! First-party extensions that ship with Dupin: `Contents/Resources/extensions` inside
//! Dupin.app, or the repository's `extensions/` for a build run from the source tree.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use dupin_js_runtime::Diagnostic;

/// Where the running Dupin's bundled extensions are, if it has any.
pub fn directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
    in_app(&executable).or_else(|| {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
        source.is_dir().then_some(source)
    })
}

/// `Dupin.app/Contents/Resources/extensions` for `Dupin.app/Contents/MacOS/<executable>`. An
/// app bundle never falls back to the source tree, even one without bundled extensions.
fn in_app(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.join("Resources/extensions"))
}

/// The bundled package called `name`, by the `name` in its package.json.
pub fn find(bundled: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(bundled).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .find(|folder| {
            super::manifest::Stamps::read(folder)
                .and_then(|stamps| super::manifest::read(&stamps))
                .is_ok_and(|manifest| manifest.id == name)
        })
}

/// Copies the bundled package `name` into `user`, the extensions folder, for the user to edit.
/// The copy appears in one rename, so a running Dupin never loads half of it.
pub fn eject(bundled: &Path, user: &Path, name: &str) -> Result<PathBuf, Diagnostic> {
    let source = find(bundled, name).ok_or_else(|| {
        Diagnostic::new(
            "not-bundled",
            format!("`{name}` isn't an extension bundled with Dupin"),
        )
        .hint("Settings → Extensions lists the bundled extensions by name.")
    })?;
    let target = user.join(name);
    if target.symlink_metadata().is_ok() {
        return Err(
            Diagnostic::new("already-exists", format!("{} already exists", target.display())).hint(
                "Edit that copy, or delete it to go back to the bundled extension before ejecting again.",
            ),
        );
    }
    let io = |error: std::io::Error| Diagnostic::new("io-error", error.to_string());
    std::fs::create_dir_all(user).map_err(io)?;
    let staging = user.join(format!(".eject-{name}-{}", std::process::id()));
    let copied = copy(&source, &staging).and_then(|()| std::fs::rename(&staging, &target));
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(error));
    }
    Ok(target)
}

/// Bundled files are read-only, so the copy gets the modes of a file the user wrote.
fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))?;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} is not a file or folder", entry.path().display()),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_finds_its_resources_and_a_source_build_its_repository() {
        assert_eq!(
            in_app(Path::new("/Applications/Dupin.app/Contents/MacOS/dupin")),
            Some(PathBuf::from(
                "/Applications/Dupin.app/Contents/Resources/extensions"
            ))
        );
        assert_eq!(in_app(Path::new("/repo/target/debug/dupin")), None);
        let source = directory().expect("the repository's extensions folder");
        assert!(find(&source, "dupin").is_some(), "{}", source.display());
    }

    #[test]
    fn every_bundled_extension_loads_and_the_dupin_one_answers() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let source = directory().unwrap();
        for entry in std::fs::read_dir(&source).unwrap() {
            let folder = entry.unwrap().path();
            if folder.is_dir() {
                crate::extensions::check_folder(&folder)
                    .unwrap_or_else(|diagnostic| panic!("{}: {diagnostic}", folder.display()));
            }
        }
        let dupin = crate::extensions::load_folder(&find(&source, "dupin").unwrap()).unwrap();
        let outcome = dupin
            .query(
                "report",
                None,
                10,
                std::time::Instant::now() + crate::extensions::QUERY_DEADLINE,
                || false,
            )
            .unwrap();
        let dupin_plugin_protocol::Outcome::Results { items, .. } = outcome else {
            panic!("{outcome:?}");
        };
        let views: Vec<_> = items.into_iter().map(|item| item.view).collect();
        assert_eq!(
            views,
            [dupin_plugin_protocol::View::Text {
                title: "Report a problem".into(),
                subtitle: "Open an issue on GitHub".into(),
            }]
        );
    }

    #[test]
    fn ejecting_copies_a_bundled_package_into_the_extensions_folder_once() {
        let bundled = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let package = bundled.path().join("folder");
        std::fs::create_dir_all(package.join("lib")).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name": "about", "version": "1.0.0", "dupin": {"title": "About", "keyword": "about"}}"#,
        )
        .unwrap();
        std::fs::write(package.join("main.js"), "export const search = () => [];").unwrap();
        std::fs::write(package.join("lib/rows.js"), "export const rows = [];").unwrap();
        for file in ["package.json", "main.js", "lib/rows.js"] {
            std::fs::set_permissions(package.join(file), std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        let extensions = user.path().join("extensions");

        let copy = eject(bundled.path(), &extensions, "about").unwrap();
        assert_eq!(copy, extensions.join("about"));
        assert_eq!(
            std::fs::read_to_string(copy.join("lib/rows.js")).unwrap(),
            "export const rows = [];"
        );
        let mode = copy.join("main.js").metadata().unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
        assert_eq!(
            std::fs::read_dir(&extensions).unwrap().count(),
            1,
            "no staging folder is left behind"
        );

        assert_eq!(
            eject(bundled.path(), &extensions, "about").unwrap_err().code,
            "already-exists"
        );
        assert_eq!(
            eject(bundled.path(), &extensions, "folder").unwrap_err().code,
            "not-bundled"
        );
    }
}
