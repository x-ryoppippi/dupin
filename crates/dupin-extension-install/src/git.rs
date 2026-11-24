//! Fetching a package with the `git` command, without checking it out: Dupin reads the commit's
//! tree and blobs itself, so no hook, filter, attribute, submodule or symlink in the repository
//! takes effect, and limits apply before anything is written.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::Error;
use crate::source::{is_full_commit, valid_rev};
use crate::tree::{Listing, MAX_FILES, MAX_PATH_BYTES, MAX_TOTAL_BYTES, Stage};
use crate::version::newest_version;

/// How Dupin runs `git`.
#[derive(Clone, Debug)]
pub struct Git {
    pub program: PathBuf,
    /// For each `git` invocation, network fetches included.
    pub timeout: Duration,
    /// How much a fetch may write into the scratch repository before it is stopped.
    pub max_fetch_bytes: u64,
    /// Added to every invocation's environment, after Dupin's own settings.
    pub env: Vec<(OsString, OsString)>,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            program: PathBuf::from("git"),
            timeout: Duration::from_secs(120),
            max_fetch_bytes: 4 * MAX_TOTAL_BYTES,
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

/// What one `git` invocation may print, and how large the scratch repository may grow while it
/// runs, so a hostile repository can't fill memory or disk before the package limits apply.
struct Limits<'a> {
    stdout: u64,
    repo: Option<(&'a Path, u64)>,
}

impl Limits<'_> {
    const SMALL: Self = Self {
        stdout: 4 * 1024 * 1024,
        repo: None,
    };
}

/// One `ls-tree -l -z` record: mode, type, a SHA-256 object id and a padded size around a path.
const LS_TREE_RECORD_BYTES: u64 = MAX_PATH_BYTES as u64 + 128;

#[derive(Debug)]
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
            .stderr(Stdio::piped())
            // Its own process group, so stopping it also stops the helpers it starts for the
            // network, such as git-remote-https or ssh.
            .process_group(0);
        command
    }

    /// Fails before anything runs when `git` is missing, or is the macOS stand-in that would
    /// open the Command Line Tools installer instead of fetching.
    fn check_installed(&self, url: &str) -> Result<(), Error> {
        let missing = |why: String| {
            Error::new("git-unavailable", format!("{url} is fetched with Git, but {why}")).hint(GIT_HINT)
        };
        let Some(program) = find_program(&self.program, std::env::var_os("PATH")) else {
            return Err(missing(format!("`{}` isn't installed", self.program.display())));
        };
        if cfg!(target_os = "macos") && needs_developer_tools(&program, developer_dir) {
            return Err(missing(
                "this Mac has no Git: /usr/bin/git only offers to install the Command Line Tools".to_owned(),
            ));
        }
        Ok(())
    }

    fn run(
        &self,
        mut command: Command,
        input: Option<Vec<u8>>,
        limits: Limits<'_>,
    ) -> Result<Vec<u8>, Error> {
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|error| {
            Error::new(
                "git-unavailable",
                format!("couldn't run `{}`: {error}", self.program.display()),
            )
            .hint(GIT_HINT)
        })?;
        let writer = input.and_then(|bytes| {
            let mut stdin = child.stdin.take()?;
            Some(std::thread::spawn(move || {
                let _ = stdin.write_all(&bytes);
            }))
        });
        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let overflowed = Arc::new(AtomicBool::new(false));
        let out = std::thread::spawn({
            let overflowed = Arc::clone(&overflowed);
            move || {
                let mut bytes = Vec::new();
                let _ = (&mut stdout).take(limits.stdout + 1).read_to_end(&mut bytes);
                if bytes.len() as u64 > limits.stdout {
                    overflowed.store(true, Ordering::Relaxed);
                }
                bytes
            }
        });
        let err = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut bytes);
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            bytes
        });
        let too_much_output = || {
            Error::new(
                "package-too-large",
                format!("git printed more than {} KiB", limits.stdout / 1024),
            )
            .hint("Ship only the built entry and the assets it needs.")
        };
        let started = Instant::now();
        let mut next_size_check = started;
        let stopped = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {}
                Err(error) => break Err(Error::io("git-failed", "wait for git", error)),
            }
            if overflowed.load(Ordering::Relaxed) {
                break Err(too_much_output());
            }
            if let Some((repo, cap)) = limits.repo
                && Instant::now() >= next_size_check
            {
                next_size_check = Instant::now() + Duration::from_millis(200);
                if size_of(repo) > cap {
                    break Err(Error::new(
                        "package-too-large",
                        format!("fetching it took more than {} MiB", cap / 1024 / 1024),
                    )
                    .hint("Keep the package in a small repository of its own, or install from a folder."));
                }
            }
            if started.elapsed() >= self.timeout {
                break Err(Error::new(
                    "git-failed",
                    format!("git took longer than {} seconds", self.timeout.as_secs()),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if stopped.is_err() {
            if let Ok(group) = libc::pid_t::try_from(child.id()) {
                // SAFETY: signals only the process group spawned above, led by `child`, which
                // hasn't been reaped yet, so its id can't have been reused.
                unsafe { libc::killpg(group, libc::SIGKILL) };
            }
            let _ = child.wait();
        }
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        // Git may exit on the closed pipe before the loop sees the overflow.
        if stdout.len() as u64 > limits.stdout {
            return Err(too_much_output());
        }
        let status = stopped?;
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
        String::from_utf8(self.run(command, None, Limits::SMALL)?)
            .map_err(|_| Error::new("git-failed", "git printed something that isn't UTF-8"))
    }

    fn init(&self, repo: &Path) -> Result<(), Error> {
        let mut command = self.command(repo);
        command.args(["init", "--bare", "--quiet"]).arg(repo);
        self.run(command, None, Limits::SMALL).map(drop)
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
        let limits = Limits {
            repo: Some((repo, self.max_fetch_bytes)),
            ..Limits::SMALL
        };
        self.run(command, None, limits).map(drop)
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
        self.check_installed(url)?;
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
        let listing = self.run(
            command,
            None,
            Limits {
                stdout: (MAX_FILES as u64 + 1) * LS_TREE_RECORD_BYTES,
                repo: None,
            },
        )?;
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
        let listed: u64 = files.iter().map(|(_, _, size, _)| size).sum();
        let output = self.run(
            command,
            Some(request.into_bytes()),
            Limits {
                stdout: listed + files.len() as u64 * 128,
                repo: None,
            },
        )?;
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

const GIT_HINT: &str = "Install from `github:owner/repo` or a folder, which need no Git, or install Git (for example with `xcode-select --install`).";

/// Where `PATH` finds `program`, as the shell would; a path with a `/` is taken as it is.
fn find_program(program: &Path, path: Option<OsString>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |candidate: &Path| {
        candidate
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    if program.components().count() > 1 {
        return executable(program).then(|| program.to_owned());
    }
    std::env::split_paths(&path?)
        .map(|dir| dir.join(program))
        .find(|candidate| executable(candidate))
}

/// Whether `program` is the `/usr/bin/git` stub macOS ships, with no developer directory behind
/// it to run the real Git from.
fn needs_developer_tools(program: &Path, developer_dir: impl FnOnce() -> Option<PathBuf>) -> bool {
    let program = program.canonicalize().unwrap_or_else(|_| program.to_owned());
    program == Path::new("/usr/bin/git")
        && !developer_dir().is_some_and(|dir| dir.join("usr/bin/git").is_file())
}

/// The developer directory the stub would run Git from, found the way `xcode-select -p` finds
/// it, without running the stub.
fn developer_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DEVELOPER_DIR").filter(|dir| !dir.is_empty()) {
        return Some(dir.into());
    }
    let output = Command::new("/usr/bin/xcode-select")
        .arg("--print-path")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let printed = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !printed.trim().is_empty()).then(|| printed.trim().into())
}

/// The bytes under `path`, counting what can be read.
fn size_of(path: &Path) -> u64 {
    let Ok(metadata) = path.symlink_metadata() else {
        return 0;
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| size_of(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_git(script: &str) -> (tempfile::TempDir, Git) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("git");
        std::fs::write(&program, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let git = Git {
            program,
            timeout: Duration::from_millis(500),
            ..Git::default()
        };
        (dir, git)
    }

    #[test]
    fn the_macos_git_stub_counts_as_missing_without_developer_tools() {
        let tools = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tools.path().join("usr/bin")).unwrap();
        std::fs::write(tools.path().join("usr/bin/git"), "").unwrap();
        let stub = Path::new("/usr/bin/git");
        assert!(needs_developer_tools(stub, || None));
        assert!(needs_developer_tools(stub, || Some("/nowhere".into())));
        assert!(!needs_developer_tools(stub, || Some(tools.path().to_owned())));
        assert!(!needs_developer_tools(&tools.path().join("usr/bin/git"), || None));
    }

    #[test]
    fn git_is_looked_up_on_path_without_running_it() {
        let (dir, git) = fake_git("exit 1");
        assert_eq!(
            find_program(Path::new("git"), Some(dir.path().as_os_str().to_owned())),
            Some(git.program.clone())
        );
        assert_eq!(find_program(Path::new("git"), Some("/nowhere".into())), None);
        assert_eq!(find_program(Path::new("git"), None), None);
        let missing = Git {
            program: dir.path().join("absent"),
            ..git
        };
        let error = missing
            .resolve(&dir.path().join("repo.git"), "file:///x", None)
            .unwrap_err();
        assert_eq!(error.code, "git-unavailable", "{error}");
        assert!(!dir.path().join("repo.git").exists());
    }

    #[test]
    fn a_timeout_stops_the_helpers_git_started_too() {
        let (dir, git) = fake_git("sleep 30 &\nsleep 30");
        let started = Instant::now();
        let error = git.init(&dir.path().join("repo.git")).unwrap_err();
        assert!(error.message.contains("longer than"), "{error}");
        // The output readers return only once every process holding the pipes is gone.
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn output_and_fetched_bytes_are_capped_while_git_runs() {
        let (dir, mut git) = fake_git("exec yes");
        git.timeout = Duration::from_secs(20);
        assert_eq!(
            git.init(&dir.path().join("repo.git")).unwrap_err().code,
            "package-too-large"
        );

        let (dir, mut git) = fake_git(
            "while [ \"$1\" != --git-dir ]; do shift; done\nwhile :; do head -c 65536 /dev/zero >> \"$2/pack\"; done",
        );
        git.timeout = Duration::from_secs(20);
        git.max_fetch_bytes = 1024 * 1024;
        let repo = dir.path().join("repo.git");
        std::fs::create_dir(&repo).unwrap();
        let started = Instant::now();
        let error = git.fetch(&repo, "file:///nowhere", &["HEAD"], true).unwrap_err();
        assert_eq!(error.code, "package-too-large", "{error}");
        assert!(started.elapsed() < Duration::from_secs(20));
    }
}
