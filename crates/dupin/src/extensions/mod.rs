//! User extensions: Wasm guests in `extensions/<name>/` beside config.toml, each routed by one
//! keyword. Guests get the query and return views; they have no imports, so no host access.

mod provider;

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use dupin_plugin_runtime::{GuestModule, MAX_MODULE_BYTES};
use serde::Deserialize;

pub use provider::{ExtensionProvider, QUERY_DEADLINE};

pub(crate) use crate::catalog_watch::Watch;

pub const MANIFEST: &str = "extension.toml";
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
const MAX_EXTENSIONS: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    keyword: String,
    entry: String,
}

pub struct Extension {
    pub id: String,
    pub folder_name: String,
    pub name: String,
    pub version: String,
    pub keyword: String,
    module: GuestModule,
}

impl std::fmt::Debug for Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extension")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("keyword", &self.keyword)
            .finish_non_exhaustive()
    }
}

/// What a scan of the extensions folder found: the valid extensions, sorted by folder name,
/// and one line for each folder that was skipped.
#[derive(Debug, Default)]
pub struct Catalog {
    pub extensions: Vec<Arc<Extension>>,
    pub problems: Vec<String>,
}

/// `$XDG_CONFIG_HOME/dupin/extensions`, beside config.toml.
pub fn directory() -> Option<PathBuf> {
    Some(crate::settings::path()?.parent()?.join("extensions"))
}

impl Catalog {
    pub fn scan(directory: &Path) -> Catalog {
        let mut catalog = Catalog::default();
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return catalog,
            Err(error) => {
                catalog
                    .problems
                    .push(format!("Couldn't read {}: {error}", directory.display()));
                return catalog;
            }
        };
        let mut folders: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        folders.sort();
        if folders.len() > MAX_EXTENSIONS {
            catalog.problems.push(format!(
                "Only the first {MAX_EXTENSIONS} extension folders were read."
            ));
            folders.truncate(MAX_EXTENSIONS);
        }
        for folder in folders {
            let name = folder
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            match load(&folder) {
                Ok(extension) if catalog.extensions.iter().any(|e| e.id == extension.id) => {
                    catalog.problems.push(format!(
                        "{name}: id `{}` is used by another extension",
                        extension.id
                    ));
                }
                Ok(extension) => catalog.extensions.push(Arc::new(extension)),
                Err(problem) => catalog.problems.push(format!("{name}: {problem}")),
            }
        }
        catalog
    }
}

fn load(folder: &Path) -> Result<Extension, String> {
    let source = read_capped(&folder.join(MANIFEST), MAX_MANIFEST_BYTES)
        .map_err(|error| format!("{MANIFEST}: {error}"))?;
    let source = std::str::from_utf8(&source).map_err(|_| format!("{MANIFEST} is not UTF-8"))?;
    let manifest: Manifest =
        toml::from_str(source).map_err(|error| format!("{MANIFEST}: {}", error.message()))?;
    validate(&manifest)?;
    let entry = folder.join(&manifest.entry);
    // A symlink must not lead the host into reading a module from outside the folder.
    let inside = match (std::fs::canonicalize(folder), std::fs::canonicalize(&entry)) {
        (Ok(folder), Ok(entry)) => entry.starts_with(folder),
        _ => false,
    };
    if !inside {
        return Err(format!("entry `{}` is not a file in this folder", manifest.entry));
    }
    let bytes =
        read_capped(&entry, MAX_MODULE_BYTES).map_err(|error| format!("{}: {error}", manifest.entry))?;
    let module = GuestModule::load(&bytes).map_err(|error| format!("{}: {error}", manifest.entry))?;
    Ok(Extension {
        id: manifest.id,
        folder_name: folder
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        name: manifest.name,
        version: manifest.version,
        keyword: manifest.keyword,
        module,
    })
}

fn validate(manifest: &Manifest) -> Result<(), String> {
    let id = &manifest.id;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("id must be 1–64 lowercase letters, digits or hyphens".into());
    }
    let printable = |value: &str| !value.chars().any(char::is_control);
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 80 || !printable(&manifest.name) {
        return Err("name must be 1–80 printable characters".into());
    }
    if manifest.version.is_empty()
        || manifest.version.len() > 32
        || manifest
            .version
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err("version must be 1–32 characters without spaces".into());
    }
    if manifest.keyword.is_empty()
        || manifest.keyword.len() > 32
        || manifest
            .keyword
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err("keyword must be 1–32 bytes without spaces".into());
    }
    let entry = Path::new(&manifest.entry);
    if entry.extension().is_none_or(|extension| extension != "wasm")
        || !entry
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("entry must be a relative path to a .wasm file inside the folder".into());
    }
    Ok(())
}

fn read_capped(path: &Path, cap: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > cap {
        return Err(format!("larger than {} KiB", cap / 1024));
    }
    Ok(bytes)
}

/// Scans now and after every change under `directory`, delivering each catalogue in order.
/// Dropping it stops the watch; a scan already running still delivers once.
pub fn watch(directory: PathBuf, deliver: impl Fn(Catalog) + Send + 'static) -> std::io::Result<Watch> {
    Watch::start(directory, true, scan_for_watch, deliver)
}

fn scan_for_watch(directory: &Path, watch_problem: Option<String>) -> Catalog {
    let mut catalog = Catalog::scan(directory);
    catalog.problems.extend(watch_problem);
    catalog
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    pub(crate) fn guest(response: &str) -> Vec<u8> {
        let data: String = response.bytes().map(|byte| format!("\\{byte:02x}")).collect();
        wat::parse_str(format!(
            r#"(module
            (memory (export "memory") 8)
            (global $next (mut i32) (i32.const 1024))
            (func (export "dupin_abi_version") (result i32) i32.const 1)
            (func (export "dupin_alloc") (param i32) (result i32)
                global.get $next global.get $next local.get 0 i32.add global.set $next)
            (func (export "dupin_query") (param i32 i32 i32 i32) (result i32)
                local.get 2 i32.const 400000 i32.const {len} memory.copy i32.const {len})
            (data (i32.const 400000) "{data}"))"#,
            len = response.len()
        ))
        .unwrap()
    }

    pub(crate) fn module(response: &str) -> GuestModule {
        GuestModule::load(&guest(response)).unwrap()
    }

    pub(crate) fn extension(name: &str, keyword: &str, module: GuestModule) -> Extension {
        let id = name.to_lowercase().replace(' ', "-");
        Extension {
            folder_name: format!("{id}-folder"),
            id,
            name: name.into(),
            version: "1.0.0".into(),
            keyword: keyword.into(),
            module,
        }
    }

    fn folder() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "dupin-extensions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn install(root: &Path, folder: &str, manifest: &str, wasm: Option<&[u8]>) {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), manifest).unwrap();
        if let Some(wasm) = wasm {
            std::fs::write(dir.join("guest.wasm"), wasm).unwrap();
        }
    }

    fn manifest(id: &str, keyword: &str) -> String {
        format!(
            "id = \"{id}\"\nname = \"Word Count\"\nversion = \"0.1.0\"\nkeyword = \"{keyword}\"\nentry = \"guest.wasm\"\n"
        )
    }

    fn observe(directory: &Path) -> (Watch, mpsc::Receiver<Catalog>) {
        let (updates, received) = mpsc::channel();
        let watcher = watch(directory.to_owned(), move |catalog| {
            let _ = updates.send(catalog);
        })
        .unwrap();
        (watcher, received)
    }

    fn wait_for_keyword(received: &mpsc::Receiver<Catalog>, keyword: Option<&str>) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            let catalog = received
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("no catalogue with keyword {keyword:?}: {error}"));
            let keywords = catalog
                .extensions
                .iter()
                .map(|extension| extension.keyword.as_str());
            if keywords.eq(keyword) {
                assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
                return;
            }
        }
    }

    #[test]
    fn retargeted_catalogue_symlink_is_rewatched_before_later_edits() {
        let root = folder();
        let directory = root.join("extensions");
        let first = root.join("generation-one");
        let second = root.join("generation-two");
        let wasm = guest("{}");
        install(&first, "words", &manifest("words", "first"), Some(&wasm));
        install(&second, "words", &manifest("words", "second"), Some(&wasm));
        symlink(&first, &directory).unwrap();

        let (watcher, received) = observe(&directory);
        wait_for_keyword(&received, Some("first"));

        let replacement = root.join("replacement");
        symlink(&second, &replacement).unwrap();
        fs::rename(&replacement, &directory).unwrap();
        wait_for_keyword(&received, Some("second"));

        fs::write(second.join("words").join(MANIFEST), manifest("words", "edited")).unwrap();
        wait_for_keyword(&received, Some("edited"));
        drop(watcher);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retargeted_extension_symlink_hop_is_rewatched_before_later_edits() {
        let root = folder();
        let directory = root.join("extensions");
        let links = root.join("links");
        fs::create_dir(&directory).unwrap();
        fs::create_dir(&links).unwrap();
        let wasm = guest("{}");
        install(&root, "first", &manifest("words", "first"), Some(&wasm));
        install(&root, "second", &manifest("words", "second"), Some(&wasm));
        symlink("../first", links.join("current")).unwrap();
        symlink("../links/current", directory.join("words")).unwrap();

        let (watcher, received) = observe(&directory);
        wait_for_keyword(&received, Some("first"));

        let replacement = links.join("replacement");
        symlink("../second", &replacement).unwrap();
        fs::rename(&replacement, links.join("current")).unwrap();
        wait_for_keyword(&received, Some("second"));

        fs::write(root.join("second").join(MANIFEST), manifest("words", "edited")).unwrap();
        wait_for_keyword(&received, Some("edited"));
        drop(watcher);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn moved_catalogue_directory_is_rewatched_before_later_edits() {
        let root = folder();
        let directory = root.join("extensions");
        let wasm = guest("{}");
        install(&directory, "words", &manifest("words", "first"), Some(&wasm));

        let (watcher, received) = observe(&directory);
        wait_for_keyword(&received, Some("first"));

        fs::rename(&directory, root.join("moved")).unwrap();
        wait_for_keyword(&received, None);

        install(
            &directory,
            "words",
            &manifest("words", "replacement"),
            Some(&wasm),
        );
        wait_for_keyword(&received, Some("replacement"));

        fs::write(
            directory.join("words").join(MANIFEST),
            manifest("words", "edited"),
        )
        .unwrap();
        wait_for_keyword(&received, Some("edited"));
        drop(watcher);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_folder_is_an_empty_catalogue() {
        let catalog = Catalog::scan(&folder().join("absent"));
        assert!(catalog.extensions.is_empty());
        assert!(catalog.problems.is_empty());
    }

    #[test]
    fn valid_extensions_load_and_invalid_ones_are_reported_and_skipped() {
        let root = folder();
        let wasm = guest("{}");
        install(&root, "a-valid", &manifest("words", "wc"), Some(&wasm));
        install(&root, "b-duplicate", &manifest("words", "wc2"), Some(&wasm));
        install(&root, "c-bad-id", &manifest("Words!", "w"), Some(&wasm));
        install(&root, "d-spaced-keyword", &manifest("spaced", "w c"), Some(&wasm));
        install(&root, "e-missing-entry", &manifest("missing", "m"), None);
        install(
            &root,
            "f-unknown-key",
            &format!("{}extra = 1\n", manifest("x", "x")),
            Some(&wasm),
        );
        install(
            &root,
            "g-escape",
            &manifest("escape", "e").replace("guest.wasm", "../a-valid/guest.wasm"),
            None,
        );
        install(&root, "h-not-wasm", &manifest("notwasm", "n"), Some(b"not wasm"));
        let imports = wat::parse_str(r#"(module (import "wasi" "fd_write" (func)))"#).unwrap();
        install(&root, "i-imports", &manifest("imports", "i"), Some(&imports));
        std::fs::create_dir_all(root.join("j-no-manifest")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::os::unix::fs::symlink(root.join("a-valid/guest.wasm"), root.join("k-link.wasm")).unwrap();
        install(
            &root,
            "k-symlink",
            &manifest("symlink", "s").replace("guest.wasm", "outside.wasm"),
            None,
        );
        std::os::unix::fs::symlink(root.join("k-link.wasm"), root.join("k-symlink/outside.wasm")).unwrap();

        let catalog = Catalog::scan(&root);
        let ids: Vec<_> = catalog.extensions.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["words"]);
        let words = &catalog.extensions[0];
        assert_eq!(
            (words.name.as_str(), words.keyword.as_str()),
            ("Word Count", "wc")
        );
        let problems = catalog.problems.join("\n");
        for (folder, reason) in [
            ("b-duplicate", "used by another extension"),
            ("c-bad-id", "id must be"),
            ("d-spaced-keyword", "keyword must be"),
            ("e-missing-entry", "not a file in this folder"),
            ("f-unknown-key", "unknown field"),
            ("g-escape", "entry must be a relative path"),
            ("h-not-wasm", "invalid plugin module"),
            ("i-imports", "imports are unavailable"),
            ("j-no-manifest", "extension.toml"),
            ("k-symlink", "not a file in this folder"),
        ] {
            assert!(
                problems
                    .lines()
                    .any(|line| line.starts_with(folder) && line.contains(reason)),
                "{folder}: {reason}\n{problems}"
            );
        }
        assert_eq!(catalog.problems.len(), 10, "{problems}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_manifests_are_rejected_before_parsing() {
        let root = folder();
        install(&root, "big", &"#".repeat(MAX_MANIFEST_BYTES + 1), None);
        let catalog = Catalog::scan(&root);
        assert!(
            catalog.problems[0].contains("larger than 16 KiB"),
            "{:?}",
            catalog.problems
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
