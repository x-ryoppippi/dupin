//! User extensions in `extensions/<name>/` beside config.toml, each routed by one keyword: a
//! JavaScript package run by QuickJS-ng in the sandboxed extension host process, or a Wasm
//! guest. Guests get the query and return views; neither runtime gives them any host access.

mod provider;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime};

use dupin_core::Page;
use dupin_extension_host::Host;
use dupin_js_runtime::{DEFAULT_ENTRY, Diagnostic, Package, RunError};
use dupin_plugin_protocol as protocol;
use dupin_plugin_runtime::{GuestModule, HostError, MAX_MODULE_BYTES, QueryControl};
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
    entry: Option<String>,
}

pub struct Extension {
    pub id: String,
    pub folder_name: String,
    pub name: String,
    pub version: String,
    pub keyword: String,
    guest: Guest,
    /// Numbers each Wasm request, which the guest must echo back.
    requests: AtomicU64,
}

/// The manifest's `entry` decides the runtime: a `.wasm` module, or a JavaScript module.
pub(crate) enum Guest {
    Wasm(GuestModule),
    JavaScript(Package),
}

impl std::fmt::Debug for Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extension")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("keyword", &self.keyword)
            .field("runtime", &self.runtime())
            .finish_non_exhaustive()
    }
}

impl Extension {
    pub fn runtime(&self) -> &'static str {
        match self.guest {
            Guest::Wasm(_) => "wasm",
            Guest::JavaScript(_) => "javascript",
        }
    }

    /// Links a JavaScript package and checks that its entry exports `search`. A Wasm module was
    /// checked as far as it can be without a request when it loaded.
    pub fn check(&self, deadline: Instant) -> Result<(), RunError> {
        match &self.guest {
            Guest::Wasm(_) => Ok(()),
            Guest::JavaScript(package) => host().check(package, deadline, &|| false),
        }
    }

    /// Asks the guest for `query`, inside `page` if a push opened one.
    pub fn query(
        &self,
        query: &str,
        page: Option<&Page>,
        limit: usize,
        deadline: Instant,
        is_cancelled: impl Fn() -> bool + 'static,
    ) -> Result<protocol::Outcome, RunError> {
        match &self.guest {
            Guest::Wasm(module) => {
                let control = QueryControl {
                    deadline,
                    is_cancelled: &is_cancelled,
                };
                match module.query_controlled(&self.wire(query, page, limit), &control) {
                    Ok(response) => Ok(response.outcome),
                    Err(HostError::Cancelled) => Err(RunError::Cancelled),
                    Err(error) => Err(RunError::Failed(Diagnostic::new("wasm-error", error.to_string()))),
                }
            }
            Guest::JavaScript(_) if page.is_some() => Err(RunError::Failed(Diagnostic::new(
                "unsupported-view",
                "JavaScript extensions can't open views yet",
            ))),
            Guest::JavaScript(package) => {
                let items = host().search(package, query, limit, deadline, &is_cancelled)?;
                Ok(protocol::Outcome::Results { items, state: None })
            }
        }
    }

    fn wire(&self, query: &str, page: Option<&Page>, limit: usize) -> protocol::Request {
        let correlation = (self.requests.fetch_add(1, Ordering::Relaxed) + 1).to_string();
        protocol::Request {
            version: protocol::VERSION,
            request_id: correlation.clone(),
            generation: correlation,
            command: self.id.clone(),
            query: query.into(),
            limit: limit.min(protocol::MAX_RESULTS as usize) as u32,
            state: None,
            view: page.map(|page| page.view.clone()),
            context: page.and_then(|page| page.context.clone()),
        }
    }
}

/// The one extension host every JavaScript extension shares, started on the first query.
fn host() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(|| Host::new(Box::new(helper_command)))
}

/// Dupin's own executable as the extension host, with nothing inherited it doesn't need.
fn helper_command() -> std::io::Result<std::process::Command> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.env_clear().current_dir("/");
    #[cfg(not(test))]
    command.arg(dupin_extension_host::FLAG);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "extensions::tests::extension_host",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(tests::EXTENSION_HOST_ENV, "1");
    Ok(command)
}

/// What a scan of the extensions folder found: the valid extensions, sorted by folder name,
/// and one line for each folder that was skipped.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub extensions: Vec<Arc<Extension>>,
    pub problems: Vec<String>,
}

/// `$XDG_CONFIG_HOME/dupin/extensions`, beside config.toml.
pub fn directory() -> Option<PathBuf> {
    Some(crate::settings::path()?.parent()?.join("extensions"))
}

#[derive(Default)]
struct Scanner {
    loaded: HashMap<PathBuf, LoadedExtension>,
    previous: Option<Catalog>,
}

impl Scanner {
    fn update(&mut self, directory: &Path, watch_problem: Option<String>) -> Option<Catalog> {
        let mut catalog = self.scan(directory);
        catalog.problems.extend(watch_problem);
        if self.previous.as_ref().is_some_and(|previous| {
            previous.problems == catalog.problems
                && previous.extensions.len() == catalog.extensions.len()
                && previous
                    .extensions
                    .iter()
                    .zip(&catalog.extensions)
                    .all(|(old, new)| Arc::ptr_eq(old, new))
        }) {
            return None;
        }
        self.previous = Some(catalog.clone());
        Some(catalog)
    }

    fn scan(&mut self, directory: &Path) -> Catalog {
        let mut previous = std::mem::take(&mut self.loaded);
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
            match load(&folder, previous.remove(&folder)) {
                Ok(loaded) => {
                    let extension = &loaded.extension;
                    if catalog.extensions.iter().any(|e| e.id == extension.id) {
                        catalog.problems.push(format!(
                            "{}: id `{}` is used by another extension",
                            extension.folder_name, extension.id
                        ));
                    } else {
                        catalog.extensions.push(extension.clone());
                    }
                    self.loaded.insert(folder, loaded);
                }
                Err(problem) => catalog
                    .problems
                    .push(format!("{}: {problem}", folder_name(&folder))),
            }
        }
        catalog
    }
}

fn folder_name(folder: &Path) -> std::borrow::Cow<'_, str> {
    folder.file_name().unwrap_or_default().to_string_lossy()
}

struct LoadedExtension {
    manifest: FileStamp,
    entry: PathBuf,
    /// Every file the guest was built from: the Wasm module, or each JavaScript file.
    files: Vec<FileStamp>,
    extension: Arc<Extension>,
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

impl FileStamp {
    fn read(path: PathBuf) -> std::io::Result<Self> {
        let metadata = path.metadata()?;
        Ok(Self {
            path,
            len: metadata.len(),
            modified: metadata.modified()?,
        })
    }
}

/// Reads the extension in `folder` as the scanner does, for `dupin ext` to check or run.
pub fn load_folder(folder: &Path) -> Result<Arc<Extension>, Diagnostic> {
    load(folder, None).map(|loaded| loaded.extension)
}

fn load(folder: &Path, previous: Option<LoadedExtension>) -> Result<LoadedExtension, Diagnostic> {
    load_with(folder, previous, build_guest)
}

/// `build` reads the guest's files only after they are stamped, so an edit that lands while they
/// are read leaves a stamp that no longer matches, and the next scan loads the guest again.
fn load_with(
    folder: &Path,
    previous: Option<LoadedExtension>,
    build: impl FnOnce(&Path, &str, &[FileStamp]) -> Result<Guest, Diagnostic>,
) -> Result<LoadedExtension, Diagnostic> {
    let manifest_file = folder
        .join(MANIFEST)
        .canonicalize()
        .and_then(FileStamp::read)
        .map_err(|error| {
            Diagnostic::new("missing-manifest", format!("Couldn't read it: {error}"))
                .file(MANIFEST)
                .hint("Create extension.toml with `id`, `name`, `version` and `keyword`.")
        })?;
    if let Some(previous) = previous
        && previous.manifest == manifest_file
        && revision(folder, &previous.entry).is_ok_and(|files| files == previous.files)
    {
        return Ok(previous);
    }

    let source = read_capped(&manifest_file.path, MAX_MANIFEST_BYTES)
        .map_err(|error| Diagnostic::new("invalid-manifest", error).file(MANIFEST))?;
    let source = std::str::from_utf8(&source).map_err(|_| {
        Diagnostic::new("invalid-manifest", "is not UTF-8")
            .file(MANIFEST)
            .hint("Save extension.toml as UTF-8.")
    })?;
    let manifest: Manifest = toml::from_str(source).map_err(|error| toml_problem(source, &error))?;
    validate(&manifest)?;
    let entry_name = manifest.entry.as_deref().unwrap_or(DEFAULT_ENTRY);
    let entry = PathBuf::from(entry_name);
    let files = revision(folder, &entry)?;
    let guest = build(folder, entry_name, &files)?;
    Ok(LoadedExtension {
        manifest: manifest_file,
        entry,
        files,
        extension: Arc::new(Extension {
            id: manifest.id,
            folder_name: folder_name(folder).into_owned(),
            name: manifest.name,
            version: manifest.version,
            keyword: manifest.keyword,
            guest,
            requests: AtomicU64::new(0),
        }),
    })
}

fn build_guest(folder: &Path, entry_name: &str, files: &[FileStamp]) -> Result<Guest, Diagnostic> {
    if !is_wasm(Path::new(entry_name)) {
        return Package::load(folder, entry_name).map(Guest::JavaScript);
    }
    let bytes = read_capped(&files[0].path, MAX_MODULE_BYTES)
        .map_err(|error| Diagnostic::new("invalid-module", error).file(entry_name))?;
    GuestModule::load(&bytes)
        .map(Guest::Wasm)
        .map_err(|error| Diagnostic::new("invalid-module", error.to_string()).file(entry_name))
}

fn is_wasm(entry: &Path) -> bool {
    entry.extension().is_some_and(|extension| extension == "wasm")
}

/// The files the guest at `entry` would be built from now, to compare with a loaded revision.
fn revision(folder: &Path, entry: &Path) -> Result<Vec<FileStamp>, Diagnostic> {
    if is_wasm(entry) {
        Ok(vec![module_stamp(folder, entry)?])
    } else {
        stamps(
            dupin_js_runtime::source_files(folder)?
                .into_iter()
                .map(|(_, path)| path),
        )
    }
}

fn stamps(paths: impl Iterator<Item = PathBuf>) -> Result<Vec<FileStamp>, Diagnostic> {
    paths
        .map(|path| {
            FileStamp::read(path.clone()).map_err(|error| {
                Diagnostic::new("package-unreadable", format!("Couldn't read it: {error}"))
                    .file(path.display().to_string())
            })
        })
        .collect()
}

fn toml_problem(source: &str, error: &toml::de::Error) -> Diagnostic {
    let mut diagnostic = Diagnostic::new("invalid-manifest", error.message().trim_end())
        .file(MANIFEST)
        .hint("extension.toml takes `id`, `name`, `version`, `keyword` and optionally `entry`, all strings.");
    if let Some(span) = error.span() {
        let before = &source[..span.start.min(source.len())];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or_default().chars().count() + 1;
        diagnostic = diagnostic.at(line as u32, column as u32);
    }
    diagnostic
}

fn module_stamp(folder: &Path, entry: &Path) -> Result<FileStamp, Diagnostic> {
    let missing = || {
        Diagnostic::new("missing-entry", "is not a file in this folder")
            .file(entry.display().to_string())
            .hint("Point `entry` in extension.toml at a module inside the extension's folder.")
    };
    // A symlink must not lead the host into reading a module from outside the folder.
    let entry_path = match (folder.canonicalize(), folder.join(entry).canonicalize()) {
        (Ok(folder_path), Ok(entry_path)) if entry_path.starts_with(&folder_path) => entry_path,
        _ => return Err(missing()),
    };
    FileStamp::read(entry_path).map_err(|_| missing())
}

fn validate(manifest: &Manifest) -> Result<(), Diagnostic> {
    let invalid = |field: &str, message: &str, hint: &str| {
        Diagnostic::new("invalid-manifest", message)
            .file(MANIFEST)
            .field(field)
            .hint(hint)
    };
    let id = &manifest.id;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(invalid(
            "id",
            "must be 1–64 lowercase letters, digits or hyphens",
            "Use a name such as `word-count`.",
        ));
    }
    let printable = |value: &str| !value.chars().any(char::is_control);
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 80 || !printable(&manifest.name) {
        return Err(invalid(
            "name",
            "must be 1–80 printable characters",
            "Use the name Settings and the keyword chip should show, such as `Word Count`.",
        ));
    }
    if manifest.version.is_empty()
        || manifest.version.len() > 32
        || manifest
            .version
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(invalid(
            "version",
            "must be 1–32 characters without spaces",
            "Use a version such as `0.1.0`.",
        ));
    }
    if manifest.keyword.is_empty()
        || manifest.keyword.len() > 32
        || manifest
            .keyword
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(invalid(
            "keyword",
            "must be 1–32 bytes without spaces",
            "Use a short word such as `wc`.",
        ));
    }
    if let Some(entry) = &manifest.entry {
        let entry = Path::new(entry);
        let runnable = entry
            .extension()
            .is_some_and(|extension| extension == "wasm" || extension == "js" || extension == "mjs");
        if !runnable
            || !entry
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(invalid(
                "entry",
                "must be a relative path to a .js, .mjs or .wasm file inside the folder",
                "Leave `entry` out to use main.js.",
            ));
        }
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

/// Scans now and after changes under `directory`, delivering changed catalogues in order.
pub fn watch(directory: PathBuf, deliver: impl Fn(Catalog) + Send + 'static) -> std::io::Result<Watch> {
    let mut scanner = Scanner::default();
    Watch::start(
        directory,
        crate::catalog_watch::Depth::Trees,
        move |directory, problem| scanner.update(directory, problem),
        deliver,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    pub(super) const EXTENSION_HOST_ENV: &str = "DUPIN_EXTENSION_HOST_TEST";

    /// Not a test: [`helper_command`] starts this test binary, running only this function, as
    /// the extension host.
    #[test]
    fn extension_host() {
        if std::env::var_os(EXTENSION_HOST_ENV).is_some() {
            dupin_extension_host::serve();
        }
    }

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
            guest: Guest::Wasm(module),
            requests: AtomicU64::new(0),
        }
    }

    fn folder() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("dupin-extensions-")
            .tempdir()
            .unwrap()
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
        let fixture = folder();
        let root = fixture.path();
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
    }

    #[test]
    fn retargeted_extension_symlink_hop_is_rewatched_before_later_edits() {
        let fixture = folder();
        let root = fixture.path();
        let directory = root.join("extensions");
        let links = root.join("links");
        fs::create_dir(&directory).unwrap();
        fs::create_dir(&links).unwrap();
        let wasm = guest("{}");
        install(root, "first", &manifest("words", "first"), Some(&wasm));
        install(root, "second", &manifest("words", "second"), Some(&wasm));
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
    }

    #[test]
    fn moved_catalogue_directory_is_rewatched_before_later_edits() {
        let fixture = folder();
        let root = fixture.path();
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
    }

    #[test]
    fn a_missing_folder_is_an_empty_catalogue() {
        let catalog = Scanner::default().scan(&folder().path().join("absent"));
        assert!(catalog.extensions.is_empty());
        assert!(catalog.problems.is_empty());
    }

    #[test]
    fn catalogue_updates_reuse_unchanged_extensions_and_skip_unchanged_deliveries() {
        let fixture = folder();
        let root = fixture.path();
        let directory = root.join("extensions");
        let wasm = guest("{}");
        install(&directory, "one", &manifest("one", "first"), Some(&wasm));
        install(&directory, "two", &manifest("two", "second"), Some(&wasm));
        let mut scanner = Scanner::default();
        let initial = scanner.update(&directory, None).unwrap();

        fs::write(root.join("config.toml"), "extensions = true").unwrap();
        assert!(scanner.update(&directory, None).is_none());

        fs::write(directory.join("one/guest.wasm"), guest("{\"changed\":true}")).unwrap();
        let updated = scanner.update(&directory, None).unwrap();
        assert!(!Arc::ptr_eq(&initial.extensions[0], &updated.extensions[0]));
        assert!(Arc::ptr_eq(&initial.extensions[1], &updated.extensions[1]));

        fs::write(directory.join("two").join(MANIFEST), manifest("two", "reloaded")).unwrap();
        let reloaded = scanner.update(&directory, None).unwrap();
        assert!(Arc::ptr_eq(&updated.extensions[0], &reloaded.extensions[0]));
        assert!(!Arc::ptr_eq(&updated.extensions[1], &reloaded.extensions[1]));
        assert_eq!(reloaded.extensions[1].keyword, "reloaded");
        assert!(scanner.update(&directory, None).is_none());

        fs::remove_file(directory.join("one/guest.wasm")).unwrap();
        let removed = scanner.update(&directory, None).unwrap();
        assert_eq!(removed.extensions.len(), 1);
        assert!(Arc::ptr_eq(&reloaded.extensions[1], &removed.extensions[0]));
        assert!(removed.problems[0].starts_with("one:"));
        assert!(scanner.update(&directory, None).is_none());
    }

    #[test]
    fn valid_extensions_load_and_invalid_ones_are_reported_and_skipped() {
        let fixture = folder();
        let root = fixture.path();
        let wasm = guest("{}");
        install(root, "a-valid", &manifest("words", "wc"), Some(&wasm));
        install(root, "b-duplicate", &manifest("words", "wc2"), Some(&wasm));
        install(root, "c-bad-id", &manifest("Words!", "w"), Some(&wasm));
        install(root, "d-spaced-keyword", &manifest("spaced", "w c"), Some(&wasm));
        install(root, "e-missing-entry", &manifest("missing", "m"), None);
        install(
            root,
            "f-unknown-key",
            &format!("{}extra = 1\n", manifest("x", "x")),
            Some(&wasm),
        );
        install(
            root,
            "g-escape",
            &manifest("escape", "e").replace("guest.wasm", "../a-valid/guest.wasm"),
            None,
        );
        install(root, "h-not-wasm", &manifest("notwasm", "n"), Some(b"not wasm"));
        let imports = wat::parse_str(r#"(module (import "wasi" "fd_write" (func)))"#).unwrap();
        install(root, "i-imports", &manifest("imports", "i"), Some(&imports));
        std::fs::create_dir_all(root.join("j-no-manifest")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::os::unix::fs::symlink(root.join("a-valid/guest.wasm"), root.join("k-link.wasm")).unwrap();
        install(
            root,
            "k-symlink",
            &manifest("symlink", "s").replace("guest.wasm", "outside.wasm"),
            None,
        );
        std::os::unix::fs::symlink(root.join("k-link.wasm"), root.join("k-symlink/outside.wasm")).unwrap();

        let catalog = Scanner::default().scan(root);
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
            ("c-bad-id", "`id` must be"),
            ("d-spaced-keyword", "`keyword` must be"),
            ("e-missing-entry", "not a file in this folder"),
            ("f-unknown-key", "unknown field"),
            ("g-escape", "`entry` must be a relative path"),
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
    }

    #[test]
    fn oversized_manifests_are_rejected_before_parsing() {
        let fixture = folder();
        let root = fixture.path();
        install(root, "big", &"#".repeat(MAX_MANIFEST_BYTES + 1), None);
        let catalog = Scanner::default().scan(root);
        assert!(
            catalog.problems[0].contains("larger than 16 KiB"),
            "{:?}",
            catalog.problems
        );
    }

    fn install_javascript(root: &Path, folder: &str, id: &str, keyword: &str, files: &[(&str, &str)]) {
        let manifest =
            format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\nkeyword = \"{keyword}\"\n");
        install(root, folder, &manifest, None);
        for (name, source) in files {
            let path = root.join(folder).join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
    }

    #[test]
    fn javascript_and_wasm_extensions_load_side_by_side() {
        let fixture = folder();
        let root = fixture.path();
        install(root, "a-wasm", &manifest("wasm", "wa"), Some(&guest("{}")));
        install_javascript(
            root,
            "b-js",
            "js",
            "js",
            &[
                (
                    "main.js",
                    "import { title } from './lib/title.js'; export const search = () => [{ title }];",
                ),
                ("lib/title.js", "export const title = 'first';"),
            ],
        );
        install_javascript(
            root,
            "c-entry",
            "entry",
            "en",
            &[("src/index.mjs", "export function search() { return []; }")],
        );
        fs::write(
            root.join("c-entry").join(MANIFEST),
            "id = \"entry\"\nname = \"Entry\"\nversion = \"1\"\nkeyword = \"en\"\nentry = \"src/index.mjs\"\n",
        )
        .unwrap();
        install_javascript(
            root,
            "d-broken",
            "broken",
            "br",
            &[("main.js", "export function search( {")],
        );
        install_javascript(root, "e-no-main", "nomain", "nm", &[("index.js", "")]);

        let mut scanner = Scanner::default();
        let catalog = scanner.update(root, None).unwrap();
        let loaded: Vec<_> = catalog
            .extensions
            .iter()
            .map(|extension| (extension.id.as_str(), extension.runtime()))
            .collect();
        assert_eq!(
            loaded,
            [("wasm", "wasm"), ("js", "javascript"), ("entry", "javascript")]
        );
        let problems = catalog.problems.join("\n");
        assert!(problems.contains("d-broken: main.js:1:"), "{problems}");
        assert!(
            problems.contains("e-no-main: main.js: is not a JavaScript file in the package folder"),
            "{problems}"
        );
        assert_eq!(catalog.problems.len(), 2, "{problems}");
        assert!(scanner.update(root, None).is_none());

        // An edit to an imported file, not only to the entry, is a new revision.
        fs::write(
            root.join("b-js/lib/title.js"),
            "export const title = 'second edit';",
        )
        .unwrap();
        let updated = scanner.update(root, None).unwrap();
        assert!(Arc::ptr_eq(&catalog.extensions[0], &updated.extensions[0]));
        assert!(!Arc::ptr_eq(&catalog.extensions[1], &updated.extensions[1]));
        assert!(Arc::ptr_eq(&catalog.extensions[2], &updated.extensions[2]));
        let outcome = updated.extensions[1]
            .query("", None, 10, std::time::Instant::now() + QUERY_DEADLINE, || false)
            .unwrap();
        let protocol::Outcome::Results { items, .. } = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(
            items[0].view,
            protocol::View::Text {
                title: "second edit".into(),
                subtitle: String::new()
            }
        );

        fs::write(root.join("b-js/extra.js"), "export {};").unwrap();
        assert!(scanner.update(root, None).is_some());
    }

    #[test]
    fn an_edit_while_a_guest_is_read_is_picked_up_by_the_next_scan() {
        let fixture = folder();
        let root = fixture.path();
        install_javascript(
            root,
            "js",
            "js",
            "js",
            &[("main.js", "export const search = () => [{ title: 'old' }];")],
        );
        let wasm = guest("{}");
        install(root, "wasm", &manifest("wasm", "wa"), Some(&wasm));

        for (name, file, edit) in [
            (
                "js",
                "main.js",
                b"export const search = () => [{ title: 'new edit' }];".as_slice(),
            ),
            ("wasm", "guest.wasm", guest("{\"changed\":true}").as_slice()),
        ] {
            let folder = root.join(name);
            let edited = folder.join(file);
            let loaded = load_with(&folder, None, |folder, entry, files| {
                let guest = build_guest(folder, entry, files);
                fs::write(&edited, edit).unwrap();
                guest
            })
            .unwrap();
            let before = Arc::clone(&loaded.extension);
            let reloaded = load(&folder, Some(loaded)).unwrap();
            assert!(!Arc::ptr_eq(&before, &reloaded.extension), "{name}");
            let current = Arc::clone(&reloaded.extension);
            let unchanged = load(&folder, Some(reloaded)).unwrap();
            assert!(Arc::ptr_eq(&current, &unchanged.extension), "{name}");
        }
    }
}
