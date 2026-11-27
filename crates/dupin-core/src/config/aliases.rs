//! `[aliases]`: shorthands the user types in the main search, each reaching a command, an
//! app, a web search or a file.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The most aliases config.toml takes.
pub const MAX_ALIASES: usize = 64;
const MAX_ALIAS_CHARS: usize = 32;
const MAX_TARGET_BYTES: usize = 1024;

/// Shorthands by what is typed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "BTreeMap<String, AliasTarget>")]
pub struct Aliases(pub BTreeMap<String, AliasTarget>);

impl Aliases {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<BTreeMap<String, AliasTarget>> for Aliases {
    type Error = String;

    fn try_from(aliases: BTreeMap<String, AliasTarget>) -> Result<Self, String> {
        if aliases.len() > MAX_ALIASES {
            return Err(format!(
                "aliases has {} entries; it takes at most {MAX_ALIASES}",
                aliases.len()
            ));
        }
        for alias in aliases.keys() {
            check_alias(alias).map_err(|problem| format!("aliases.{alias:?}: {problem}"))?;
        }
        Ok(Self(aliases))
    }
}

/// Whether `alias` can be typed as a keyword: up to 32 ASCII letters, digits, underscores or
/// hyphens, as a web search's keyword is.
pub fn check_alias(alias: &str) -> Result<(), String> {
    if alias.is_empty()
        || alias.len() > MAX_ALIAS_CHARS
        || !alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(format!(
            "an alias is 1 to {MAX_ALIAS_CHARS} ASCII letters, digits, underscores or hyphens"
        ));
    }
    Ok(())
}

/// Whether `name` can name a command, as an alias or a `target = "dupin"` binding does: a
/// built-in's name or an extension's package name.
pub fn check_command_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!(
            "a command is named in lowercase letters, digits and hyphens, such as `calendar` or an extension's package name, not {name:?}"
        ));
    }
    Ok(())
}

/// What an alias reaches, written as a string in config.toml.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AliasTarget {
    /// A command by name: a built-in's (`calendar`, `dictionary`, `files`, …) or an
    /// extension's package name. Typing the alias and Space enters it.
    Command(String),
    /// `app:<bundle id>`: the app, shown first while the alias is typed.
    App(String),
    /// `web:<keyword>`: a web search, which the alias takes a query for as its keyword does.
    Web(String),
    /// `file:<path>`: a file or folder by absolute or `~/` path, shown first while the alias is
    /// typed.
    File(String),
}

impl AliasTarget {
    pub fn parse(target: &str) -> Result<Self, String> {
        if target.len() > MAX_TARGET_BYTES || target.chars().any(char::is_control) {
            return Err(format!(
                "a target is at most {MAX_TARGET_BYTES} bytes on one line"
            ));
        }
        let parsed = match target.split_once(':') {
            Some(("app", bundle)) => {
                if bundle.is_empty()
                    || !bundle
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
                {
                    return Err(format!(
                        "`app:` takes a bundle identifier such as `app:com.apple.Safari`, not {target:?}"
                    ));
                }
                Self::App(bundle.into())
            }
            Some(("web", keyword)) => {
                check_alias(keyword).map_err(|_| {
                    format!("`web:` takes a web search's keyword such as `web:g`, not {target:?}")
                })?;
                Self::Web(keyword.into())
            }
            Some(("file", path)) => {
                if !(path.starts_with('/') || path.starts_with("~/")) {
                    return Err(format!(
                        "`file:` takes an absolute or `~/` path such as `file:~/Documents`, not {target:?}"
                    ));
                }
                Self::File(path.into())
            }
            Some(_) => {
                return Err(format!(
                    "a target is a command name, `app:<bundle id>`, `web:<keyword>` or `file:<path>`, not {target:?}"
                ));
            }
            None => {
                check_command_name(target)?;
                Self::Command(target.into())
            }
        };
        Ok(parsed)
    }
}

impl fmt::Display for AliasTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(name) => f.write_str(name),
            Self::App(bundle) => write!(f, "app:{bundle}"),
            Self::Web(keyword) => write!(f, "web:{keyword}"),
            Self::File(path) => write!(f, "file:{path}"),
        }
    }
}

impl Serialize for AliasTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AliasTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let target = String::deserialize(deserializer)?;
        Self::parse(&target).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn aliases_round_trip_each_kind_of_target() {
        let config = Config::parse(
            r#"
[aliases]
c = "calendar"
ff = "app:org.mozilla.firefox"
goo = "web:g"
dl = "file:~/Downloads"
root = "file:/"
wc = "word-count"
"#,
        )
        .unwrap();
        let aliases = &config.aliases.0;
        assert_eq!(aliases["c"], AliasTarget::Command("calendar".into()));
        assert_eq!(aliases["ff"], AliasTarget::App("org.mozilla.firefox".into()));
        assert_eq!(aliases["goo"], AliasTarget::Web("g".into()));
        assert_eq!(aliases["dl"], AliasTarget::File("~/Downloads".into()));
        assert_eq!(aliases["root"], AliasTarget::File("/".into()));
        assert_eq!(aliases["wc"], AliasTarget::Command("word-count".into()));
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
        assert!(Config::default().aliases.is_empty());
        assert!(!toml::to_string(&Config::default()).unwrap().contains("aliases"));
    }

    #[test]
    fn a_bad_alias_names_the_entry_and_what_is_wrong() {
        for (source, expected) in [
            (
                r#"aliases."has space" = "calendar""#,
                r#"aliases."has space": an alias is"#,
            ),
            (r#"aliases.x = "Calendar""#, "a command is named in lowercase"),
            (r#"aliases.x = "app:""#, "`app:` takes a bundle identifier"),
            (r#"aliases.x = "web:g q""#, "`web:` takes a web search's keyword"),
            (
                r#"aliases.x = "file:Documents""#,
                "`file:` takes an absolute or `~/` path",
            ),
            (
                r#"aliases.x = "url:https://example.com""#,
                "a target is a command name",
            ),
            (r#"aliases.x = 1"#, "invalid type"),
        ] {
            let error = Config::parse(source).unwrap_err().to_string();
            assert!(error.contains(expected), "{source}: {error}");
        }
        let many: String = (0..=MAX_ALIASES)
            .map(|index| format!("aliases.a{index} = \"calendar\"\n"))
            .collect();
        assert!(
            Config::parse(&many)
                .unwrap_err()
                .to_string()
                .contains("at most 64")
        );
    }
}
