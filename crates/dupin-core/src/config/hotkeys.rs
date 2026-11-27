use serde::{Deserialize, Serialize};

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
    /// Shows the panel in one of Dupin's own commands, named as `[aliases]` name them, with
    /// `query` typed.
    Dupin {
        #[serde(deserialize_with = "command_name")]
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "query")]
        query: Option<String>,
    },
}

const MAX_QUERY_BYTES: usize = 1024;

fn command_name<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let name = String::deserialize(deserializer)?;
    super::check_command_name(&name).map_err(serde::de::Error::custom)?;
    Ok(name)
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
    use super::HotkeyTarget;
    use crate::config::Config;

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
                command: "clipboard".into(),
                query: None,
            }
        );
        assert_eq!(
            config.bindings["alt+cmd+g"],
            HotkeyTarget::Dupin {
                command: "word-count".into(),
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
                r#"command = "app:com.apple.Safari""#,
                "a command is named in lowercase",
            ),
            ("command = \"clipboard\"\nquery = \"a\\nb\"", "on one line"),
            ("command = \"clipboard\"\nkeyword = \"c\"", "unknown field"),
            (r#"query = "x""#, "missing field `command`"),
        ] {
            let source = format!("[bindings.\"cmd+shift+v\"]\ntarget = \"dupin\"\n{binding}");
            let error = Config::parse(&source).unwrap_err().to_string();
            assert!(error.contains(expected), "{binding}: {error}");
        }
    }
}
