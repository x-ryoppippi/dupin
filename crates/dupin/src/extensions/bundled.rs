//! First-party extensions that ship with Dupin: `Contents/Resources/extensions` inside
//! Dupin.app, or the repository's `extensions/` for a build run from the source tree. Only a
//! folder whose files match what this build sealed is loaded as bundled.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use dupin_js_runtime::Diagnostic;

include!(concat!(env!("OUT_DIR"), "/bundled_seals.rs"));

/// The content hash of each bundled package folder, by folder name, as the build found them.
#[derive(Clone, Debug)]
pub struct Seals(Vec<(String, String)>);

impl Seals {
    pub fn shipped() -> Self {
        Self(
            SHIPPED
                .iter()
                .map(|(name, hash)| ((*name).to_owned(), (*hash).to_owned()))
                .collect(),
        )
    }

    #[cfg(test)]
    pub(crate) fn of(folders: &[&Path]) -> Self {
        Self(
            folders
                .iter()
                .map(|folder| {
                    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
                    (name, dupin_extension_install::content_hash(folder).unwrap())
                })
                .collect(),
        )
    }

    /// Why the folder must not load as bundled, or `None` when it is one this build shipped,
    /// byte for byte.
    pub fn check(&self, folder: &Path) -> Option<String> {
        let name = folder.file_name()?.to_string_lossy();
        let Some((_, expected)) = self.0.iter().find(|(shipped, _)| *shipped == name) else {
            return Some("isn't an extension this build of Dupin shipped, so it stays off".into());
        };
        match dupin_extension_install::content_hash(folder) {
            Ok(hash) if hash == *expected => None,
            Ok(_) => {
                Some("its files differ from the ones this build of Dupin shipped, so it stays off".into())
            }
            Err(error) => Some(format!("Couldn't check its files, so it stays off: {error}")),
        }
    }
}

/// Where the running Dupin's bundled extensions are, if it has any.
pub fn directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
    in_app(&executable)
        .or_else(|| installed(&executable).filter(|folder| folder.is_dir()))
        .or_else(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
            source.is_dir().then_some(source)
        })
}

/// `share/dupin/extensions` for `bin/<executable>`, as a package outside an app bundle, such
/// as the latency build, installs them.
fn installed(executable: &Path) -> Option<PathBuf> {
    let bin = executable.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    Some(bin.parent()?.join("share/dupin/extensions"))
}

/// `Dupin.app/Contents/Resources/extensions` for `Dupin.app/Contents/MacOS/<executable>`. An
/// app bundle never falls back to the source tree, even one without bundled extensions.
fn in_app(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.join("Resources/extensions"))
}

/// Each bundled package's folder, with the `name` and keyword in its package.json.
pub fn packages(bundled: &Path) -> Vec<(PathBuf, String, String)> {
    let Ok(entries) = std::fs::read_dir(bundled) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|folder| {
            let manifest = super::manifest::Stamps::read(&folder)
                .and_then(|stamps| super::manifest::read(&stamps))
                .ok()?;
            Some((folder, manifest.id, manifest.keyword))
        })
        .collect()
}

/// The bundled package called `name`, by the `name` in its package.json.
pub fn find(bundled: &Path, name: &str) -> Option<PathBuf> {
    packages(bundled)
        .into_iter()
        .find(|(_, id, _)| id == name)
        .map(|(folder, ..)| folder)
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
        assert_eq!(
            installed(Path::new("/nix/store/x-dupin-latency/bin/dupin")),
            Some(PathBuf::from("/nix/store/x-dupin-latency/share/dupin/extensions"))
        );
        assert_eq!(installed(Path::new("/repo/target/debug/dupin")), None);
        let source = directory().expect("the repository's extensions folder");
        assert!(find(&source, "dupin").is_some(), "{}", source.display());
    }

    #[test]
    fn this_build_sealed_every_bundled_package_as_it_is_in_the_repository() {
        let source = directory().unwrap();
        let seals = Seals::shipped();
        let mut folders = 0;
        for entry in std::fs::read_dir(&source).unwrap() {
            let folder = entry.unwrap().path();
            if folder.is_dir() && !folder.file_name().unwrap().to_string_lossy().starts_with('.') {
                assert_eq!(seals.check(&folder), None, "{}", folder.display());
                folders += 1;
            }
        }
        assert_eq!(seals.0.len(), folders);
    }

    #[test]
    fn a_folder_the_build_did_not_seal_is_refused() {
        let bundled = tempfile::tempdir().unwrap();
        let about = bundled.path().join("about");
        std::fs::create_dir(&about).unwrap();
        std::fs::write(about.join("main.js"), "export const search = () => [];").unwrap();
        let seals = Seals::of(&[&about]);
        assert_eq!(seals.check(&about), None);

        std::fs::write(about.join("main.js"), "export const search = () => [1];").unwrap();
        assert!(seals.check(&about).unwrap().contains("differ"));
        let added = bundled.path().join("added");
        std::fs::create_dir(&added).unwrap();
        assert!(seals.check(&added).unwrap().contains("isn't an extension"));
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
        let items = dupin
            .query(
                "report",
                None,
                10,
                std::time::Instant::now() + crate::extensions::QUERY_DEADLINE,
                || false,
                &(std::sync::Arc::new(dupin_extension_host::NoServices) as _),
            )
            .unwrap();
        let shown: Vec<_> = items.into_iter().map(|row| (row.title, row.subtitle)).collect();
        assert_eq!(
            shown,
            [(
                "Report a problem".to_owned(),
                "Open an issue on GitHub".to_owned()
            )]
        );
    }

    #[test]
    fn every_bundled_extension_passes_its_fixtures() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let source = directory().unwrap();
        let mut tested = Vec::new();
        for (folder, name, _) in packages(&source) {
            if folder.join("fixtures").is_dir() {
                let report = crate::extensions::test(&folder, false, &|| false).unwrap();
                assert!(report.passed, "{}:\n{}", folder.display(), report.text);
                tested.push(name);
            }
        }
        assert!(tested.iter().any(|name| name == "dictionary"), "{tested:?}");
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
