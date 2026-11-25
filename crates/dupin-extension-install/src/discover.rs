//! `dupin ext search`: the GitHub repositories tagged [`TOPIC`], each with what its package.json
//! on the default branch requests. Nobody reviews them, so everything GitHub or a repository
//! sends is shown only as plain, bounded text and nothing is installed.

use serde::Deserialize;

use crate::Error;
use crate::github::{GitHub, escape};
use crate::manifest::{self, MAX_PACKAGE_JSON_BYTES};
use crate::source::{github_name, valid_rev};
use crate::tree::Stage;

/// The topic a repository carries to be listed.
pub const TOPIC: &str = "dupin-extension";
/// The most repositories one search lists; each costs a package.json download.
pub const MAX_LISTED: usize = 20;
/// The longest query GitHub accepts, less the topic qualifier Dupin adds.
const MAX_QUERY_BYTES: usize = 200;
const MAX_SEARCH_BYTES: u64 = 1024 * 1024;
const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_TEXT_CHARS: usize = 120;
/// How many package.json downloads run at once.
const PARALLEL_READS: usize = 8;

/// What one search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// How many repositories GitHub says match, listed or not.
    pub total: u64,
    /// GitHub stopped searching before it had looked at every repository.
    pub incomplete: bool,
    pub repositories: Vec<Repository>,
}

/// One repository as `dupin ext search` lists it. Every string is plain text: no control or
/// bidirectional formatting characters, one line, bounded in length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub owner: String,
    pub repo: String,
    pub description: Option<String>,
    pub stars: u64,
    /// The day of the last push, as `YYYY-MM-DD`.
    pub pushed: Option<String>,
    pub default_branch: String,
    /// What package.json on the default branch declares, or why it can't be read as a package.
    pub package: Result<Manifest, String>,
}

impl Repository {
    /// `github:owner/repo`, which `dupin ext install` takes.
    pub fn source(&self) -> String {
        format!("github:{}/{}", self.owner, self.repo)
    }

    pub fn install_command(&self) -> String {
        format!("dupin ext install {}", self.source())
    }
}

/// The `dupin` key of a listed repository's package.json.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub title: String,
    pub keyword: String,
    /// The capability scopes it requests, as the broker names them.
    pub capabilities: Vec<String>,
}

#[derive(Deserialize)]
struct SearchResponse {
    total_count: u64,
    #[serde(default)]
    incomplete_results: bool,
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    name: String,
    owner: Owner,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    pushed_at: Option<String>,
    default_branch: String,
    #[serde(default)]
    topics: Vec<String>,
}

#[derive(Deserialize)]
struct Owner {
    login: String,
}

impl GitHub {
    /// Lists the repositories tagged [`TOPIC`] that match `query`, most stars first, reading each
    /// one's package.json from its default branch. A repository GitHub lists without the topic,
    /// or under a name `dupin ext install` wouldn't take, is left out.
    pub fn search(&self, query: Option<&str>) -> Result<Found, Error> {
        let query = query.map(str::trim).filter(|query| !query.is_empty());
        if let Some(query) = query
            && (query.len() > MAX_QUERY_BYTES || query.chars().any(char::is_control))
        {
            return Err(Error::new(
                "invalid-query",
                format!("must be at most {MAX_QUERY_BYTES} bytes, without control characters"),
            ));
        }
        let terms = match query {
            Some(query) => format!("topic:{TOPIC} {query}"),
            None => format!("topic:{TOPIC}"),
        };
        let scratch = Stage::new(&std::env::temp_dir(), "search")?;
        let out = scratch.root.join("search.json");
        let url = format!(
            "{}/search/repositories?q={}&sort=stars&order=desc&per_page={MAX_LISTED}",
            self.api,
            encode(&terms)
        );
        let status = self.get(&url, &out, MAX_SEARCH_BYTES, true)?;
        match status {
            200 => {}
            401 => {
                return Err(
                    Error::new("download-failed", "GitHub refused the token (HTTP 401)")
                        .hint("Set GITHUB_TOKEN or GH_TOKEN to a valid token, or unset both."),
                );
            }
            403 | 429 => {
                return Err(Error::new(
                    "rate-limited",
                    format!("GitHub refused the search (HTTP {status})"),
                )
                .hint(if self.token.is_some() {
                    "GitHub may be rate limiting this token; wait a minute and try again."
                } else {
                    "GitHub allows 10 searches a minute without a token; wait, or set GITHUB_TOKEN or GH_TOKEN."
                }));
            }
            422 => {
                return Err(Error::new("invalid-query", "isn't a search GitHub accepts")
                    .hint("Search with plain words, such as `dupin ext search clipboard`."));
            }
            status => {
                return Err(Error::new(
                    "download-failed",
                    format!("GitHub answered HTTP {status} to the search"),
                ));
            }
        }
        let bytes =
            std::fs::read(&out).map_err(|error| Error::io("io-error", "read GitHub's response", error))?;
        let response: SearchResponse = serde_json::from_slice(&bytes).map_err(|_| {
            Error::new(
                "download-failed",
                "GitHub answered the search in an unexpected form",
            )
        })?;
        let items: Vec<Item> = response
            .items
            .into_iter()
            .filter(|item| {
                github_name(&item.owner.login)
                    && github_name(&item.name)
                    && valid_rev(&item.default_branch)
                    && item.topics.iter().any(|topic| topic == TOPIC)
            })
            .take(MAX_LISTED)
            .collect();
        let mut packages = Vec::with_capacity(items.len());
        for (batch, chunk) in items.chunks(PARALLEL_READS).enumerate() {
            std::thread::scope(|scope| {
                let reads: Vec<_> = chunk
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let out = scratch
                            .root
                            .join(format!("package-{}.json", batch * PARALLEL_READS + index));
                        scope.spawn(move || self.package(item, &out))
                    })
                    .collect();
                for read in reads {
                    packages.push(
                        read.join()
                            .unwrap_or_else(|_| Err("couldn't read package.json".to_owned())),
                    );
                }
            });
        }
        let repositories = items
            .into_iter()
            .zip(packages)
            .map(|(item, package)| Repository {
                description: item
                    .description
                    .as_deref()
                    .map(|description| plain(description, MAX_DESCRIPTION_CHARS))
                    .filter(|description| !description.is_empty()),
                stars: item.stargazers_count,
                pushed: item.pushed_at.as_deref().and_then(day),
                owner: item.owner.login,
                repo: item.name,
                default_branch: plain(&item.default_branch, MAX_TEXT_CHARS),
                package,
            })
            .collect();
        Ok(Found {
            total: response.total_count,
            incomplete: response.incomplete_results,
            repositories,
        })
    }

    /// The package.json of `item`'s default branch, read from the raw host without the token,
    /// since a listed repository is public.
    fn package(&self, item: &Item, out: &std::path::Path) -> Result<Manifest, String> {
        let branch = plain(&item.default_branch, MAX_TEXT_CHARS);
        let url = format!(
            "{}/{}/{}/{}/{}",
            self.raw,
            item.owner.login,
            item.name,
            escape(&item.default_branch),
            manifest::PACKAGE_JSON
        );
        let status = match self.get(&url, out, MAX_PACKAGE_JSON_BYTES, false) {
            Ok(status) => status,
            Err(error) if error.code == "package-too-large" => {
                return Err("package.json is larger than 64 KiB".to_owned());
            }
            Err(error) => return Err(plain(&error.to_string(), MAX_DESCRIPTION_CHARS)),
        };
        match status {
            200 => {}
            404 => return Err(format!("no package.json on `{branch}`")),
            status => return Err(format!("GitHub answered HTTP {status} for package.json")),
        }
        let bytes = std::fs::read(out).map_err(|_| "couldn't read package.json".to_owned())?;
        let package = manifest::parse_bytes(&bytes)
            .map_err(|error| plain(&format!("not a Dupin package: {error}"), MAX_DESCRIPTION_CHARS))?;
        Ok(Manifest {
            name: package.name,
            version: plain(&package.version, MAX_TEXT_CHARS),
            title: plain(&package.title, MAX_TEXT_CHARS),
            keyword: plain(&package.keyword, MAX_TEXT_CHARS),
            capabilities: package
                .capabilities
                .iter()
                .map(|scope| plain(scope, MAX_DESCRIPTION_CHARS))
                .collect(),
        })
    }
}

/// `text` as one line a terminal shows literally: control and invisible formatting characters
/// dropped, each run of whitespace one space, and at most `max` characters before an ellipsis.
pub(crate) fn plain(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = count > 0;
            continue;
        }
        if ch.is_control() || invisible(ch) {
            continue;
        }
        let needed = usize::from(space) + 1;
        if count + needed > max {
            out.push('…');
            return out;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(ch);
        count += needed;
    }
    out
}

/// Characters that reorder or hide the text around them.
fn invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// The date of an ISO 8601 timestamp such as `2026-10-01T09:00:00Z`.
fn day(timestamp: &str) -> Option<String> {
    let date = timestamp.get(..10)?;
    let shape = date.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 4 | 7) {
            byte == b'-'
        } else {
            byte.is_ascii_digit()
        }
    });
    shape.then(|| date.to_owned())
}

/// `text` as a URL query value.
fn encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_keeps_no_terminal_controls() {
        assert_eq!(
            plain("\u{1b}[2J\u{1b}]8;;https://evil\u{7}click\u{1b}]8;;\u{7}", 80),
            "[2J]8;;https://evilclick]8;;"
        );
        assert_eq!(plain("a\u{202E}gnp.exe\u{2066}b\u{200B}c", 80), "agnp.exebc");
        assert_eq!(plain("  one\n\ttwo\u{2028}three  ", 80), "one two three");
        assert_eq!(plain("\u{9b}31m red", 80), "31m red");
        assert_eq!(plain("abcdef", 3), "abc…");
        assert_eq!(plain("ab cd", 3), "ab…");
        assert_eq!(plain("abc", 3), "abc");
    }

    #[test]
    fn only_a_date_shape_is_kept() {
        assert_eq!(day("2026-10-01T09:00:00Z").as_deref(), Some("2026-10-01"));
        assert_eq!(day("2026-10-0\u{1b}T"), None);
        assert_eq!(day("yesterday"), None);
    }

    #[test]
    fn queries_are_encoded_whole() {
        assert_eq!(
            encode("topic:dupin-extension a&b=c #d"),
            "topic%3Adupin-extension%20a%26b%3Dc%20%23d"
        );
    }
}
