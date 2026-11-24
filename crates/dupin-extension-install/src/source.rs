use std::path::{Path, PathBuf};

use crate::Error;

/// Where a package comes from, as `dupin ext install` takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// `github:owner/repo[@ref]`.
    GitHub {
        owner: String,
        repo: String,
        rev: Option<String>,
    },
    /// An `https://`, `ssh://` or `file://` URL, or scp-style `user@host:path`, with `@ref`.
    Git { url: String, rev: Option<String> },
    /// A folder on this Mac, copied as it is now.
    Path(PathBuf),
}

const SPEC_HINT: &str = "Use `github:owner/repo`, `github:owner/repo@v1.0.0`, an https:// or ssh:// Git URL, or a folder path such as `./word-count`.";

impl Source {
    /// Reads `spec`; a relative folder path resolves against `cwd`.
    pub(crate) fn parse(spec: &str, cwd: &Path) -> Result<Self, Error> {
        let invalid = |message: &str| Error::new("invalid-source", message.to_owned()).hint(SPEC_HINT);
        if spec.is_empty() || spec.chars().any(char::is_control) {
            return Err(invalid("must be non-empty, without control characters"));
        }
        let remote = spec.starts_with("github:") || spec.contains("://") || scp_colon(spec).is_some();
        if remote && spec.chars().any(char::is_whitespace) {
            return Err(invalid("must not contain spaces"));
        }
        if spec.starts_with('-') {
            return Err(invalid("must not start with `-`"));
        }
        if let Some(rest) = spec.strip_prefix("github:") {
            let (path, rev) = split_rev(rest, 0)?;
            let (owner, repo) = path
                .split_once('/')
                .ok_or_else(|| invalid("must name `github:owner/repo`"))?;
            let repo = repo.strip_suffix(".git").unwrap_or(repo);
            if !github_name(owner) || !github_name(repo) {
                return Err(invalid(
                    "must name a GitHub owner and repository of letters, digits, `.`, `-` or `_`",
                ));
            }
            return Ok(Self::GitHub {
                owner: owner.into(),
                repo: repo.into(),
                rev,
            });
        }
        if let Some((scheme, after)) = spec.split_once("://") {
            if !matches!(scheme, "https" | "ssh" | "file") {
                return Err(Error::new(
                    "invalid-source",
                    format!("uses `{scheme}://`, which Dupin doesn't fetch from"),
                )
                .hint(
                    "Use an https://, ssh:// or file:// URL; plain http:// and git:// aren't authenticated.",
                ));
            }
            let authority = &after[..after.find('/').unwrap_or(after.len())];
            if authority
                .rsplit_once('@')
                .is_some_and(|(userinfo, _)| userinfo.contains(':'))
            {
                return Err(Error::new(
                    "invalid-source",
                    "carries a password or token, which would be stored in extensions.lock and shown in messages",
                )
                .hint("Drop it from the URL and let Git supply it, for example from a credential helper or an SSH key."));
            }
            let path_start = scheme.len() + 3 + authority.len();
            let (url, rev) = split_rev(spec, path_start)?;
            return Ok(Self::Git { url, rev });
        }
        if let Some(colon) = scp_colon(spec) {
            let (url, rev) = split_rev(spec, colon + 1)?;
            return Ok(Self::Git { url, rev });
        }
        let path = match spec.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(rest))
                .ok_or_else(|| invalid("starts with `~/`, but HOME isn't set"))?,
            None => cwd.join(spec),
        };
        // One folder keeps one locator however it was spelt, so reinstalling it isn't taken
        // for another source.
        Ok(Self::Path(path.components().collect()))
    }

    /// The source without its ref, as the lock file records it.
    pub(crate) fn locator(&self) -> String {
        match self {
            Self::GitHub { owner, repo, .. } => format!("github:{owner}/{repo}"),
            Self::Git { url, .. } => url.clone(),
            Self::Path(path) => path.display().to_string(),
        }
    }

    pub(crate) fn with_rev(&self, rev: Option<String>) -> Self {
        match self.clone() {
            Self::GitHub { owner, repo, .. } => Self::GitHub { owner, repo, rev },
            Self::Git { url, .. } => Self::Git { url, rev },
            path @ Self::Path(_) => path,
        }
    }
}

fn github_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// `user@host:path`, which Git reads as SSH, as long as no `/` comes before the colon.
fn scp_colon(spec: &str) -> Option<usize> {
    let colon = spec.find(':')?;
    let host = &spec[..colon];
    (host.contains('@') && !host.contains('/') && colon + 1 < spec.len()).then_some(colon)
}

/// Splits a trailing `@ref` off `spec`, looking only past `path_start` so `user@host` stays.
fn split_rev(spec: &str, path_start: usize) -> Result<(String, Option<String>), Error> {
    let Some(at) = spec[path_start..].rfind('@').map(|at| at + path_start) else {
        return Ok((spec.to_owned(), None));
    };
    let (location, rev) = (&spec[..at], &spec[at + 1..]);
    if !valid_rev(rev) {
        return Err(Error::new(
            "invalid-source",
            format!("`{rev}` is not a tag, branch or commit"),
        )
        .hint("Name a tag such as `v1.0.0`, a branch, or a full 40-character commit SHA after `@`."));
    }
    Ok((location.to_owned(), Some(rev.to_owned())))
}

pub(crate) fn valid_rev(rev: &str) -> bool {
    !rev.is_empty()
        && rev.len() <= 255
        && !rev.starts_with('-')
        && !rev.contains("..")
        && !rev.contains("@{")
        && rev
            .strip_prefix("refs/")
            .is_none_or(|name| name.starts_with("heads/") || name.starts_with("tags/"))
        && !rev.chars().any(|ch| {
            ch.is_control() || ch.is_whitespace() || matches!(ch, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
}

pub(crate) fn is_full_commit(rev: &str) -> bool {
    matches!(rev.len(), 40 | 64) && rev.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(spec: &str) -> Result<Source, Error> {
        Source::parse(spec, Path::new("/work"))
    }

    #[test]
    fn github_sources_take_an_optional_ref() {
        assert_eq!(
            parse("github:ryo/word-count@v1.2.0").unwrap(),
            Source::GitHub {
                owner: "ryo".into(),
                repo: "word-count".into(),
                rev: Some("v1.2.0".into())
            }
        );
        let plain = parse("github:ryo/word-count.git").unwrap();
        assert_eq!(plain.locator(), "github:ryo/word-count");
        assert!(matches!(plain, Source::GitHub { rev: None, .. }));
        for bad in [
            "github:ryo",
            "github:../x",
            "github:ryo/a b",
            "github:ryo/x@",
            "github:ryo/x@-c",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn urls_keep_user_info_and_split_the_ref_after_the_host() {
        assert_eq!(
            parse("https://user@example.com/team/ext.git@feature/x").unwrap(),
            Source::Git {
                url: "https://user@example.com/team/ext.git".into(),
                rev: Some("feature/x".into())
            }
        );
        assert_eq!(
            parse("git@github.com:ryo/ext.git").unwrap(),
            Source::Git {
                url: "git@github.com:ryo/ext.git".into(),
                rev: None
            }
        );
        assert!(matches!(
            parse("git@github.com:ryo/ext.git@v1").unwrap(),
            Source::Git { rev: Some(rev), .. } if rev == "v1"
        ));
        assert!(parse("http://example.com/x.git").is_err());
        assert!(matches!(parse("ext::sh -c touch").unwrap(), Source::Path(_)));
        assert!(parse("https://example.com/a b.git").is_err());
        assert!(parse("git://example.com/x.git").is_err());
        assert!(parse("--upload-pack=touch").is_err());
        let token = parse("https://me:ghp_secret@github.com/me/ext.git").unwrap_err();
        assert!(!token.to_string().contains("ghp_secret"), "{token}");
        assert!(parse("ssh://me:pw@example.com/ext.git").is_err());
    }

    #[test]
    fn anything_else_is_a_folder() {
        assert_eq!(
            parse("./word-count").unwrap(),
            Source::Path(PathBuf::from("/work/word-count"))
        );
        assert_eq!(parse("/abs/x").unwrap(), Source::Path(PathBuf::from("/abs/x")));
        assert_eq!(parse("/My Extensions/x").unwrap().locator(), "/My Extensions/x");
        assert!(is_full_commit(&"a".repeat(40)));
        assert!(!is_full_commit("abc1234"));
    }
}
