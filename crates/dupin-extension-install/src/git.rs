//! Fetching a package with the `git` command, without checking it out: Dupin reads the commit's
//! tree and blobs itself, so no hook, filter, attribute, submodule or symlink in the repository
//! takes effect, and limits apply before anything is written.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::Error;
use crate::source::{is_full_commit, valid_rev};
use crate::tree::{Listing, Stage};

/// How Dupin runs `git`.
#[derive(Clone, Debug)]
pub struct Git {
    pub program: PathBuf,
    /// For each `git` invocation, network fetches included.
    pub timeout: Duration,
    /// Added to every invocation's environment, after Dupin's own settings.
    pub env: Vec<(OsString, OsString)>,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            program: PathBuf::from("git"),
            timeout: Duration::from_secs(120),
            env: Vec::new(),
        }
    }
}

/// Configuration that keeps a fetch from running anything the repository or a URL supplies.
const HARDENING: [&str; 7] = [
    "core.hooksPath=/dev/null",
    "core.fsmonitor=false",
    "fetch.recurseSubmodules=false",
    "submodule.recurse=false",
    "transfer.fsckObjects=true",
    "protocol.ext.allow=never",
    "protocol.version=2",
];

/// Variables that would point `git` at another repository than the scratch one.
const REDIRECTS: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

pub(crate) struct Fetched {
    pub commit: String,
    /// The tag or branch the commit came from, when the request named one or none.
    pub rev: Option<String>,
}

impl Git {
    fn command(&self, repo: &Path) -> Command {
        let mut command = Command::new(&self.program);
        for setting in HARDENING {
            command.arg("-c").arg(setting);
        }
        command.arg("--git-dir").arg(repo);
        for name in REDIRECTS {
            command.env_remove(name);
        }
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ALLOW_PROTOCOL", "https:ssh:file")
            .env("GIT_LFS_SKIP_SMUDGE", "1")
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn run(&self, mut command: Command, input: Option<Vec<u8>>) -> Result<Vec<u8>, Error> {
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|error| {
            Error::new(
                "git-unavailable",
                format!("couldn't run `{}`: {error}", self.program.display()),
            )
            .hint("Install Git (for example with `xcode-select --install`), or install from a local folder.")
        })?;
        let writer = input.and_then(|bytes| {
            let mut stdin = child.stdin.take()?;
            Some(std::thread::spawn(move || {
                let _ = stdin.write_all(&bytes);
            }))
        });
        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let out = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        });
        let err = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut bytes);
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            bytes
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::new(
                        "git-failed",
                        format!("git took longer than {} seconds", self.timeout.as_secs()),
                    ));
                }
            }
        };
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        if !status.success() {
            let message = String::from_utf8_lossy(&stderr);
            let message = message
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(Error::new("git-failed", format!("git failed: {message}")));
        }
        Ok(stdout)
    }

    fn text(&self, command: Command) -> Result<String, Error> {
        String::from_utf8(self.run(command, None)?)
            .map_err(|_| Error::new("git-failed", "git printed something that isn't UTF-8"))
    }

    fn init(&self, repo: &Path) -> Result<(), Error> {
        let mut command = self.command(repo);
        command.args(["init", "--bare", "--quiet"]).arg(repo);
        self.run(command, None).map(drop)
    }

    /// The remote's tags, without fetching anything.
    fn tags(&self, repo: &Path, url: &str) -> Result<Vec<String>, Error> {
        let mut command = self.command(repo);
        command.args(["ls-remote", "--tags", "--refs", url]);
        Ok(self
            .text(command)?
            .lines()
            .filter_map(|line| line.split_once('\t')?.1.strip_prefix("refs/tags/"))
            .map(str::to_owned)
            .collect())
    }

    fn fetch(&self, repo: &Path, url: &str, refspecs: &[&str], shallow: bool) -> Result<(), Error> {
        let mut command = self.command(repo);
        command.args(["fetch", "--quiet", "--no-tags", "--no-recurse-submodules"]);
        if shallow {
            command.arg("--depth=1");
        }
        command.arg(url).args(refspecs);
        self.run(command, None).map(drop)
    }

    fn commit_of(&self, repo: &Path, rev: &str) -> Result<String, Error> {
        let mut command = self.command(repo);
        command.args(["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]);
        let commit = self.text(command)?.trim().to_owned();
        if is_full_commit(&commit) {
            Ok(commit)
        } else {
            Err(Error::new("git-failed", format!("`{rev}` is not a commit")))
        }
    }

    /// Fetches `rev` from `url` into the bare `repo`: a tag, branch or full commit SHA, or, with
    /// none, the newest version tag, or the default branch when there is no version tag.
    pub(crate) fn resolve(&self, repo: &Path, url: &str, rev: Option<&str>) -> Result<Fetched, Error> {
        if url.starts_with('-') || rev.is_some_and(|rev| !valid_rev(rev)) {
            return Err(Error::new(
                "invalid-source",
                "is not a Git source Dupin can fetch",
            ));
        }
        self.init(repo)?;
        let rev = match rev {
            Some(rev) => Some(rev.to_owned()),
            None => newest_version(&self.tags(repo, url)?).map(str::to_owned),
        };
        match rev.as_deref() {
            Some(commit) if is_full_commit(commit) => {
                if self.fetch(repo, url, &[commit], true).is_err() {
                    // Not every server lets a client ask for a commit by SHA; a full fetch
                    // brings it if any branch or tag reaches it.
                    self.fetch(
                        repo,
                        url,
                        &["+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"],
                        false,
                    )?;
                }
                let resolved = self
                    .commit_of(repo, commit)
                    .map_err(|_| Error::new("unknown-ref", format!("commit {commit} isn't in {url}")))?;
                Ok(Fetched {
                    commit: resolved,
                    rev: None,
                })
            }
            Some(name) => {
                self.fetch(repo, url, &[name], true).map_err(|error| {
                    let mut error = Error::new(
                        "unknown-ref",
                        format!("couldn't fetch `{name}` from {url}: {}", error.message),
                    );
                    if name.len() < 40 && name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        error = error.hint("Name a commit by its full 40-character SHA.");
                    }
                    error
                })?;
                Ok(Fetched {
                    commit: self.commit_of(repo, "FETCH_HEAD")?,
                    rev: Some(name.to_owned()),
                })
            }
            None => {
                self.fetch(repo, url, &["HEAD"], true)?;
                Ok(Fetched {
                    commit: self.commit_of(repo, "FETCH_HEAD")?,
                    rev: None,
                })
            }
        }
    }

    /// Writes the files of `commit` into `stage`, refusing symlinks, submodules and anything
    /// past the package limits before reading a single blob.
    pub(crate) fn export(&self, repo: &Path, commit: &str, stage: &Stage) -> Result<(), Error> {
        let mut command = self.command(repo);
        command.args(["ls-tree", "-r", "-z", "-l", "--full-tree", commit]);
        let listing = self.run(command, None)?;
        let mut listing_check = Listing::new();
        let mut files = Vec::new();
        for record in listing
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            let tab = record
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| Error::new("git-failed", "git listed a tree in an unexpected form"))?;
            let path = std::str::from_utf8(&record[tab + 1..])
                .map_err(|_| Error::new("invalid-package-path", "a file name isn't UTF-8"))?;
            let head = std::str::from_utf8(&record[..tab]).unwrap_or_default();
            let fields: Vec<_> = head.split_whitespace().collect();
            let [mode, kind, object, size] = fields[..] else {
                return Err(Error::new(
                    "git-failed",
                    "git listed a tree in an unexpected form",
                ));
            };
            let executable = match (mode, kind) {
                ("100644", "blob") => false,
                ("100755", "blob") => true,
                ("120000", _) => {
                    return Err(Error::new("invalid-package-path", "is a symbolic link")
                        .file(path)
                        .hint("Replace the link with the file it points to."));
                }
                ("160000", _) => {
                    return Err(Error::new("invalid-package-path", "is a Git submodule")
                        .file(path)
                        .hint("Dupin doesn't fetch submodules; bundle what the package needs into it."));
                }
                _ => {
                    return Err(Error::new("invalid-package-path", format!("has Git mode {mode}")).file(path));
                }
            };
            let size: u64 = size
                .parse()
                .map_err(|_| Error::new("git-failed", "git listed a tree in an unexpected form"))?;
            listing_check.add(path, size)?;
            files.push((path.to_owned(), object.to_owned(), size, executable));
        }
        if files.is_empty() {
            return Ok(());
        }
        let mut command = self.command(repo);
        command.args(["cat-file", "--batch"]);
        let request: String = files
            .iter()
            .map(|(_, object, ..)| format!("{object}\n"))
            .collect();
        let output = self.run(command, Some(request.into_bytes()))?;
        let mut rest = &output[..];
        for (path, object, size, executable) in &files {
            let newline = rest
                .iter()
                .position(|byte| *byte == b'\n')
                .ok_or_else(|| Error::new("git-failed", "git returned fewer files than it listed"))?;
            let header = std::str::from_utf8(&rest[..newline]).unwrap_or_default();
            let expected = format!("{object} blob {size}");
            if header != expected {
                return Err(Error::new(
                    "git-failed",
                    format!("git returned `{header}` for {path}"),
                ));
            }
            let start = newline + 1;
            let end = start + *size as usize;
            if rest.len() < end + 1 {
                return Err(Error::new(
                    "git-failed",
                    "git returned fewer bytes than it listed",
                ));
            }
            stage.write(path, &rest[start..end], *executable)?;
            rest = &rest[end + 1..];
        }
        Ok(())
    }
}

/// The highest `1.2.3` or `v1.2.3` tag; a pre-release (`v2.0.0-beta.1`) ranks below its release.
fn newest_version(tags: &[String]) -> Option<&str> {
    tags.iter()
        .filter_map(|tag| version_key(tag).map(|key| (key, tag)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, tag)| tag.as_str())
}

fn version_key(tag: &str) -> Option<(Vec<u64>, bool, String)> {
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let mut parts = core
        .split('.')
        .map(|part| {
            (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        })
        .collect::<Option<Vec<u64>>>()?;
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    parts.resize(4, 0);
    Some((parts, pre.is_none(), pre.unwrap_or_default().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_version_tag_wins_over_names_and_pre_releases() {
        let tags: Vec<String> = [
            "v1.2.0",
            "v1.10.0",
            "latest",
            "v2.0.0-beta.1",
            "1.9",
            "v1.10.0-rc.1",
        ]
        .map(str::to_owned)
        .into();
        assert_eq!(newest_version(&tags), Some("v2.0.0-beta.1"));
        let tags: Vec<String> = ["v1.2.0", "v1.10.0", "nightly"].map(str::to_owned).into();
        assert_eq!(newest_version(&tags), Some("v1.10.0"));
        let tags: Vec<String> = ["v1.10.0-rc.1", "v1.10.0"].map(str::to_owned).into();
        assert_eq!(newest_version(&tags), Some("v1.10.0"));
        assert_eq!(newest_version(&["nightly".to_owned()]), None);
    }
}
