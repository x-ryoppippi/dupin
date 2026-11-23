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
}
