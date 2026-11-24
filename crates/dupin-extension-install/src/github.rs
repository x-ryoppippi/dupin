//! Fetching a `github:` package without Git, which a Mac may not have: the REST API names the
//! commit, the macOS system curl downloads GitHub's tarball of exactly that commit, and Dupin
//! unpacks it itself after checking each file against the commit's tree.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::Error;
use crate::git::Fetched;
use crate::source::{is_full_commit, valid_rev};
use crate::tarball::{self, Blob};
use crate::tree::{Listing, MAX_TOTAL_BYTES, Stage};
use crate::version::newest_version;

/// The largest API response Dupin reads; a recursive tree of a package within the file limit is
/// far smaller.
const MAX_API_BYTES: u64 = 8 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How many annotated tags may point at each other before a commit.
const MAX_TAG_DEPTH: usize = 8;
/// The most tags GitHub lists in one page.
const TAGS_PER_PAGE: usize = 100;

/// How Dupin talks to GitHub.
#[derive(Clone)]
pub struct GitHub {
    /// The REST API root, without a trailing slash.
    pub api: String,
    pub curl: PathBuf,
    /// For each request, the tarball download included.
    pub timeout: Duration,
    pub max_download_bytes: u64,
    /// Sent to the API host only, for private repositories and a higher rate limit.
    pub token: Option<String>,
}

impl Default for GitHub {
    fn default() -> Self {
        Self {
            api: "https://api.github.com".to_owned(),
            curl: PathBuf::from("/usr/bin/curl"),
            timeout: Duration::from_secs(120),
            max_download_bytes: 4 * MAX_TOTAL_BYTES,
            token: std::env::var("GITHUB_TOKEN")
                .ok()
                .filter(|token| !token.is_empty()),
        }
    }
}

impl fmt::Debug for GitHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHub")
            .field("api", &self.api)
            .field("curl", &self.curl)
            .field("timeout", &self.timeout)
            .field("max_download_bytes", &self.max_download_bytes)
            .field("token", &self.token.as_ref().map(|_| "…"))
            .finish()
    }
}

#[derive(Deserialize)]
struct Repository {
    default_branch: String,
}

#[derive(Deserialize)]
struct NamedReference {
    #[serde(rename = "ref")]
    name: String,
    object: Object,
}

#[derive(Deserialize)]
struct Object {
    sha: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Reference {
    object: Object,
}

#[derive(Deserialize)]
struct Comparison {
    status: String,
}

/// One entry of the tag list, which names the commit each tag peels to.
#[derive(Deserialize)]
struct Tag {
    commit: Object,
}

#[derive(Deserialize)]
struct Tree {
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    size: Option<u64>,
}

/// A repository on GitHub, and the scratch folder its responses land in.
pub(crate) struct Repo<'a> {
    pub owner: &'a str,
    pub name: &'a str,
    pub scratch: &'a Path,
}

impl Repo<'_> {
    fn locator(&self) -> String {
        format!("github:{}/{}", self.owner, self.name)
    }

    fn path(&self) -> String {
        format!("/repos/{}/{}", self.owner, self.name)
    }
}

impl GitHub {
    /// The commit `rev` names: a tag, branch or full commit SHA, or, with none, the newest version
    /// tag, or the default branch when there is no version tag. It answers as Git would. `on` is
    /// the branch or tag a pinned commit was installed from, as extensions.lock records it.
    pub(crate) fn resolve(
        &self,
        repo: &Repo<'_>,
        rev: Option<&str>,
        on: Option<&str>,
    ) -> Result<Fetched, Error> {
        if rev.into_iter().chain(on).any(|rev| !valid_rev(rev)) {
            return Err(Error::new(
                "invalid-source",
                "is not a GitHub source Dupin can fetch",
            ));
        }
        let repository: Repository = self.api(repo, &repo.path())?.ok_or_else(|| {
            Error::new("unknown-source", format!("{} doesn't exist", repo.locator()))
                .hint("Check the owner and name. If the repository is private, set GITHUB_TOKEN to a token that can read it.")
        })?;
        match rev {
            Some(commit) if is_full_commit(commit) => {
                let found: Option<Object> =
                    self.api(repo, &format!("{}/git/commits/{commit}", repo.path()))?;
                let commit = match found {
                    Some(found) if found.sha.eq_ignore_ascii_case(commit) => found.sha.to_ascii_lowercase(),
                    _ => {
                        return Err(Error::new(
                            "unknown-ref",
                            format!("commit {commit} isn't in {}", repo.locator()),
                        ));
                    }
                };
                if !self.publishes(repo, on, &repository.default_branch, &commit)? {
                    let refs = match on {
                        Some(on) => format!("`{on}`, the default branch or a tag"),
                        None => "the default branch or a tag".to_owned(),
                    };
                    return Err(Error::new(
                        "unknown-ref",
                        format!("commit {commit} isn't on {refs} of {}", repo.locator()),
                    )
                    .hint("Install the branch or tag the commit is on by name."));
                }
                Ok(Fetched { commit, rev: None })
            }
            Some(name) => {
                let candidates = match name.strip_prefix("refs/") {
                    Some(exact) => vec![exact.to_owned()],
                    None => vec![format!("tags/{name}"), format!("heads/{name}")],
                };
                for candidate in candidates {
                    if let Some(commit) = self.commit_of_ref(repo, &candidate)? {
                        return Ok(Fetched {
                            commit,
                            rev: Some(name.to_owned()),
                        });
                    }
                }
                let mut error = Error::new(
                    "unknown-ref",
                    format!("{} has no tag or branch `{name}`", repo.locator()),
                );
                if name.len() < 40 && name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    error = error.hint("Name a commit by its full 40-character SHA.");
                }
                Err(error)
            }
            None => {
                let tags = self.tags(repo)?;
                let names: Vec<String> = tags.iter().map(|(name, _)| name.clone()).collect();
                if let Some(newest) = newest_version(&names)
                    && let Some((name, object)) = tags.into_iter().find(|(name, _)| name == newest)
                {
                    return Ok(Fetched {
                        commit: self.peel(repo, object, &format!("tags/{name}"))?,
                        rev: Some(name),
                    });
                }
                let branch = &repository.default_branch;
                let commit = self
                    .commit_of_ref(repo, &format!("heads/{branch}"))?
                    .ok_or_else(|| {
                        Error::new(
                            "unknown-ref",
                            format!("{} has no commits on `{branch}`", repo.locator()),
                        )
                    })?;
                Ok(Fetched { commit, rev: None })
            }
        }
    }

    /// Writes the files of `commit` into `stage`, refusing symlinks, submodules and anything past
    /// the package limits before downloading a byte of the archive.
    pub(crate) fn export(&self, repo: &Repo<'_>, commit: &str, stage: &Stage) -> Result<(), Error> {
        let tree: Tree = self
            .api(repo, &format!("{}/git/trees/{commit}?recursive=1", repo.path()))?
            .ok_or_else(|| {
                Error::new(
                    "unknown-ref",
                    format!("commit {commit} isn't in {}", repo.locator()),
                )
            })?;
        if tree.truncated {
            return Err(
                Error::new("package-too-large", "has more files than GitHub lists at once")
                    .hint("Ship only the built entry and the assets it needs."),
            );
        }
        let mut listing = Listing::new();
        let mut expected = BTreeMap::new();
        for entry in tree.tree {
            let path = entry.path;
            let executable = match (entry.mode.as_str(), entry.kind.as_str()) {
                ("040000", "tree") => continue,
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
                (mode, _) => {
                    return Err(Error::new("invalid-package-path", format!("has Git mode {mode}")).file(path));
                }
            };
            let size = entry.size.ok_or_else(|| {
                Error::new("download-failed", "GitHub listed a file without its size").file(path.clone())
            })?;
            listing.add(&path, size)?;
            expected.insert(
                path,
                Blob {
                    id: entry.sha.to_ascii_lowercase(),
                    size,
                    executable,
                },
            );
        }
        if expected.is_empty() {
            return Ok(());
        }
        let archive = repo.scratch.join("archive.tar.gz");
        let url = format!("{}{}/tarball/{commit}", self.api, repo.path());
        let status = self.get(&url, &archive, self.max_download_bytes)?;
        if status != 200 {
            return Err(self.refused(status, repo));
        }
        let file = std::fs::File::open(&archive)
            .map_err(|error| Error::io("io-error", "read the downloaded archive", error))?;
        tarball::extract(std::io::BufReader::new(file), &expected, stage)
    }

    /// Whether the repository's own history holds `commit`: the branch or tag `on`, or the
    /// default branch, reaches it, or a tag names it. GitHub serves every commit of a fork
    /// network through each repository in it, so a commit pushed only to a fork would otherwise
    /// install as this repository's.
    fn publishes(
        &self,
        repo: &Repo<'_>,
        on: Option<&str>,
        default_branch: &str,
        commit: &str,
    ) -> Result<bool, Error> {
        let on = on.map(|name| {
            name.strip_prefix("refs/heads/")
                .or_else(|| name.strip_prefix("refs/tags/"))
                .unwrap_or(name)
        });
        for base in on.into_iter().chain([default_branch]) {
            let compared: Option<Comparison> = self.api(
                repo,
                &format!("{}/compare/{}...{commit}", repo.path(), escape(base)),
            )?;
            if compared.is_some_and(|compared| matches!(compared.status.as_str(), "identical" | "behind")) {
                return Ok(true);
            }
        }
        // The tag list names the commit behind each tag, where the ref listing would take a
        // request per annotated tag to peel.
        for page in 1.. {
            let tags: Vec<Tag> = self
                .api(
                    repo,
                    &format!("{}/tags?per_page={TAGS_PER_PAGE}&page={page}", repo.path()),
                )?
                .unwrap_or_default();
            if tags.iter().any(|tag| tag.commit.sha.eq_ignore_ascii_case(commit)) {
                return Ok(true);
            }
            if tags.len() < TAGS_PER_PAGE {
                break;
            }
        }
        Ok(false)
    }

    /// Every tag of the repository, as `ls-remote --tags` lists them: GitHub answers this listing
    /// whole, where its tag list would take a request per hundred tags.
    fn tags(&self, repo: &Repo<'_>) -> Result<Vec<(String, Object)>, Error> {
        let refs: Vec<NamedReference> = self
            .api(repo, &format!("{}/git/matching-refs/tags", repo.path()))?
            .unwrap_or_default();
        Ok(refs
            .into_iter()
            .filter_map(|reference| {
                let name = reference.name.strip_prefix("refs/tags/")?.to_owned();
                Some((name, reference.object))
            })
            .collect())
    }

    /// The commit a ref such as `tags/v1.0.0` points at, through any annotated tags, or `None`
    /// when there is no such ref.
    fn commit_of_ref(&self, repo: &Repo<'_>, name: &str) -> Result<Option<String>, Error> {
        let Some(reference) =
            self.api::<Reference>(repo, &format!("{}/git/ref/{}", repo.path(), escape(name)))?
        else {
            return Ok(None);
        };
        self.peel(repo, reference.object, name).map(Some)
    }

    /// The commit `object`, the target of ref `name`, points at through any annotated tags.
    fn peel(&self, repo: &Repo<'_>, mut object: Object, name: &str) -> Result<String, Error> {
        for _ in 0..MAX_TAG_DEPTH {
            match object.kind.as_deref() {
                Some("commit") => return full_commit(&object.sha),
                Some("tag") => {
                    let tag: Reference = self
                        .api(repo, &format!("{}/git/tags/{}", repo.path(), object.sha))?
                        .ok_or_else(|| {
                            Error::new(
                                "download-failed",
                                format!("GitHub lost tag object {}", object.sha),
                            )
                        })?;
                    object = tag.object;
                }
                _ => break,
            }
        }
        Err(Error::new(
            "unknown-ref",
            format!("`{name}` in {} doesn't point at a commit", repo.locator()),
        ))
    }

    /// A JSON response from the API, or `None` when GitHub says there is no such thing.
    fn api<T: DeserializeOwned>(&self, repo: &Repo<'_>, path: &str) -> Result<Option<T>, Error> {
        let out = repo.scratch.join("response.json");
        let status = self.get(&format!("{}{path}", self.api), &out, MAX_API_BYTES)?;
        match status {
            200 => {}
            404 | 409 | 422 => return Ok(None),
            status => return Err(self.refused(status, repo)),
        }
        let bytes =
            std::fs::read(&out).map_err(|error| Error::io("io-error", "read GitHub's response", error))?;
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            Error::new(
                "download-failed",
                format!("GitHub answered in an unexpected form: {error}"),
            )
        })
    }

    fn refused(&self, status: u16, repo: &Repo<'_>) -> Error {
        match status {
            401 => Error::new("download-failed", "GitHub refused GITHUB_TOKEN (HTTP 401)")
                .hint("Set GITHUB_TOKEN to a valid token, or unset it for a public repository."),
            403 | 429 => Error::new(
                "download-failed",
                format!("GitHub refused to serve {} (HTTP {status})", repo.locator()),
            )
            .hint(if self.token.is_some() {
                "GitHub may be rate limiting this token; wait a while and try again."
            } else {
                "GitHub allows 60 requests an hour without a token; wait, or set GITHUB_TOKEN."
            }),
            404 => Error::new(
                "unknown-ref",
                format!("GitHub has no archive of {}", repo.locator()),
            ),
            status => Error::new(
                "download-failed",
                format!("GitHub answered HTTP {status} for {}", repo.locator()),
            ),
        }
    }

    /// Downloads `url` into `out` with the system curl, following redirects on the API's own
    /// scheme, and returns the final status. The options, the token among them, go through stdin
    /// so no other process can read them from curl's arguments.
    fn get(&self, url: &str, out: &Path, max_bytes: u64) -> Result<u16, Error> {
        let scheme = self.api.split_once("://").map_or("https", |(scheme, _)| scheme);
        let mut config = Config::default();
        for flag in ["silent", "show-error", "globoff", "location"] {
            config.flag(flag);
        }
        config.option("proto", &format!("={scheme}"));
        config.option("proto-redir", &format!("={scheme}"));
        config.option("max-redirs", "5");
        let max_time = Duration::from_secs(self.timeout.as_secs().max(1));
        config.option("connect-timeout", &CONNECT_TIMEOUT.as_secs().to_string());
        config.option("max-time", &max_time.as_secs().to_string());
        config.option("max-filesize", &max_bytes.to_string());
        config.option("user-agent", "Dupin");
        config.option("header", "Accept: application/vnd.github+json");
        config.option("header", "X-GitHub-Api-Version: 2022-11-28");
        if let Some(token) = &self.token {
            // curl drops a custom Authorization header when a redirect leaves the API host, so
            // the token never reaches the archive host.
            config.option("header", &format!("Authorization: Bearer {token}"));
        }
        config.option("url", url);
        config.option("output", &out.display().to_string());
        config.option("write-out", "%{http_code}");
        let _ = std::fs::remove_file(out);

        let started = Instant::now();
        let mut child = Command::new(&self.curl)
            .args(["--disable", "--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                Error::new(
                    "download-failed",
                    format!("couldn't run {}: {error}", self.curl.display()),
                )
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(config.0.as_bytes());
        }
        let output = child
            .wait_with_output()
            .map_err(|error| Error::io("download-failed", "wait for curl", error))?;
        let too_large = || {
            Error::new(
                "package-too-large",
                format!("GitHub sent more than {} MiB", max_bytes / 1024 / 1024),
            )
            .hint("Keep the package in a small repository of its own, or install from a folder.")
        };
        match output.status.code() {
            Some(0) => {}
            Some(63) => return Err(too_large()),
            // curl exits 28 for both limits; only the whole-transfer one can take `max_time`.
            Some(28) if started.elapsed() < max_time => {
                return Err(Error::new(
                    "download-failed",
                    format!(
                        "couldn't connect to GitHub within {} seconds",
                        CONNECT_TIMEOUT.as_secs()
                    ),
                )
                .hint("Check the network: a firewall, VPN or captive portal may be blocking GitHub."));
            }
            Some(28) => {
                return Err(Error::new(
                    "download-failed",
                    format!("GitHub took longer than {} seconds", max_time.as_secs()),
                ));
            }
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::new(
                    "download-failed",
                    format!("couldn't reach GitHub: {}", stderr.trim()),
                ));
            }
        }
        if out.metadata().is_ok_and(|metadata| metadata.len() > max_bytes) {
            return Err(too_large());
        }
        std::str::from_utf8(&output.stdout)
            .ok()
            .and_then(|code| code.trim().parse().ok())
            .ok_or_else(|| Error::new("download-failed", "curl didn't report a status"))
    }
}

fn full_commit(sha: &str) -> Result<String, Error> {
    if is_full_commit(sha) {
        Ok(sha.to_ascii_lowercase())
    } else {
        Err(Error::new(
            "download-failed",
            format!("GitHub named `{sha}` as a commit"),
        ))
    }
}

/// A ref name as one URL path, its `/` kept as separators.
fn escape(name: &str) -> String {
    let mut escaped = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("%{byte:02X}"));
        }
    }
    escaped
}

/// A curl config file: one option per line, values quoted.
#[derive(Default)]
struct Config(String);

impl Config {
    fn flag(&mut self, name: &str) {
        self.0.push_str(name);
        self.0.push('\n');
    }

    fn option(&mut self, name: &str, value: &str) {
        self.0.push_str(name);
        self.0.push_str(" = \"");
        for ch in value.chars() {
            match ch {
                '\\' => self.0.push_str("\\\\"),
                '"' => self.0.push_str("\\\""),
                '\n' => self.0.push_str("\\n"),
                '\r' => self.0.push_str("\\r"),
                '\t' => self.0.push_str("\\t"),
                ch => self.0.push(ch),
            }
        }
        self.0.push_str("\"\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_names_are_escaped_per_byte_but_keep_their_slashes() {
        assert_eq!(escape("tags/v1.0.0"), "tags/v1.0.0");
        assert_eq!(escape("heads/feature/x+y#z"), "heads/feature/x%2By%23z");
        assert_eq!(escape("heads/ü"), "heads/%C3%BC");
    }

    #[test]
    fn the_token_stays_out_of_debug_output() {
        let github = GitHub {
            token: Some("ghp_secret".to_owned()),
            ..GitHub::default()
        };
        assert!(!format!("{github:?}").contains("ghp_secret"));
    }
}
