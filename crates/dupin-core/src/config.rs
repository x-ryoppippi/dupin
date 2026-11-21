//! `config.toml`: user settings. Every key is optional; a missing file is the defaults.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The global shortcut that shows and hides the panel, as Rune parses accelerators:
    /// modifiers (`cmd`, `alt`, `ctrl`, `shift`) and a key joined by `+`.
    pub hotkey: String,
    /// The most files a Spotlight search shows.
    pub file_limit: usize,
    /// Where the panel opens on the display under the pointer.
    pub position: Position,
    /// The keyboard input source to type with while the panel shows.
    pub input_source: InputSource,
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
            position: Position::Center,
            input_source: InputSource::Ascii,
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
}
