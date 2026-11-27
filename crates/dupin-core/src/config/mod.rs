//! `config.toml`: user settings. Every key is optional; a missing file is the defaults.

mod advanced;
mod aliases;
mod calculator;
mod calendar;
mod clipboard;
mod dictionary;
mod extensions;
mod files;
mod hotkeys;
mod panel;
mod snippets;
mod sync;
mod telemetry;
mod theme;
mod updates;
mod web;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use advanced::{
    Advanced, KindAction, KindActions, KindConfig, Route, Routes, SourceStages, contribution,
};
use aliases::check_command_name;
pub use aliases::{AliasTarget, Aliases, MAX_ALIASES, check_alias};
pub use calendar::Calendar;
pub use clipboard::{Clipboard, Retention};
pub use hotkeys::{Address, HotkeyTarget};
pub use panel::{DisplaySelection, InputSource, Position, Rows};
pub use snippets::Snippet;
pub use sync::Sync;
pub use theme::{Backdrop, Colors, DEFAULT_SEARCH_ICON, FontFamily, SearchIcon, Theme};
pub use web::WebSearch;

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
    /// Window management is the optional Window Management extension now.
    #[serde(skip_serializing)]
    pub window_management: Retired,
    pub calculator: bool,
    pub calculator_currency: bool,
    /// Web search keywords and their URL templates, which replace the bundled web-search
    /// extension's own when set. `{query}` is replaced when selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_searches: Option<BTreeMap<String, WebSearch>>,
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
    /// How many result rows the panel shows before its list scrolls.
    pub rows: Rows,
    /// The keyboard input source to type with while the panel shows.
    pub input_source: InputSource,
    /// Whether a signed release build installs its channel's updates by itself. Builds
    /// without an update feed (Nix, `cargo run`) never update.
    pub auto_update: bool,
    /// Whether a signed release build sends the anonymous daily usage ping. Builds without the
    /// collector's address (Nix, `cargo run`) never send it.
    pub telemetry: bool,
    pub system_automation: bool,
    /// The shader that animates behind the query field.
    pub backdrop: Backdrop,
    /// Load extensions from the `extensions` folder beside config.toml. Off by default
    /// because they are third-party code.
    pub extensions: bool,
    pub theme: Theme,
    pub calendar: Calendar,
    pub clipboard: Clipboard,
    /// This Mac's sync folder and switches, device-local.
    #[serde(skip_serializing_if = "Sync::is_empty")]
    pub sync: Sync,
    /// Shorthands typed in the main search, by what is typed.
    #[serde(skip_serializing_if = "Aliases::is_empty")]
    pub aliases: Aliases,
    /// Recomposition of the built-in sources, filters and kinds.
    #[serde(skip_serializing_if = "Advanced::is_empty")]
    pub advanced: Advanced,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: hotkeys::DEFAULT_HOTKEY.into(),
            selection_hotkey: String::new(),
            bindings: BTreeMap::new(),
            show_recent: true,
            file_limit: files::DEFAULT_LIMIT,
            file_demotions: files::default_demotions(),
            dictionary_keywords: dictionary::default_keywords(),
            display: DisplaySelection::default(),
            window_management: Retired,
            calculator: calculator::DEFAULT_ENABLED,
            calculator_currency: calculator::DEFAULT_CURRENCY,
            web_searches: None,
            web_fallbacks: Vec::new(),
            snippets: BTreeMap::new(),
            snippet_keywords: snippets::default_keywords(),
            snippet_paste: snippets::DEFAULT_PASTE,
            position: Position::default(),
            rows: Rows::default(),
            input_source: InputSource::default(),
            auto_update: updates::DEFAULT_ENABLED,
            telemetry: telemetry::DEFAULT_ENABLED,
            system_automation: false,
            backdrop: Backdrop::default(),
            extensions: extensions::DEFAULT_ENABLED,
            theme: Theme::default(),
            calendar: Calendar::default(),
            clipboard: Clipboard::default(),
            sync: Sync::default(),
            aliases: Aliases::default(),
            advanced: Advanced::default(),
        }
    }
}

impl Config {
    pub fn parse(src: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(src)
    }
}

/// The top-level keys whose value is the same choice on every Mac, which sync carries
/// (docs/design/sync.md#settings). Any other key is device-local: hotkeys, display choices, keys
/// whose enabling asks for access on this Mac, updates and telemetry, snippets (kept apart from settings) and
/// any key added later, so a new key never syncs by accident.
const SHARED_KEYS: &[&str] = &[
    "show_recent",
    "file_limit",
    "file_demotions",
    "dictionary_keywords",
    "calculator",
    "calculator_currency",
    "web_searches",
    "web_fallbacks",
    "snippet_keywords",
    "theme",
    "aliases",
    "advanced",
];

pub fn is_shared(key: &str) -> bool {
    SHARED_KEYS.contains(&key)
}

/// A key Dupin no longer reads. Settings wrote every key to each config.toml it saved, so a
/// retired one is still accepted, whatever its value. Settings never writes one, and a save
/// leaves one already in the file as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retired;

impl<'de> Deserialize<'de> for Retired {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(deserializer).map(|_| Self)
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn rejects_unknown_keys() {
        assert!(Config::parse("hotkye = \"alt+space\"").is_err());
    }

    #[test]
    fn a_retired_key_loads_and_is_never_written() {
        let config = Config::parse("window_management = true\nshow_recent = false").unwrap();
        assert!(!config.show_recent);
        assert_eq!(config, Config::parse("show_recent = false").unwrap());
        assert!(!toml::to_string(&config).unwrap().contains("window_management"));
    }

    #[test]
    fn every_shared_key_is_a_key_and_the_device_local_ones_stay_local() {
        let written = toml::Table::try_from(Config {
            web_searches: Some(Default::default()),
            aliases: super::Aliases::default(),
            ..Config::default()
        })
        .unwrap();
        for key in super::SHARED_KEYS {
            assert!(
                written.contains_key(*key) || ["aliases", "advanced"].contains(key),
                "{key} isn't a key of config.toml"
            );
        }
        for key in [
            "hotkey",
            "selection_hotkey",
            "bindings",
            "display",
            "position",
            "rows",
            "input_source",
            "backdrop",
            "calendar",
            "system_automation",
            "snippet_paste",
            "extensions",
            "clipboard",
            "auto_update",
            "telemetry",
            "snippets",
            "a key added later",
        ] {
            assert!(!super::is_shared(key), "{key} must stay on this Mac");
        }
    }
}
