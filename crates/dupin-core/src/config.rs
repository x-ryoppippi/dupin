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
    /// Directories to rank lower in file search. Relative names match directory components;
    /// paths beginning with `~/` are expanded by the platform layer.
    pub file_demotions: Vec<String>,
    /// Keywords followed by Space that enter macOS Dictionary search.
    pub dictionary_keywords: Vec<String>,
    pub calculator: bool,
    pub calculator_currency: bool,
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
    pub theme: Theme,
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
            file_demotions: [
                "~/Library",
                "Caches",
                "node_modules",
                ".git",
                "target",
                "build",
                "dist",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            dictionary_keywords: vec!["define".into(), "d".into()],
            calculator: true,
            calculator_currency: false,
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
            theme: Theme::default(),
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
    fn file_demotions_default_and_round_trip() {
        let defaults = Config::default();
        assert_eq!(
            defaults.file_demotions,
            [
                "~/Library",
                "Caches",
                "node_modules",
                ".git",
                "target",
                "build",
                "dist",
            ]
        );

        let config = Config::parse("file_demotions = [\"~/Library\", \"Vendor/Cache\", \"\"]").unwrap();
        assert_eq!(config.file_demotions, ["~/Library", "Vendor/Cache", ""]);
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, try_from = "ThemeValues")]
pub struct Theme {
    pub panel_width: u16,
    pub row_height: u16,
    pub corner_radius: u16,
    pub font_size: u16,
    pub font_family: FontFamily,
    pub light: Colors,
    pub dark: Colors,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FontFamily {
    #[default]
    System,
    Monospace,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    pub background: Option<String>,
    pub text: Option<String>,
    pub secondary: Option<String>,
    pub accent: Option<String>,
    pub selected_text: Option<String>,
    pub hover: Option<String>,
    pub separator: Option<String>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ThemeValues {
    panel_width: u16,
    row_height: u16,
    corner_radius: u16,
    font_size: u16,
    font_family: FontFamily,
    light: Colors,
    dark: Colors,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            panel_width: 680,
            row_height: 52,
            corner_radius: 14,
            font_size: 15,
            font_family: FontFamily::System,
            light: Colors::default(),
            dark: Colors::default(),
        }
    }
}
impl Default for ThemeValues {
    fn default() -> Self {
        let t = Theme::default();
        Self {
            panel_width: t.panel_width,
            row_height: t.row_height,
            corner_radius: t.corner_radius,
            font_size: t.font_size,
            font_family: t.font_family,
            light: t.light,
            dark: t.dark,
        }
    }
}
impl TryFrom<ThemeValues> for Theme {
    type Error = String;
    fn try_from(v: ThemeValues) -> Result<Self, String> {
        if !(400..=1200).contains(&v.panel_width)
            || !(52..=100).contains(&v.row_height)
            || v.corner_radius > 32
            || !(12..=20).contains(&v.font_size)
            || v.row_height < v.font_size * 3 + 7
        {
            return Err("theme geometry out of range: width 400..1200, rows 52..100 and at least 3 * font_size + 7, radius 0..32, font 12..20".into());
        }
        for c in [&v.light, &v.dark] {
            for value in [
                &c.background,
                &c.text,
                &c.secondary,
                &c.accent,
                &c.selected_text,
                &c.hover,
                &c.separator,
            ]
            .into_iter()
            .flatten()
            {
                if value.len() != 7
                    || !value.starts_with('#')
                    || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err("theme colors must be #RRGGBB".into());
                }
            }
        }
        Ok(Self {
            panel_width: v.panel_width,
            row_height: v.row_height,
            corner_radius: v.corner_radius,
            font_size: v.font_size,
            font_family: v.font_family,
            light: v.light,
            dark: v.dark,
        })
    }
}

#[cfg(test)]
mod theme_tests {
    use super::*;
    #[test]
    fn themes_round_trip_and_reject_invalid_geometry_and_colors() {
        let config = Config::parse("[theme]\npanel_width = 400\nrow_height = 100\nfont_size = 20\nfont_family = \"monospace\"\n[theme.dark]\naccent = \"#123456\"").unwrap();
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
        for field in [
            "panel_width = 399",
            "panel_width = 1201",
            "row_height = 51",
            "row_height = 101",
            "font_size = 11",
            "font_size = 21",
            "corner_radius = 33",
            "font_size = 20",
            "panel_width = nan",
            "row_height = inf",
            "corner_radius = -1",
            "font_family = \"unknown\"",
            "light.accent = \"#GG0000\"",
            "dark.text = \"red\"",
        ] {
            assert!(Config::parse(&format!("[theme]\n{field}")).is_err(), "{field}");
        }
        assert_eq!(Config::parse("").unwrap().theme, Theme::default());
    }
}
