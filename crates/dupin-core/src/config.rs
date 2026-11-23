//! `config.toml`: user settings. Every key is optional; a missing file is the defaults.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MAPS_SEARCH_TEMPLATE: &str = "maps://?q={query}";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The global shortcut that shows and hides the panel, as Rune parses accelerators:
    /// modifiers (`cmd`, `alt`, `ctrl`, `shift`) and a key joined by `+`.
    pub hotkey: String,
    /// Optional shortcut to search text selected in another app.
    pub selection_hotkey: String,
    pub bindings: BTreeMap<String, HotkeyTarget>,
    pub show_recent: bool,
    /// The most files a Spotlight search shows.
    pub file_limit: usize,
    /// Directories to rank lower in file search. Relative names match directory components;
    /// paths beginning with `~/` are expanded by the platform layer.
    pub file_demotions: Vec<String>,
    /// Keywords followed by Space that enter macOS Dictionary search.
    pub dictionary_keywords: Vec<String>,
    pub window_management: bool,
    pub calculator: bool,
    pub calculator_currency: bool,
    /// Web search keywords and their URL templates. `{query}` is replaced when selected.
    pub web_searches: BTreeMap<String, WebSearch>,
    /// Web search keywords offered when no regular results match.
    pub web_fallbacks: Vec<String>,
    /// Text snippets by keyword. Placeholders in the text are filled in when one is used.
    pub snippets: BTreeMap<String, Snippet>,
    /// Keywords followed by Space that enter snippet search.
    pub snippet_keywords: Vec<String>,
    /// Whether Enter on a snippet also pastes it into the frontmost app, which needs
    /// Accessibility access. Off, it only copies.
    pub snippet_paste: bool,
    pub display: DisplaySelection,
    /// Where the panel opens on the selected display.
    pub position: Position,
    /// The keyboard input source to type with while the panel shows.
    pub input_source: InputSource,
    /// Whether a signed release build installs its channel's updates by itself. Builds
    /// without an update feed (Nix, `cargo run`) never update.
    pub auto_update: bool,
    /// The shader that animates behind the query field.
    pub backdrop: Backdrop,
    pub theme: Theme,
    pub calendar: Calendar,
    pub clipboard: Clipboard,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Calendar {
    pub enabled: bool,
}

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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearch {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snippet {
    /// Shown in results instead of the keyword.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub text: String,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplaySelection {
    #[default]
    Mouse,
    Main,
    Active,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "cmd+space".into(),
            selection_hotkey: String::new(),
            bindings: BTreeMap::new(),
            show_recent: true,
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
            display: DisplaySelection::default(),
            window_management: false,
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
                    "maps".into(),
                    WebSearch {
                        name: "Apple Maps".into(),
                        url: MAPS_SEARCH_TEMPLATE.into(),
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
            snippets: BTreeMap::new(),
            snippet_keywords: vec!["snip".into()],
            snippet_paste: false,
            position: Position::Center,
            input_source: InputSource::Ascii,
            auto_update: true,
            backdrop: Backdrop::Off,
            theme: Theme::default(),
            calendar: Calendar::default(),
            clipboard: Clipboard::default(),
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
    fn reads_the_display_selection() {
        assert_eq!(Config::default().display, DisplaySelection::Mouse);
        for (name, expected) in [
            ("mouse", DisplaySelection::Mouse),
            ("main", DisplaySelection::Main),
            ("active", DisplaySelection::Active),
        ] {
            let config = Config::parse(&format!("display = \"{name}\"")).unwrap();
            assert_eq!(config.display, expected);
            assert_eq!(serde_json::to_value(&config).unwrap()["display"], name);
        }
        assert!(Config::parse("display = \"other\"").is_err());
    }

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
        assert_eq!(config.web_searches.len(), 1);
        assert!(
            Config::parse("web_searches = {}")
                .unwrap()
                .web_searches
                .is_empty()
        );
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
    fn snippets_default_to_none_and_round_trip() {
        let defaults = Config::default();
        assert!(defaults.snippets.is_empty());
        assert_eq!(defaults.snippet_keywords, ["snip"]);
        assert!(!defaults.snippet_paste);

        let config = Config::parse(
            r#"
snippet_keywords = ["s"]
snippet_paste = true

[snippets.sig]
name = "Signature"
text = "Best wishes,\nDupin"

[snippets.today]
text = "{date}"
"#,
        )
        .unwrap();
        assert_eq!(config.snippet_keywords, ["s"]);
        assert!(config.snippet_paste);
        assert_eq!(config.snippets["sig"].name.as_deref(), Some("Signature"));
        assert_eq!(config.snippets["sig"].text, "Best wishes,\nDupin");
        assert_eq!(config.snippets["today"].name, None);
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);

        assert!(Config::parse("[snippets.sig]\nname = \"Signature\"").is_err());
        assert!(Config::parse("[snippets.sig]\ntext = \"x\"\nkeyword = \"y\"").is_err());
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

/// The clipboard history kept in memory while Dupin runs. Off by default: it records what
/// you copy, which can be private.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, try_from = "ClipboardValues")]
pub struct Clipboard {
    pub enabled: bool,
    pub max_entries: usize,
    /// Larger copies are skipped rather than truncated, so a copied-back entry is never partial.
    pub(crate) max_entry_bytes: usize,
    /// Bundle identifiers of apps whose copies are never recorded.
    pub(crate) ignored_apps: Vec<String>,
}

const CLIPBOARD_ENTRIES: std::ops::RangeInclusive<usize> = 1..=1000;
const CLIPBOARD_ENTRY_BYTES: std::ops::RangeInclusive<usize> = 1024..=1024 * 1024;

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: 200,
            max_entry_bytes: 64 * 1024,
            ignored_apps: [
                "com.apple.keychainaccess",
                "com.apple.Passwords",
                "com.1password.1password",
                "com.agilebits.onepassword7",
                "com.bitwarden.desktop",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ClipboardValues {
    enabled: bool,
    max_entries: usize,
    max_entry_bytes: usize,
    ignored_apps: Vec<String>,
}

impl Default for ClipboardValues {
    fn default() -> Self {
        let Clipboard {
            enabled,
            max_entries,
            max_entry_bytes,
            ignored_apps,
        } = Clipboard::default();
        Self {
            enabled,
            max_entries,
            max_entry_bytes,
            ignored_apps,
        }
    }
}

impl TryFrom<ClipboardValues> for Clipboard {
    type Error = String;
    fn try_from(v: ClipboardValues) -> Result<Self, String> {
        if !CLIPBOARD_ENTRIES.contains(&v.max_entries) || !CLIPBOARD_ENTRY_BYTES.contains(&v.max_entry_bytes)
        {
            return Err(format!(
                "clipboard limits out of range: max_entries {}..={}, max_entry_bytes {}..={}",
                CLIPBOARD_ENTRIES.start(),
                CLIPBOARD_ENTRIES.end(),
                CLIPBOARD_ENTRY_BYTES.start(),
                CLIPBOARD_ENTRY_BYTES.end(),
            ));
        }
        Ok(Self {
            enabled: v.enabled,
            max_entries: v.max_entries,
            max_entry_bytes: v.max_entry_bytes,
            ignored_apps: v.ignored_apps,
        })
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;

    #[test]
    fn clipboard_history_is_off_by_default_and_round_trips() {
        let defaults = Config::parse("").unwrap().clipboard;
        assert!(!defaults.enabled);
        assert_eq!(defaults, Clipboard::default());
        assert!(
            defaults
                .ignored_apps
                .iter()
                .any(|id| id == "com.apple.keychainaccess")
        );

        let config = Config::parse(
            "[clipboard]\nenabled = true\nmax_entries = 50\nmax_entry_bytes = 4096\nignored_apps = [\"com.example.vault\"]",
        )
        .unwrap();
        assert!(config.clipboard.enabled);
        assert_eq!(config.clipboard.max_entries, 50);
        assert_eq!(config.clipboard.max_entry_bytes, 4096);
        assert_eq!(config.clipboard.ignored_apps, ["com.example.vault"]);
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);
    }

    #[test]
    fn clipboard_limits_and_unknown_keys_are_rejected() {
        for field in [
            "max_entries = 0",
            "max_entries = 1001",
            "max_entry_bytes = 1023",
            "max_entry_bytes = 1048577",
            "max_entries = -1",
            "history = true",
        ] {
            assert!(
                Config::parse(&format!("[clipboard]\n{field}")).is_err(),
                "{field}"
            );
        }
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
