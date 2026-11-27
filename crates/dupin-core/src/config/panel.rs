use serde::{Deserialize, Serialize};

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

/// Where the panel opens. Results grow away from the field: downwards from it, or upwards at
/// the bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    /// The field at the display's vertical center.
    #[default]
    Center,
    /// The field near the top, about where Spotlight puts it.
    Top,
    /// The field near the bottom, as far from it as `Top` is from the top, with the results
    /// above it and the best match next to it, as fzf and Telescope lay out.
    Bottom,
}

impl Position {
    /// Whether the panel's lists run upwards from the field.
    pub fn grows_up(self) -> bool {
        self == Position::Bottom
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplaySelection {
    #[default]
    Mouse,
    Main,
    Active,
}

#[cfg(test)]
mod tests {
    use super::{DisplaySelection, InputSource, Position};
    use crate::config::Config;

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
    fn reads_the_position() {
        assert_eq!(Config::default().position, Position::Center);
        assert_eq!(
            Config::parse("position = \"top\"").unwrap().position,
            Position::Top
        );
        assert_eq!(
            Config::parse("position = \"bottom\"").unwrap().position,
            Position::Bottom
        );
        assert!(Position::Bottom.grows_up());
        assert!(!Position::Top.grows_up() && !Position::Center.grows_up());
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
