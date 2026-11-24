//! `config.toml`: user settings. Every key is optional; a missing file is the defaults.

mod calculator;
mod calendar;
mod clipboard;
mod dictionary;
mod extensions;
mod files;
mod hotkeys;
mod panel;
mod snippets;
mod theme;
mod updates;
mod web;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use calendar::Calendar;
pub use clipboard::{Clipboard, Retention};
pub use hotkeys::HotkeyTarget;
pub use panel::{DisplaySelection, InputSource, Position};
pub use snippets::Snippet;
pub use theme::{Backdrop, Colors, FontFamily, Theme};
pub use web::{MAPS_SEARCH_TEMPLATE, WebSearch};

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
    pub system_automation: bool,
    /// The shader that animates behind the query field.
    pub backdrop: Backdrop,
    /// Load extensions from the `extensions` folder beside config.toml. Off by default
    /// because they are third-party code.
    pub extensions: bool,
    pub theme: Theme,
    pub calendar: Calendar,
    pub clipboard: Clipboard,
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
            window_management: false,
            calculator: calculator::DEFAULT_ENABLED,
            calculator_currency: calculator::DEFAULT_CURRENCY,
            web_searches: web::default_searches(),
            web_fallbacks: Vec::new(),
            snippets: BTreeMap::new(),
            snippet_keywords: snippets::default_keywords(),
            snippet_paste: snippets::DEFAULT_PASTE,
            position: Position::default(),
            input_source: InputSource::default(),
            auto_update: updates::DEFAULT_ENABLED,
            system_automation: false,
            backdrop: Backdrop::default(),
            extensions: extensions::DEFAULT_ENABLED,
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
    use super::Config;

    #[test]
    fn rejects_unknown_keys() {
        assert!(Config::parse("hotkye = \"alt+space\"").is_err());
    }
}
