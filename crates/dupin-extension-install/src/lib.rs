//! Installs Dupin extension packages from GitHub, Git or a local folder into the extensions directory,
//! pins each one in `extensions.lock`, and detects a package whose files changed after install.
//!
//! The launcher calls [`Integrity::check`] while it scans the extensions directory; `dupin ext`
//! drives an [`Installer`]. Grants aren't kept here: the capability broker owns them. Nothing here runs
//! package code, a package manager, or anything a repository supplies, and a `github:` source
//! needs no Git.

mod discover;
mod error;
mod git;
mod github;
mod hash;
mod integrity;
mod lock;
mod manifest;
mod place;
mod source;
mod tarball;
mod tree;
mod version;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub use discover::{Found, MAX_LISTED, Manifest, Repository, TOPIC};
pub use error::Error;
pub use git::Git;
pub use github::GitHub;
pub use hash::content_hash;
pub use integrity::Integrity;
pub use lock::{Entry, LOCK_FILE, Lock};
use manifest::Package;
use source::Source;

use lock::Guard;
use tree::{Listing, Stage};

/// Answers who already uses a keyword outside the extensions directory, such as `in` or a web
/// search keyword, or `None` when it is free.
pub type Taken<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Each package an update or sync touched, by name, with what happened to it.
pub type Outcomes = Vec<(String, Result<Change, Error>)>;

pub struct Installer {
    /// `$XDG_CONFIG_HOME/dupin/extensions`.
    pub extensions: PathBuf,
    /// `extensions.lock` in Dupin's data directory.
    pub lock: PathBuf,
    /// For Git URLs other than `github:`.
    pub git: Git,
    pub github: GitHub,
    /// The names of the packages bundled with the launcher, which no installed package may
    /// take: one would replace the bundled package under its name.
    pub bundled: BTreeSet<String>,
}

/// What an install, update or sync did to one package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub name: String,
    pub title: String,
    pub entry: Entry,
    /// The lock entry it replaced.
    pub previous: Option<Entry>,
    /// Scopes the new revision requests that the previous one didn't; the broker holds them
    /// back until the user agrees.
    pub added: Vec<String>,
    /// Scopes the previous revision requested and this one doesn't; their grants are dropped.
    pub removed: Vec<String>,
}

impl Change {
    /// Whether the installed files stayed as they were.
    pub fn unchanged(&self) -> bool {
        self.previous.as_ref().is_some_and(|previous| {
            previous.content_hash == self.entry.content_hash && previous.commit == self.entry.commit
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Installed,
    Missing,
    /// The folder no longer matches the lock, and the launcher keeps it off.
    Modified(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub entry: Entry,
    pub state: State,
}

struct Target<'a> {
    source: Source,
    /// The installed package this replaces, for update and sync.
    name: Option<&'a str>,
    /// The lock entry the result must reproduce, for sync.
    locked: Option<&'a Entry>,
}

impl Installer {
    /// Installs `spec` (`github:owner/repo[@ref]`, a Git URL with an optional `@ref`, or a folder)
    /// under its package name. Installing a package that is already installed replaces it, as an
    /// update to the named revision would.
    pub fn install(&self, spec: &str, cwd: &Path, taken: Taken<'_>) -> Result<Change, Error> {
        let _guard = Guard::acquire(&self.lock)?;
        let source = Source::parse(spec, cwd)?;
        self.place(
            Target {
                source,
                name: None,
                locked: None,
            },
            taken,
        )
    }

    /// Moves `name`, or every installed package, to the newest version tag of its repository,
    /// or its default branch when it has no version tag; a folder source is copied again.
    pub fn update(&self, name: Option<&str>, taken: Taken<'_>) -> Result<Outcomes, Error> {
        let _guard = Guard::acquire(&self.lock)?;
        let lock = Lock::read(&self.lock)?;
        let names: Vec<String> = match name {
            Some(name) if lock.packages.contains_key(name) => vec![name.to_owned()],
            Some(name) => return Err(not_installed(name)),
            None => lock.packages.keys().cloned().collect(),
        };
        Ok(names
            .into_iter()
            .map(|name| {
                let entry = &lock.packages[&name];
                let result = Source::parse(&entry.source, Path::new("/")).and_then(|source| {
                    self.place(
                        Target {
                            source: source.with_rev(None),
                            name: Some(&name),
                            locked: None,
                        },
                        taken,
                    )
                });
                (name, result)
            })
            .collect())
    }

    /// Reinstalls every package whose folder is missing or no longer matches the lock, from the
    /// locked commit, and checks the result has the locked content hash.
    pub fn sync(&self, taken: Taken<'_>) -> Result<Outcomes, Error> {
        let listed = self.list()?;
        let _guard = Guard::acquire(&self.lock)?;
        Ok(listed
            .into_iter()
            .filter(|listed| listed.state != State::Installed)
            .map(|Listed { name, entry, .. }| {
                let result = Source::parse(&entry.source, Path::new("/")).and_then(|source| {
                    let source = match &entry.commit {
                        Some(commit) => source.with_rev(Some(commit.clone())),
                        None => source,
                    };
                    self.place(
                        Target {
                            source,
                            name: Some(&name),
                            locked: Some(&entry),
                        },
                        taken,
                    )
                });
                (name, result)
            })
            .collect())
    }

    /// Deletes an installed package and its lock entry, and with it the user's grants.
    pub fn remove(&self, name: &str) -> Result<Entry, Error> {
        let _guard = Guard::acquire(&self.lock)?;
        let mut lock = Lock::read(&self.lock)?;
        let entry = lock.packages.remove(name).ok_or_else(|| {
            let error = not_installed(name);
            if self.extensions.join(name).symlink_metadata().is_ok() {
                error.hint(format!(
                    "{} wasn't installed with `dupin ext install`; remove it where it came from, such as your Home Manager configuration.",
                    self.extensions.join(name).display()
                ))
            } else {
                error
            }
        })?;
        // The folder goes first: a lock that drops the entry while the folder stays would leave
        // it running as an unchecked, hand-made extension.
        let target = self.extensions.join(name);
        let stage = Stage::new(&self.extensions, "remove")?;
        let moved = target.symlink_metadata().is_ok();
        if moved {
            std::fs::rename(&target, stage.package())
                .map_err(|error| Error::io("io-error", format!("move {} aside", target.display()), error))?;
        }
        if let Err(error) = lock.write(&self.lock) {
            if moved {
                let _ = std::fs::rename(stage.package(), &target);
            }
            return Err(error);
        }
        Ok(entry)
    }

    /// The locked packages and whether each folder still matches.
    pub fn list(&self) -> Result<Vec<Listed>, Error> {
        let lock = Lock::read(&self.lock)?;
        Ok(lock
            .packages
            .into_iter()
            .map(|(name, entry)| {
                let folder = self.extensions.join(&name);
                let state = if folder.symlink_metadata().is_err() {
                    State::Missing
                } else {
                    match content_hash(&folder) {
                        Ok(hash) if hash == entry.content_hash => State::Installed,
                        Ok(_) => State::Modified("its files differ from what was installed".into()),
                        Err(error) => State::Modified(error.to_string()),
                    }
                };
                Listed { name, entry, state }
            })
            .collect())
    }

    fn place(&self, target: Target<'_>, taken: Taken<'_>) -> Result<Change, Error> {
        let stage = Stage::new(&self.extensions, "install")?;
        let on = target.locked.and_then(|locked| locked.rev.as_deref());
        // Downloads go outside the extensions folder, so the launcher's watcher doesn't rescan
        // on every object Git writes.
        let fetched = match &target.source {
            Source::Path(folder) => {
                copy_folder(folder, &stage)?;
                None
            }
            Source::GitHub { owner, repo, rev } => {
                let scratch = Stage::new(&std::env::temp_dir(), "fetch")?;
                let repo = github::Repo {
                    owner,
                    name: repo,
                    scratch: &scratch.root,
                };
                let fetched = self.github.resolve(&repo, rev.as_deref(), on)?;
                self.github.export(&repo, &fetched.commit, &stage)?;
                Some(fetched)
            }
            Source::Git { url, rev } => {
                let scratch = Stage::new(&std::env::temp_dir(), "fetch")?;
                let repo = scratch.root.join("repo.git");
                let fetched = self.git.resolve(&repo, url, rev.as_deref(), on)?;
                self.git.export(&repo, &fetched.commit, &stage)?;
                Some(fetched)
            }
        };
        let staged = stage.package();
        std::fs::create_dir_all(&staged)
            .map_err(|error| Error::io("io-error", format!("create {}", staged.display()), error))?;
        let package = validate(&staged)?;
        if let Some(name) = target.name
            && name != package.name
        {
            return Err(Error::new(
                "renamed-package",
                format!("is now named `{}`, not `{name}`", package.name),
            )
            .file(manifest::PACKAGE_JSON)
            .field("name")
            .hint(format!(
                "Run `dupin ext remove {name}`, then install the package again."
            )));
        }
        let hash = content_hash(&staged).map_err(|error| Error::io("io-error", "hash the package", error))?;
        if let Some(expected) = target.locked.map(|locked| &locked.content_hash)
            && *expected != hash
        {
            return Err(Error::new(
                "hash-mismatch",
                format!("fetched {hash}, but extensions.lock pins {expected}"),
            )
            .hint("The source no longer has the locked contents; run `dupin ext update` to move to what it has now."));
        }

        let mut lock = Lock::read(&self.lock)?;
        let destination = self.extensions.join(&package.name);
        let previous = lock.packages.get(&package.name).cloned();
        if target.name.is_none()
            && let Some(previous) = &previous
            && previous.source != target.source.locator()
        {
            return Err(Error::new(
                "source-changed",
                format!(
                    "`{}` is already installed from {}, not {}",
                    package.name,
                    previous.source,
                    target.source.locator()
                ),
            )
            .file(manifest::PACKAGE_JSON)
            .field("name")
            .hint(format!(
                "Its grants belong to that source. To replace it, run `dupin ext remove {}`, then install again.",
                package.name
            )));
        }
        if previous.is_none() && destination.symlink_metadata().is_ok() {
            return Err(Error::new(
                "folder-exists",
                format!(
                    "{} already exists and wasn't installed with `dupin ext install`",
                    destination.display()
                ),
            )
            .hint("Move or delete that folder first, or give the package another name."));
        }
        if self.bundled.contains(&package.name) {
            return Err(Error::new(
                "name-bundled",
                format!(
                    "`{}` is the name of an extension bundled with Dupin",
                    package.name
                ),
            )
            .file(manifest::PACKAGE_JSON)
            .field("name")
            .hint(format!(
                "Give the package another name, or run `dupin ext eject {}` to edit the bundled one.",
                package.name
            )));
        }
        self.check_keyword(&package, taken)?;

        let (mut entry, added, removed) =
            next_entry(&target.source, fetched, &package, hash, previous.as_ref());
        if let Some(locked) = target.locked {
            // Sync fetches the locked commit by SHA, which names no tag or branch of its own.
            entry.rev.clone_from(&locked.rev);
        }
        let change = Change {
            name: package.name.clone(),
            title: package.title.clone(),
            entry,
            previous: previous.clone(),
            added,
            removed,
        };
        if let Some(previous) = &previous
            && change.unchanged()
            && previous.source == change.entry.source
            && content_hash(&destination).is_ok_and(|installed| installed == previous.content_hash)
        {
            return Ok(Change {
                entry: previous.clone(),
                ..change
            });
        }

        let before = lock.clone();
        lock.packages.insert(package.name.clone(), change.entry.clone());
        // The lock goes first: the watcher rescans when the folder moves, and by then the lock
        // already pins the new contents.
        lock.write(&self.lock)?;
        if let Err(error) = place::swap_into(&staged, &destination) {
            let _ = before.write(&self.lock);
            return Err(Error::io(
                "io-error",
                format!("move the package to {}", destination.display()),
                error,
            ));
        }
        Ok(change)
    }

    fn check_keyword(&self, package: &Package, taken: Taken<'_>) -> Result<(), Error> {
        let clash = |owner: String| {
            Error::new(
                "keyword-taken",
                format!("`{}` is already used by {owner}", package.keyword),
            )
            .file(manifest::PACKAGE_JSON)
            .field("dupin.keyword")
            .hint("Choose another `dupin.keyword`, or free this one in config.toml or by removing the extension that uses it.")
        };
        if let Some(owner) = taken(&package.keyword) {
            return Err(clash(owner));
        }
        let Ok(entries) = std::fs::read_dir(&self.extensions) else {
            return Ok(());
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == package.name || !entry.path().is_dir() {
                continue;
            }
            if manifest::keyword_of(&entry.path()).as_deref() == Some(package.keyword.as_str()) {
                return Err(clash(format!("the extension in `{name}`")));
            }
        }
        Ok(())
    }
}

fn not_installed(name: &str) -> Error {
    Error::new("not-installed", format!("`{name}` isn't in extensions.lock"))
        .hint("`dupin ext list` shows what `dupin ext install` installed.")
}

/// The lock entry for a newly placed package, and the scopes it adds and drops. The broker
/// reconciles grants itself when the launcher loads the new revision.
fn next_entry(
    source: &Source,
    fetched: Option<git::Fetched>,
    package: &Package,
    content_hash: String,
    previous: Option<&Entry>,
) -> (Entry, Vec<String>, Vec<String>) {
    let requested: BTreeSet<_> = package.capabilities.iter().cloned().collect();
    let (added, removed) = match previous {
        None => (Vec::new(), Vec::new()),
        Some(previous) => {
            let before: BTreeSet<_> = previous.requested.iter().cloned().collect();
            (
                requested.difference(&before).cloned().collect(),
                before.difference(&requested).cloned().collect(),
            )
        }
    };
    let (commit, rev) = match fetched {
        Some(fetched) => (Some(fetched.commit), fetched.rev),
        None => (None, None),
    };
    let entry = Entry {
        source: source.locator(),
        rev,
        commit,
        content_hash,
        installed_at: lock::now(),
        version: package.version.clone(),
        keyword: package.keyword.clone(),
        requested: requested.into_iter().collect(),
    };
    (entry, added, removed)
}

/// Checks a staged package before it is placed: its manifest, and that its entry is a module
/// the extension host runs without a build: JavaScript, or TypeScript it erases the types of.
fn validate(folder: &Path) -> Result<Package, Error> {
    let package = manifest::read(folder)?;
    let main = Path::new(&package.main);
    let extension = main
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    let declaration = [".d.ts", ".d.mts", ".d.tsx"]
        .iter()
        .any(|suffix| package.main.ends_with(suffix));
    let module = matches!(extension, "js" | "mjs" | "ts" | "mts" | "tsx") && !declaration;
    if tree::check_path(&package.main).is_err() || !module {
        let error = Error::new(
            "invalid-manifest",
            "must be a relative path to a .js, .mjs, .ts, .mts or .tsx module inside the package",
        )
        .file(manifest::PACKAGE_JSON)
        .field("dupin.main");
        return Err(match extension {
            "jsx" => error.hint("Dupin runs JavaScript and erasable TypeScript only: build the entry with `dupin ext build`, commit the output and point `dupin.main` at it."),
            _ if declaration => error.hint("A declaration file only describes types: point `dupin.main` at the module itself."),
            _ => error,
        });
    }
    let entry = folder.join(main);
    let is_file = entry.symlink_metadata().is_ok_and(|metadata| metadata.is_file());
    if !is_file {
        return Err(Error::new("missing-entry", "is not in the package")
            .file(package.main.clone())
            .hint("Commit the entry, built if the package has a build: Dupin never runs a build or a package manager when it installs."));
    }
    let source = std::fs::read(&entry)
        .map_err(|error| Error::io("io-error", format!("read {}", package.main), error))?;
    if std::str::from_utf8(&source).is_err() {
        return Err(Error::new("invalid-entry", "is not UTF-8").file(package.main.clone()));
    }
    Ok(package)
}

/// Copies a folder's regular files into `stage`, skipping `.git` and the top-level
/// `node_modules`, which a built package doesn't need.
fn copy_folder(folder: &Path, stage: &Stage) -> Result<(), Error> {
    let metadata = folder.metadata().map_err(|error| {
        Error::new(
            "invalid-source",
            format!("{} can't be read: {error}", folder.display()),
        )
    })?;
    if !metadata.is_dir() {
        return Err(Error::new(
            "invalid-source",
            format!("{} is not a folder", folder.display()),
        ));
    }
    let mut files = Vec::new();
    collect(folder, "", &mut files)?;
    let mut listing = Listing::new();
    for (path, size, _) in &files {
        listing.add(path, *size)?;
    }
    for (path, _, executable) in &files {
        let bytes = std::fs::read(folder.join(path))
            .map_err(|error| Error::io("io-error", format!("read {path}"), error))?;
        stage.write(path, &bytes, *executable)?;
    }
    Ok(())
}

fn collect(dir: &Path, prefix: &str, files: &mut Vec<(String, u64, bool)>) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    let entries = std::fs::read_dir(dir)
        .map_err(|error| Error::io("io-error", format!("read {}", dir.display()), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| Error::io("io-error", format!("read {}", dir.display()), error))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::new("invalid-package-path", "a file name isn't UTF-8"))?;
        if name == ".git" || (prefix.is_empty() && name == "node_modules") {
            continue;
        }
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let metadata = entry
            .path()
            .symlink_metadata()
            .map_err(|error| Error::io("io-error", format!("read {path}"), error))?;
        if metadata.is_dir() {
            tree::check_path(&path)?;
            collect(&entry.path(), &path, files)?;
        } else if metadata.is_file() {
            files.push((path, metadata.len(), metadata.permissions().mode() & 0o111 != 0));
        } else {
            return Err(
                Error::new("invalid-package-path", "is a symbolic link or a special file")
                    .file(path)
                    .hint("Replace the link with the file it points to."),
            );
        }
        if files.len() > tree::MAX_FILES {
            return Err(Error::new(
                "package-too-large",
                format!("has more than {} files", tree::MAX_FILES),
            ));
        }
    }
    Ok(())
}
