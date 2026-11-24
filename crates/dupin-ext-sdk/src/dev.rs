//! `dupin ext dev`: rebuild on every change and publish each good revision as a development
//! copy in the launcher's extensions folder.
//!
//! `extensions/<name>` is a symlink to `extensions/.dev/<name>/r<N>`. A revision is built and
//! checked in full before one `rename` swaps the link to it, so the launcher's scanner (which
//! skips dot-folders and follows linked folders) only ever sees a complete, checked revision,
//! and a failed build leaves the link where it was.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use dupin_js_runtime::Diagnostic;

use crate::build::{BuildOptions, Bundler, bundle, missing_bundler};
use crate::manifest::{MANIFEST_FILE, read_package};

const REVISIONS: &str = ".dev";
/// The live revision and the one before it, which the launcher may still be reading.
const KEPT_REVISIONS: usize = 2;
const MAX_WATCHED_FILES: usize = 10_000;
const MAX_WATCHED_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub name: String,
    pub revision: u64,
    /// `extensions/<name>`, the link the launcher loads.
    pub link: PathBuf,
}

/// A package and the extensions folder its development copy goes to.
pub struct DevCopy<'a> {
    pub package_dir: &'a Path,
    pub extensions: &'a Path,
    pub bundler: Option<&'a Bundler>,
    pub options: BuildOptions,
    /// Checks a staged revision before it goes live.
    pub validate: &'a dyn Fn(&Path) -> Result<(), Diagnostic>,
}

impl DevCopy<'_> {
    /// Builds the package into a new revision, validates it and, only if that passes, points
    /// `extensions/<name>` at it.
    pub(crate) fn publish(&self) -> Result<Published, Diagnostic> {
        publish(
            self.package_dir,
            self.extensions,
            self.bundler,
            self.options,
            self.validate,
        )
    }

    /// Publishes once, then again after each change that has settled for one `interval`, until
    /// `keep_going` returns false. Every outcome goes to `report`.
    pub fn watch(
        &self,
        interval: Duration,
        keep_going: &dyn Fn() -> bool,
        report: &mut dyn FnMut(Result<Published, Diagnostic>),
    ) {
        let mut seen = Fingerprint::of(self.package_dir);
        report(self.publish());
        while keep_going() {
            std::thread::sleep(interval);
            let now = Fingerprint::of(self.package_dir);
            if now == seen {
                continue;
            }
            seen = now;
            loop {
                std::thread::sleep(interval);
                let settled = Fingerprint::of(self.package_dir);
                if settled == seen {
                    break;
                }
                seen = settled;
            }
            report(self.publish());
        }
    }
}

fn publish(
    package_dir: &Path,
    extensions: &Path,
    bundler: Option<&Bundler>,
    options: BuildOptions,
    validate: &dyn Fn(&Path) -> Result<(), Diagnostic>,
) -> Result<Published, Diagnostic> {
    let package = read_package(package_dir)?;
    let link = extensions.join(&package.name);
    let revisions = extensions.join(REVISIONS).join(&package.name);
    ensure_development_link(&link, &package.name)?;
    std::fs::create_dir_all(&revisions).map_err(|error| unwritable(&revisions, &error))?;
    remove_partials(&revisions);

    let revision = existing_revisions(&revisions).last().copied().unwrap_or(0) + 1;
    let staging = revisions.join(format!("r{revision}.partial"));
    let filled = fill(package_dir, &staging, &package, bundler, options).and_then(|()| validate(&staging));
    if let Err(error) = filled {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    let target = revisions.join(format!("r{revision}"));
    std::fs::rename(&staging, &target).map_err(|error| unwritable(&target, &error))?;

    let relative = Path::new(REVISIONS)
        .join(&package.name)
        .join(format!("r{revision}"));
    let pending = extensions.join(format!(".{}.dev-link", package.name));
    let _ = std::fs::remove_file(&pending);
    std::os::unix::fs::symlink(&relative, &pending).map_err(|error| unwritable(&pending, &error))?;
    std::fs::rename(&pending, &link).map_err(|error| {
        let _ = std::fs::remove_file(&pending);
        unwritable(&link, &error)
    })?;

    let existing = existing_revisions(&revisions);
    for old in &existing[..existing.len().saturating_sub(KEPT_REVISIONS)] {
        let _ = std::fs::remove_dir_all(revisions.join(format!("r{old}")));
    }
    Ok(Published {
        name: package.name,
        revision,
        link,
    })
}

/// `link` may be missing or a link into this package's revisions; anything else is an
/// installed extension that a development copy must not replace.
fn ensure_development_link(link: &Path, name: &str) -> Result<(), Diagnostic> {
    match std::fs::read_link(link) {
        Ok(target) if target.starts_with(Path::new(REVISIONS).join(name)) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(Diagnostic::new(
            "dev-target-occupied",
            "is already an installed extension, not a development copy",
        )
        .file(link.display().to_string())
        .hint(format!(
            "Move or remove the installed `{name}`, or rename the package in package.json."
        ))),
    }
}

/// Copies what the launcher loads into `staging`: `package.json` and either the fresh bundle or
/// the package's JavaScript files.
fn fill(
    package_dir: &Path,
    staging: &Path,
    package: &crate::PackageInfo,
    bundler: Option<&Bundler>,
    options: BuildOptions,
) -> Result<(), Diagnostic> {
    std::fs::create_dir_all(staging).map_err(|error| unwritable(staging, &error))?;
    copy(&package_dir.join(MANIFEST_FILE), &staging.join(MANIFEST_FILE))?;
    if package.needs_build() {
        let bundler = bundler.ok_or_else(missing_bundler)?;
        return bundle(
            package_dir,
            package,
            bundler,
            &staging.join(&package.main),
            options,
        );
    }
    for (name, path) in dupin_js_runtime::source_files(package_dir)? {
        copy(&path, &staging.join(name))?;
    }
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<(), Diagnostic> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|error| unwritable(parent, &error))?;
    }
    std::fs::copy(from, to)
        .map(drop)
        .map_err(|error| unwritable(to, &error))
}

fn existing_revisions(revisions: &Path) -> Vec<u64> {
    let mut found: Vec<u64> = std::fs::read_dir(revisions)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.strip_prefix('r')?.parse().ok())
        .collect();
    found.sort_unstable();
    found
}

/// Leftovers of a publish that was interrupted between building and swapping the link.
fn remove_partials(revisions: &Path) {
    for entry in std::fs::read_dir(revisions).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().ends_with(".partial") {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn unwritable(path: &Path, error: &std::io::Error) -> Diagnostic {
    Diagnostic::new("unwritable", format!("Couldn't write it: {error}")).file(path.display().to_string())
}

/// The size and modification time of every file an author edits, to notice a change by
/// polling. Polling a package-sized tree is cheap and needs no platform watcher.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Fingerprint(Vec<(PathBuf, u64, Option<SystemTime>)>);

impl Fingerprint {
    /// Skips dot-entries, `node_modules` and, for a bundled package, the bundle itself, which
    /// the author's own tools may rewrite.
    pub(crate) fn of(package_dir: &Path) -> Self {
        let skipped: Vec<PathBuf> = read_package(package_dir)
            .ok()
            .filter(|package| package.needs_build())
            .map(|package| {
                let map = format!("{}.map", package.main);
                vec![PathBuf::from(package.main), PathBuf::from(map)]
            })
            .unwrap_or_default();
        let mut entries = Vec::new();
        let mut pending = vec![(PathBuf::new(), 0)];
        while let Some((relative, depth)) = pending.pop() {
            let Ok(read) = std::fs::read_dir(package_dir.join(&relative)) else {
                continue;
            };
            for entry in read.flatten() {
                let name = entry.file_name();
                let text = name.to_string_lossy();
                if text.starts_with('.') || text == "node_modules" {
                    continue;
                }
                let path = relative.join(&name);
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata.is_dir() {
                    if depth + 1 < MAX_WATCHED_DEPTH {
                        pending.push((path, depth + 1));
                    }
                } else if !skipped.contains(&path) && entries.len() < MAX_WATCHED_FILES {
                    entries.push((path, metadata.len(), metadata.modified().ok()));
                }
            }
        }
        entries.sort();
        Self(entries)
    }
}
