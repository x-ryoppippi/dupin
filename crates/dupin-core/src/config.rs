//! `config.toml`: user settings. Every key is optional; a missing file is the defaults.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The global shortcut that shows and hides the panel, as Rune parses accelerators:
    /// modifiers (`cmd`, `alt`, `ctrl`, `shift`) and a key joined by `+`.
    pub hotkey: String,
    /// The most files a Spotlight search shows.
    pub file_limit: usize,
    /// Keywords followed by Space that enter macOS Dictionary search.
    pub dictionary_keywords: Vec<String>,
    /// Web search keywords and their URL templates. `{query}` is replaced when selected.
    pub web_searches: BTreeMap<String, WebSearch>,
    /// Web search keywords offered when no regular results match.
    pub web_fallbacks: Vec<String>,
    /// Where the panel opens on the display under the pointer.
    pub position: Position,
    /// The keyboard input source to type with while the panel shows.
    pub input_source: InputSource,
    /// Whether a signed release build installs its channel's updates by itself. Builds
    /// without an update feed (Nix, `cargo run`) never update.
    pub auto_update: bool,
    /// The shader that animates behind the query field.
    pub backdrop: Backdrop,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearch {
    pub name: String,
    pub url: String,
}

/// What animates behind the query field. In config.toml: `"flow"`, `"off"`, or the path to a
/// WGSL file (`~/` is the home directory; a relative path is relative to config.toml).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
pub enum Backdrop {
    /// The built-in shader: the accent color flowing.
    Flow,
    /// Nothing; the panel draws no frames of its own while it rests.
    #[default]
    Off,
    /// A user's WGSL file.
    File(String),
}

impl From<String> for Backdrop {
    fn from(value: String) -> Self {
        match value.as_str() {
            "flow" => Backdrop::Flow,
            "off" => Backdrop::Off,
            _ => Backdrop::File(value),
        }
    }
}

impl From<Backdrop> for String {
    fn from(value: Backdrop) -> Self {
        match value {
            Backdrop::Flow => "flow".into(),
            Backdrop::Off => "off".into(),
            Backdrop::File(path) => path,
        }
    }
}

/// The keyboard input source the panel switches to when it shows, and switches back from when
/// it hides, as Alfred's "Force Keyboard" does. In config.toml: `"ascii"`, `"keep"`, or an
/// input source identifier such as `"com.apple.keylayout.ABC"`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
pub enum InputSource {
    /// The most recently used source that types ASCII, so a Japanese input method doesn't hold
    /// the query in composition.
    #[default]
    Ascii,
    /// Leave the input source alone.
    Keep,
    /// A specific source.
    Id(String),
}

impl From<String> for InputSource {
    fn from(value: String) -> Self {
        match value.as_str() {
            "ascii" => InputSource::Ascii,
            "keep" => InputSource::Keep,
            _ => InputSource::Id(value),
        }
    }
}

impl From<InputSource> for String {
    fn from(value: InputSource) -> Self {
        match value {
            InputSource::Ascii => "ascii".into(),
            InputSource::Keep => "keep".into(),
            InputSource::Id(id) => id,
        }
    }
}

/// Where the panel opens. Results always grow downwards from the field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    /// The field at the display's vertical center.
    #[default]
    Center,
    /// The field near the top, about where Spotlight puts it.
    Top,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "cmd+space".into(),
            file_limit: 200,
            dictionary_keywords: vec!["define".into(), "d".into()],
            web_searches: BTreeMap::from([
                (
                    "g".into(),
                    WebSearch {
                        name: "Google".into(),
                        url: "https://www.google.com/search?q={query}".into(),
                    },
                ),
                (
                    "gh".into(),
                    WebSearch {
                        name: "GitHub".into(),
                        url: "https://github.com/search?q={query}".into(),
                    },
                ),
                (
                    "yt".into(),
                    WebSearch {
                        name: "YouTube".into(),
                        url: "https://www.youtube.com/results?search_query={query}".into(),
                    },
                ),
            ]),
            web_fallbacks: Vec::new(),
            position: Position::Center,
            input_source: InputSource::Ascii,
            auto_update: true,
            backdrop: Backdrop::Off,
        }
    }
}

impl Config {
    pub fn parse(src: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(src)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_command_space() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::default().hotkey, "cmd+space");
    }

    #[test]
    fn reads_the_hotkey() {
        assert_eq!(
            Config::parse("hotkey = \"alt+space\"").unwrap().hotkey,
            "alt+space"
        );
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(Config::parse("hotkye = \"alt+space\"").is_err());
    }

    #[test]
    fn reads_the_position() {
        assert_eq!(Config::default().position, Position::Center);
        assert_eq!(
            Config::parse("position = \"top\"").unwrap().position,
            Position::Top
        );
        assert!(Config::parse("position = \"middle\"").is_err());
    }

    #[test]
    fn dictionary_keywords_default_and_can_be_configured() {
        assert_eq!(Config::parse("").unwrap().dictionary_keywords, ["define", "d"]);
        let config = Config::parse("dictionary_keywords = [\"word\", \"lookup\"]").unwrap();
        assert_eq!(config.dictionary_keywords, ["word", "lookup"]);
    }

    #[test]
    fn web_search_defaults_and_custom_entries_round_trip() {
        let defaults = Config::default();
        assert_eq!(defaults.web_searches["g"].name, "Google");
        assert_eq!(
            defaults.web_searches["g"].url,
            "https://www.google.com/search?q={query}"
        );
        assert_eq!(defaults.web_searches["gh"].name, "GitHub");
        assert_eq!(defaults.web_searches["yt"].name, "YouTube");
        assert!(defaults.web_fallbacks.is_empty());

        let config = Config::parse(
            r#"
web_searches."docs".name = "Docs"
web_searches."docs".url = "https://docs.example/search?q={query}"
web_fallbacks = ["docs", "g"]
"#,
        )
        .unwrap();
        assert_eq!(config.web_searches["docs"].name, "Docs");
        assert_eq!(config.web_fallbacks, ["docs", "g"]);

        assert!(
            Config::parse(
                r#"
[web_searches.docs]
name = "Docs"
url = "https://docs.example/search?q={query}"
unexpected = true
"#
            )
            .is_err()
        );

        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);
    }

    #[test]
    fn updates_by_default() {
        assert!(Config::default().auto_update);
        assert!(!Config::parse("auto_update = false").unwrap().auto_update);
    }

    #[test]
    fn reads_the_input_source() {
        assert_eq!(Config::default().input_source, InputSource::Ascii);
        let read = |v: &str| {
            Config::parse(&format!("input_source = \"{v}\""))
                .unwrap()
                .input_source
        };
        assert_eq!(read("keep"), InputSource::Keep);
        assert_eq!(
            read("com.apple.keylayout.ABC"),
            InputSource::Id("com.apple.keylayout.ABC".into())
        );
    }

    #[test]
    fn reads_the_backdrop() {
        assert_eq!(Config::default().backdrop, Backdrop::Off);
        let read = |v: &str| Config::parse(&format!("backdrop = \"{v}\"")).unwrap().backdrop;
        assert_eq!(read("off"), Backdrop::Off);
        assert_eq!(read("flow"), Backdrop::Flow);
        assert_eq!(
            read("~/shaders/waves.wgsl"),
            Backdrop::File("~/shaders/waves.wgsl".into())
        );
    }
}
