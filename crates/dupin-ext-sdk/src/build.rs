use dupin_spawn::{Command, Stdio};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dupin_js_runtime::Diagnostic;
use serde_json::Value;

use crate::manifest::{MANIFEST_FILE, PackageInfo, read_package};

const BUILD_DEADLINE: Duration = Duration::from_secs(120);
const MAX_REPORTED_ERROR_BYTES: usize = 900;
const PROGRAM: &str = "rolldown";
const CONFIG_PREFIX: &str = "rolldown.config.";
/// The file `rolldown.config.ts` names to report a Node built-in in, so the diagnostic can point
/// at the module that imports it.
const REPORT_VARIABLE: &str = "DUPIN_BUILD_REPORT";

/// What `dupin ext new --bundle` writes as `rolldown.config.ts`, and what a package without a
/// Rolldown configuration of its own is bundled with.
pub(crate) const CONFIG: &str = include_str!("../templates/rolldown.config.ts");

/// The Rolldown CLI already installed for a package. Dupin never downloads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rolldown {
    pub(crate) program: PathBuf,
}

impl Rolldown {
    /// The package's own `node_modules/.bin/rolldown`, otherwise `rolldown` on `PATH`.
    pub fn find(package_dir: &Path) -> Option<Self> {
        Self::find_on(package_dir, &std::env::var_os("PATH").unwrap_or_default())
    }

    pub(crate) fn find_on(package_dir: &Path, path: &OsStr) -> Option<Self> {
        let local = std::path::absolute(package_dir.join("node_modules/.bin").join(PROGRAM)).ok();
        local
            .into_iter()
            .chain(std::env::split_paths(path).map(|dir| dir.join(PROGRAM)))
            .find(|program| is_executable(program))
            .map(|program| Self { program })
    }
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
    /// The Rolldown that ran, or `None` for a package whose `dupin.main` is its source.
    pub rolldown: Option<PathBuf>,
    /// `dupin.main`, relative to the package.
    pub output: String,
    pub sourcemap: Option<String>,
}

/// Bundles the package in `dir` into `dupin.main` when `package.json` names a `source`, then
/// runs `validate` on the package. A failed bundle or validation puts back whatever `dupin.main`
/// held before, so a broken build never replaces a working entry.
pub fn build(
    dir: &Path,
    rolldown: Option<&Rolldown>,
    options: BuildOptions,
    validate: &dyn Fn(&Path) -> Result<(), Diagnostic>,
) -> Result<Built, Diagnostic> {
    let package = read_package(dir)?;
    if !package.needs_build() {
        validate(dir)?;
        return Ok(Built {
            output: package.main.clone(),
            package,
            rolldown: None,
            sourcemap: None,
        });
    }
    let rolldown = rolldown.ok_or_else(missing_rolldown)?;
    let outfile = dir.join(&package.main);
    let map = map_path(&outfile);
    let previous = [&outfile, &map].map(|path| std::fs::read(path).ok());
    let result = bundle(dir, &package, rolldown, &outfile, options).and_then(|()| validate(dir));
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
        rolldown: Some(rolldown.program.clone()),
    })
}

pub(crate) fn missing_rolldown() -> Diagnostic {
    Diagnostic::new(
        "bundler-missing",
        "Rolldown is neither in the package's node_modules nor on PATH, and `source` needs bundling",
    )
    .file(MANIFEST_FILE)
    .field("source")
    .hint("Add it to the package with `bun add -d rolldown` or `pnpm add -D rolldown`; Dupin never downloads a bundler.")
}

fn map_path(outfile: &Path) -> PathBuf {
    let mut map = outfile.as_os_str().to_owned();
    map.push(".map");
    PathBuf::from(map)
}

/// The package's own `rolldown.config.*`, relative to it. One the loader would read as
/// extension code is refused, because its Node imports would fail every check and every load.
fn own_config(dir: &Path) -> Result<Option<String>, Diagnostic> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(CONFIG_PREFIX))
        .collect();
    found.sort();
    if let Some(name) = found.iter().find(|name| dupin_js_runtime::is_source_name(name)) {
        return Err(Diagnostic::new(
            "config-is-extension-code",
            "is JavaScript, so Dupin would load it as part of the extension",
        )
        .file(name)
        .hint("Rename it to `rolldown.config.ts`: Dupin loads every `.js` and `.mjs` file in the package."));
    }
    Ok(found.into_iter().next())
}

/// Runs Rolldown in `dir` to write the package's `source` and its dependencies into `outfile`
/// as one ES module, with the package's own Rolldown configuration or else [`CONFIG`].
pub(crate) fn bundle(
    dir: &Path,
    package: &PackageInfo,
    rolldown: &Rolldown,
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
    // Rolldown runs in `dir`, so a path relative to this process would be read from there.
    let outfile = std::path::absolute(outfile).map_err(unwritable)?;
    if let Some(parent) = outfile.parent() {
        std::fs::create_dir_all(parent).map_err(unwritable)?;
    }
    let scratch = std::env::temp_dir().join(format!(
        "dupin-build-{}-{}",
        std::process::id(),
        BUILDS.fetch_add(1, Ordering::Relaxed)
    ));
    let report = scratch.with_extension("json");
    let fallback = scratch.with_extension("config.ts");
    let config = match own_config(dir)? {
        Some(name) => PathBuf::from(name),
        None => {
            std::fs::write(&fallback, CONFIG).map_err(|error| {
                Diagnostic::new(
                    "build-failed",
                    format!("Couldn't write Rolldown's configuration: {error}"),
                )
                .file(source)
            })?;
            fallback.clone()
        }
    };
    let mut command = Command::new(&rolldown.program);
    command
        .arg("--config")
        .arg(&config)
        .arg("--file")
        .arg(&outfile)
        .env(REPORT_VARIABLE, &report)
        .env("NO_COLOR", "1")
        .current_dir(dir);
    if options.sourcemap {
        command.arg("--sourcemap");
    }
    let outcome = run(&mut command);
    let reported = std::fs::read(&report).ok();
    let _ = std::fs::remove_file(&report);
    let _ = std::fs::remove_file(&fallback);
    if let Some((importer, builtin)) = reported.as_deref().and_then(node_builtin_import) {
        return Err(Diagnostic::new(
            "node-builtin",
            format!("imports the Node built-in `{builtin}`, which Dupin extensions don't have"),
        )
        .file(importer)
        .hint(
            "Use a dependency that runs in a browser, or drop it. Extensions reach the system through Dupin, not Node.",
        ));
    }
    let stderr = outcome.map_err(|error| {
        Diagnostic::new("build-failed", format!("Couldn't run Rolldown: {error}"))
            .file(source)
            .hint("Rolldown's CLI needs Node.js on PATH.")
    })?;
    if let Some(stderr) = stderr {
        return Err(
            Diagnostic::new("build-failed", format!("Rolldown failed: {stderr}"))
                .file(source)
                .hint("Fix the error above and build again."),
        );
    }
    if !outfile.is_file() {
        return Err(Diagnostic::new("build-failed", "Rolldown wrote nothing")
            .file(&package.main)
            .hint("Keep `output.file` in the Rolldown configuration; `dupin ext build` sets it to `dupin.main`."));
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

/// Rolldown's error output without the JavaScript stack traces under each error.
pub(crate) fn summarize(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|line| !line.trim_start().starts_with("at "))
        .collect();
    let joined = lines.join("\n");
    let text = joined.trim();
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

/// The module and the Node built-in it imports, from the report [`CONFIG`] writes.
pub(crate) fn node_builtin_import(report: &[u8]) -> Option<(String, String)> {
    let report: Value = serde_json::from_slice(report).ok()?;
    let found = report.get("nodeBuiltin")?;
    Some((
        found.get("importer")?.as_str()?.to_owned(),
        found.get("specifier")?.as_str()?.to_owned(),
    ))
}
