use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dupin_js_runtime::Diagnostic;
use serde_json::Value;

use crate::manifest::{MANIFEST_FILE, PackageInfo, read_package};

const BUILD_DEADLINE: Duration = Duration::from_secs(120);
const MAX_REPORTED_ERROR_BYTES: usize = 900;

/// Node's built-in modules (`require('node:module').builtinModules`). None exists in the guest,
/// and bundlers stub or polyfill some of them silently for the browser, so an import of one is
/// found in the bundler's metafile and fails the build instead.
const NODE_BUILTINS: &[&str] = &[
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundlerKind {
    Bun,
    Esbuild,
}

impl BundlerKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Esbuild => "esbuild",
        }
    }
}

/// A bundler already installed on the author's machine. Dupin never downloads one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundler {
    pub(crate) kind: BundlerKind,
    program: PathBuf,
}

impl Bundler {
    /// Bun if it is on `PATH`, otherwise esbuild.
    pub fn find() -> Option<Self> {
        Self::all_on(&std::env::var_os("PATH").unwrap_or_default())
            .into_iter()
            .next()
    }

    /// Every supported bundler on the search path `path`, Bun first.
    pub(crate) fn all_on(path: &OsStr) -> Vec<Self> {
        [BundlerKind::Bun, BundlerKind::Esbuild]
            .into_iter()
            .filter_map(|kind| {
                std::env::split_paths(path)
                    .map(|dir| dir.join(kind.name()))
                    .find(|program| is_executable(program))
                    .map(|program| Self { kind, program })
            })
            .collect()
    }

    fn arguments(&self, source: &str, outfile: &Path, metafile: &Path, sourcemap: bool) -> Vec<OsString> {
        let flag = |flag: &str, value: &Path| {
            let mut arg = OsString::from(flag);
            arg.push(value);
            arg
        };
        let (fixed, external): (&[&str], &str) = match self.kind {
            BundlerKind::Bun => (
                &["build", source, "--format=esm", "--target=browser"],
                "--external=",
            ),
            BundlerKind::Esbuild => (
                &[
                    source,
                    "--bundle",
                    "--format=esm",
                    "--platform=browser",
                    "--log-level=error",
                ],
                "--external:",
            ),
        };
        let mut args: Vec<OsString> = fixed.iter().map(OsString::from).collect();
        match self.kind {
            // Bun 1.4 writes a linked source map build to the working directory, ignoring the
            // folder `--outfile` names; `--outdir` is honored.
            BundlerKind::Bun => {
                args.push(flag("--outdir=", outfile.parent().unwrap_or(Path::new("."))));
                args.push(flag(
                    "--entry-naming=",
                    Path::new(outfile.file_name().unwrap_or_default()),
                ));
            }
            BundlerKind::Esbuild => args.push(flag("--outfile=", outfile)),
        }
        args.push(flag("--metafile=", metafile));
        if sourcemap {
            args.push("--sourcemap=linked".into());
        }
        args.extend(externals().map(|name| OsString::from(format!("{external}{name}"))));
        args
    }
}

/// `dupin` is the host's module. Node built-ins stay imports so that esbuild, which would fail
/// to resolve them, records them in the metafile like Bun does.
fn externals() -> impl Iterator<Item = String> {
    std::iter::once("dupin".to_owned())
        .chain(NODE_BUILTINS.iter().map(|name| (*name).to_owned()))
        .chain(std::iter::once("node:*".to_owned()))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildOptions {
    /// Writes `<main>.map` beside the bundle and links it from the bundle.
    pub sourcemap: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    pub package: PackageInfo,
    /// The bundler that ran, or `None` for a package whose `dupin.main` is its source.
    pub bundler: Option<BundlerKind>,
    /// `dupin.main`, relative to the package.
    pub output: String,
    pub sourcemap: Option<String>,
}

/// Bundles the package in `dir` into `dupin.main` when `package.json` names a `source`, then
/// runs `validate` on the package. A failed bundle or validation puts back whatever `dupin.main`
/// held before, so a broken build never replaces a working entry.
pub fn build(
    dir: &Path,
    bundler: Option<&Bundler>,
    options: BuildOptions,
    validate: &dyn Fn(&Path) -> Result<(), Diagnostic>,
) -> Result<Built, Diagnostic> {
    let package = read_package(dir)?;
    if !package.needs_build() {
        validate(dir)?;
        return Ok(Built {
            output: package.main.clone(),
            package,
            bundler: None,
            sourcemap: None,
        });
    }
    let bundler = bundler.ok_or_else(missing_bundler)?;
    let outfile = dir.join(&package.main);
    let map = map_path(&outfile);
    let previous = [&outfile, &map].map(|path| std::fs::read(path).ok());
    let result = bundle(dir, &package, bundler, &outfile, options).and_then(|()| validate(dir));
    if let Err(error) = result {
        for (path, contents) in [&outfile, &map].into_iter().zip(previous) {
            let _ = match contents {
                Some(contents) => std::fs::write(path, contents),
                None => std::fs::remove_file(path),
            };
        }
        return Err(error);
    }
    Ok(Built {
        output: package.main.clone(),
        sourcemap: options.sourcemap.then(|| format!("{}.map", package.main)),
        package,
        bundler: Some(bundler.kind),
    })
}

pub(crate) fn missing_bundler() -> Diagnostic {
    Diagnostic::new(
        "bundler-missing",
        "Neither `bun` nor `esbuild` is on PATH, and `source` needs bundling",
    )
    .file(MANIFEST_FILE)
    .field("source")
    .hint("Install Bun (https://bun.sh) or esbuild; Dupin never downloads one.")
}

fn map_path(outfile: &Path) -> PathBuf {
    let mut map = outfile.as_os_str().to_owned();
    map.push(".map");
    PathBuf::from(map)
}

/// Runs `bundler` in `dir` to write the package's `source` and its dependencies into `outfile`
/// as one ES module.
pub(crate) fn bundle(
    dir: &Path,
    package: &PackageInfo,
    bundler: &Bundler,
    outfile: &Path,
    options: BuildOptions,
) -> Result<(), Diagnostic> {
    static BUILDS: AtomicU64 = AtomicU64::new(0);
    let source = package.source.as_deref().unwrap_or(&package.main);
    if !dir.join(source).is_file() {
        return Err(Diagnostic::new("missing-source", "is not a file in the package")
            .file(source)
            .hint("Point `source` in package.json at the module to bundle, such as `src/main.ts`."));
    }
    let unwritable = |error: std::io::Error| {
        Diagnostic::new("unwritable", format!("Couldn't create its folder: {error}")).file(&package.main)
    };
    // The bundler runs in `dir`, so a path relative to this process would be read from there.
    let outfile = std::path::absolute(outfile).map_err(unwritable)?;
    if let Some(parent) = outfile.parent() {
        std::fs::create_dir_all(parent).map_err(unwritable)?;
    }
    let metafile = std::env::temp_dir().join(format!(
        "dupin-build-{}-{}.json",
        std::process::id(),
        BUILDS.fetch_add(1, Ordering::Relaxed)
    ));
    let outcome = run(Command::new(&bundler.program)
        .args(bundler.arguments(source, &outfile, &metafile, options.sourcemap))
        .current_dir(dir));
    let metadata = std::fs::read(&metafile);
    let _ = std::fs::remove_file(&metafile);
    let stderr = outcome.map_err(|error| {
        Diagnostic::new(
            "build-failed",
            format!("Couldn't run {}: {error}", bundler.kind.name()),
        )
        .file(source)
    })?;
    if let Some(stderr) = stderr {
        return Err(Diagnostic::new(
            "build-failed",
            format!("{} failed: {stderr}", bundler.kind.name()),
        )
        .file(source)
        .hint("Fix the error above and build again."));
    }
    let metadata: Value = metadata
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| {
            Diagnostic::new(
                "build-failed",
                format!("{} wrote no metafile", bundler.kind.name()),
            )
            .file(source)
            .hint("Update Bun or esbuild to a version that supports `--metafile`.")
        })?;
    if let Some((importer, builtin)) = node_builtin_import(&metadata) {
        return Err(Diagnostic::new(
            "node-builtin",
            format!("imports the Node built-in `{builtin}`, which Dupin extensions don't have"),
        )
        .file(importer)
        .hint(
            "Use a dependency that runs in a browser, or drop it. Extensions reach the system through Dupin, not Node.",
        ));
    }
    if !outfile.is_file() {
        return Err(
            Diagnostic::new("build-failed", format!("{} wrote nothing", bundler.kind.name()))
                .file(&package.main),
        );
    }
    Ok(())
}

/// Waits for `command`. `Ok(None)` means it succeeded, `Ok(Some(stderr))` that it failed.
fn run(command: &mut Command) -> std::io::Result<Option<String>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut bytes);
        bytes
    });
    let deadline = Instant::now() + BUILD_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stderr = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    match status {
        Some(status) if status.success() => Ok(None),
        Some(_) => Ok(Some(summarize(&stderr))),
        None => Ok(Some(format!(
            "it took longer than {} seconds",
            BUILD_DEADLINE.as_secs()
        ))),
    }
}

fn summarize(stderr: &str) -> String {
    let text = stderr.trim();
    if text.is_empty() {
        return "it exited with an error".into();
    }
    if text.len() <= MAX_REPORTED_ERROR_BYTES {
        return text.into();
    }
    let mut end = MAX_REPORTED_ERROR_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The first input file in an esbuild or Bun metafile that imports a Node built-in, and that
/// built-in as written.
pub(crate) fn node_builtin_import(metadata: &Value) -> Option<(String, String)> {
    let inputs = metadata.get("inputs")?.as_object()?;
    let mut names: Vec<&String> = inputs.keys().collect();
    names.sort();
    names.into_iter().find_map(|name| {
        inputs[name]
            .get("imports")?
            .as_array()?
            .iter()
            .find_map(|import| {
                ["original", "path"]
                    .into_iter()
                    .filter_map(|key| import.get(key)?.as_str())
                    .find(|specifier| is_node_builtin(specifier))
                    .map(|specifier| (name.clone(), specifier.to_owned()))
            })
    })
}

fn is_node_builtin(specifier: &str) -> bool {
    specifier.starts_with("node:") || NODE_BUILTINS.contains(&specifier)
}
