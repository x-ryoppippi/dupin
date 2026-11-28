use std::fmt;

use serde::{Deserialize, Serialize};

use super::AliasTarget;

pub(super) const DEFAULT_HOTKEY: &str = "cmd+space";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "target", rename_all = "snake_case", deny_unknown_fields)]
pub enum HotkeyTarget {
    App {
        bundle_id: String,
        #[serde(default)]
        hide_if_frontmost: bool,
    },
    Command {
        command: String,
    },
    Script {
        script: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    /// Reaches what `command` addresses: a command shows the panel in it with `query` typed,
    /// and a row runs as choosing it does.
    Dupin {
        command: Address,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "query")]
        query: Option<String>,
    },
}

const MAX_QUERY_BYTES: usize = 1024;

const MAX_ROW_BYTES: usize = 256;

/// What a `target = "dupin"` binding reaches: anything an alias reaches, written as an alias's
/// target is, or one row of a command's list.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Address {
    Alias(AliasTarget),
    /// `<command>/<row>`: the row with the ID `row` among those of the command `command`
    /// names as an alias names it, such as `running-apps/com.apple.Safari`; `system` names the
    /// system commands.
    Row {
        command: String,
        row: String,
    },
}

impl Address {
    pub fn parse(address: &str) -> Result<Self, String> {
        if let Some((command, row)) = address.split_once('/')
            && !command.contains(':')
        {
            super::check_command_name(command)?;
            if row.is_empty() || row.len() > MAX_ROW_BYTES || row.chars().any(char::is_control) {
                return Err(format!(
                    "a row is named after its command and a slash, by an ID of 1 to {MAX_ROW_BYTES} bytes on one line, such as `system/sleep`, not {address:?}"
                ));
            }
            return Ok(Self::Row {
                command: command.into(),
                row: row.into(),
            });
        }
        AliasTarget::parse(address).map(Self::Alias)
    }

    /// The command this names by name, to show the panel in.
    pub fn command(&self) -> Option<&str> {
        match self {
            Self::Alias(AliasTarget::Command(name)) => Some(name),
            _ => None,
        }
    }

    /// Whether a binding's `query` can be typed into what this reaches: a command's search or a
    /// web search.
    pub fn takes_query(&self) -> bool {
        matches!(self, Self::Alias(AliasTarget::Command(_) | AliasTarget::Web(_)))
    }
}

impl From<AliasTarget> for Address {
    fn from(target: AliasTarget) -> Self {
        Self::Alias(target)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Alias(target) => target.fmt(f),
            Self::Row { command, row } => write!(f, "{command}/{row}"),
        }
    }
}

impl Serialize for Address {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let address = String::deserialize(deserializer)?;
        Self::parse(&address).map_err(serde::de::Error::custom)
    }
}

fn query<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let query = String::deserialize(deserializer)?;
    if query.len() > MAX_QUERY_BYTES || query.chars().any(char::is_control) {
        return Err(serde::de::Error::custom(format!(
            "a binding's query is at most {MAX_QUERY_BYTES} bytes on one line"
        )));
    }
    Ok(Some(query))
}

#[cfg(test)]
mod tests {
    use super::{Address, HotkeyTarget};
    use crate::config::{AliasTarget, Config};

    #[test]
    fn defaults_to_command_space() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::default().hotkey, "cmd+space");
        assert!(Config::default().selection_hotkey.is_empty());
    }

    #[test]
    fn reads_the_hotkeys() {
        let config = Config::parse("hotkey = \"alt+space\"\nselection_hotkey = \"ctrl+alt+k\"").unwrap();
        assert_eq!(config.hotkey, "alt+space");
        assert_eq!(config.selection_hotkey, "ctrl+alt+k");
    }

    #[test]
    fn app_bindings_round_trip_with_hiding_opt_in() {
        let config = Config::parse(
            r#"
[bindings."ctrl+alt+c"]
target = "app"
bundle_id = "com.apple.calculator"
"#,
        )
        .unwrap();
        assert!(Config::default().bindings.is_empty());
        assert_eq!(
            config.bindings["ctrl+alt+c"],
            HotkeyTarget::App {
                bundle_id: "com.apple.calculator".into(),
                hide_if_frontmost: false,
            }
        );
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
    }

    #[test]
    fn command_bindings_round_trip_without_changing_the_command_line() {
        let config = Config::parse(
            r#"
[bindings."alt+cmd+g"]
target = "command"
command = "dupin open --query 'gh '"
"#,
        )
        .unwrap();
        assert_eq!(
            config.bindings["alt+cmd+g"],
            HotkeyTarget::Command {
                command: "dupin open --query 'gh '".into(),
            }
        );
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
    }

    #[test]
    fn dupin_bindings_name_a_command_and_optionally_a_query() {
        let config = Config::parse(
            r#"
[bindings."cmd+shift+v"]
target = "dupin"
command = "clipboard"

[bindings."alt+cmd+g"]
target = "dupin"
command = "word-count"
query = "draft "
"#,
        )
        .unwrap();
        assert_eq!(
            config.bindings["cmd+shift+v"],
            HotkeyTarget::Dupin {
                command: AliasTarget::Command("clipboard".into()).into(),
                query: None,
            }
        );
        assert_eq!(
            config.bindings["alt+cmd+g"],
            HotkeyTarget::Dupin {
                command: AliasTarget::Command("word-count".into()).into(),
                query: Some("draft ".into()),
            }
        );
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
    }

    #[test]
    fn a_bad_dupin_binding_says_what_is_wrong() {
        for (binding, expected) in [
            (r#"command = "Clipboard""#, "a command is named in lowercase"),
            (
                r#"command = "url:https://example.com""#,
                "a target is a command name",
            ),
            (
                r#"command = "Window/left-half""#,
                "a command is named in lowercase",
            ),
            (r#"command = "system/""#, "a row is named after its command"),
            ("command = \"clipboard\"\nquery = \"a\\nb\"", "on one line"),
            ("command = \"clipboard\"\nkeyword = \"c\"", "unknown field"),
            (r#"query = "x""#, "missing field `command`"),
        ] {
            let source = format!("[bindings.\"cmd+shift+v\"]\ntarget = \"dupin\"\n{binding}");
            let error = Config::parse(&source).unwrap_err().to_string();
            assert!(error.contains(expected), "{binding}: {error}");
        }
    }

    #[test]
    fn a_dupin_binding_addresses_what_an_alias_reaches_or_one_row() {
        let config = Config::parse(
            r#"
[bindings."alt+cmd+s"]
target = "dupin"
command = "system/sleep"

[bindings."alt+cmd+l"]
target = "dupin"
command = "running-apps/com.apple.Safari"

[bindings."alt+cmd+g"]
target = "dupin"
command = "web:g"
query = "rust"

[bindings."alt+cmd+f"]
target = "dupin"
command = "app:org.mozilla.firefox"

[bindings."alt+cmd+d"]
target = "dupin"
command = "file:~/Downloads"
"#,
        )
        .unwrap();
        let address = |chord: &str| match &config.bindings[chord] {
            HotkeyTarget::Dupin { command, .. } => command.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            address("alt+cmd+s"),
            Address::Row {
                command: "system".into(),
                row: "sleep".into(),
            }
        );
        assert_eq!(address("alt+cmd+l").to_string(), "running-apps/com.apple.Safari");
        assert_eq!(address("alt+cmd+g"), AliasTarget::Web("g".into()).into());
        assert_eq!(
            address("alt+cmd+f"),
            AliasTarget::App("org.mozilla.firefox".into()).into()
        );
        // A path's slashes don't make a row.
        assert_eq!(
            address("alt+cmd+d"),
            AliasTarget::File("~/Downloads".into()).into()
        );
        assert!(address("alt+cmd+g").takes_query() && !address("alt+cmd+s").takes_query());
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
    }
}
