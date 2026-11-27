use serde::{Deserialize, Serialize};

use crate::item::{SYMBOL_PREFIX, is_symbol_name};

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, try_from = "ThemeValues")]
pub struct Theme {
    pub panel_width: u16,
    pub row_height: u16,
    pub corner_radius: u16,
    pub font_size: u16,
    pub font_family: FontFamily,
    pub search_icon: SearchIcon,
    pub light: Colors,
    pub dark: Colors,
}

/// The family text draws in. In config.toml: `"system"`, `"monospace"`, or an installed
/// family's name (`"JetBrains Mono"`); a comma-separated CSS family list tries each in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
pub enum FontFamily {
    /// The system UI font (San Francisco).
    #[default]
    System,
    /// The platform's monospaced UI font (SF Mono).
    Monospace,
    /// A font installed on the system, or a CSS family list. Falls back to the system font
    /// when no named family is installed.
    Named(String),
}

impl From<String> for FontFamily {
    fn from(value: String) -> Self {
        match value.as_str() {
            "system" => FontFamily::System,
            "monospace" => FontFamily::Monospace,
            _ => FontFamily::Named(value),
        }
    }
}

impl From<FontFamily> for String {
    fn from(value: FontFamily) -> Self {
        match value {
            FontFamily::System => "system".into(),
            FontFamily::Monospace => "monospace".into(),
            FontFamily::Named(name) => name,
        }
    }
}

/// What stands before the query field. In config.toml: `"sf:"` and an SF Symbol's name
/// (`"sf:magnifyingglass"`, the default), or a few characters of text (`"⌕"`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
pub enum SearchIcon {
    /// An SF Symbol, drawn in the secondary text colour.
    Symbol(String),
    /// Text in the theme's font, such as a glyph or an emoji.
    Text(String),
}

/// The SF Symbol a theme without a search icon shows, and one whose symbol the system lacks.
pub const DEFAULT_SEARCH_SYMBOL: &str = "magnifyingglass";

/// The most characters a text search icon holds: room for a joined emoji, not for a word.
const MAX_SEARCH_ICON_TEXT: usize = 8;

impl Default for SearchIcon {
    fn default() -> Self {
        SearchIcon::Symbol(DEFAULT_SEARCH_SYMBOL.into())
    }
}

impl From<String> for SearchIcon {
    fn from(value: String) -> Self {
        match value.strip_prefix(SYMBOL_PREFIX) {
            Some(name) => SearchIcon::Symbol(name.into()),
            None => SearchIcon::Text(value),
        }
    }
}

impl From<SearchIcon> for String {
    fn from(value: SearchIcon) -> Self {
        match value {
            SearchIcon::Symbol(name) => format!("{SYMBOL_PREFIX}{name}"),
            SearchIcon::Text(text) => text,
        }
    }
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
    search_icon: SearchIcon,
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
            search_icon: SearchIcon::default(),
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
            search_icon: t.search_icon,
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
        if let FontFamily::Named(name) = &v.font_family
            && name.trim().is_empty()
        {
            return Err("theme font_family must not be empty".into());
        }
        let search_icon_fits = match &v.search_icon {
            SearchIcon::Symbol(name) => is_symbol_name(name),
            SearchIcon::Text(text) => {
                !text.trim().is_empty()
                    && text.chars().count() <= MAX_SEARCH_ICON_TEXT
                    && !text.chars().any(char::is_control)
            }
        };
        if !search_icon_fits {
            return Err(format!(
                "theme search_icon must be \"{SYMBOL_PREFIX}\" and an SF Symbol's name, or 1..{MAX_SEARCH_ICON_TEXT} characters of text"
            ));
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
            search_icon: v.search_icon,
            light: v.light,
            dark: v.dark,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Backdrop, Theme};
    use crate::config::Config;

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
            "font_family = \"\"",
            "font_family = \"   \"",
            "light.accent = \"#GG0000\"",
            "dark.text = \"red\"",
        ] {
            assert!(Config::parse(&format!("[theme]\n{field}")).is_err(), "{field}");
        }
        assert_eq!(Config::parse("").unwrap().theme, Theme::default());
    }

    #[test]
    fn search_icon_is_a_symbol_or_short_text() {
        use super::SearchIcon;
        let read = |v: &str| Config::parse(&format!("[theme]\nsearch_icon = \"{v}\""));
        assert_eq!(
            Theme::default().search_icon,
            SearchIcon::Symbol("magnifyingglass".into())
        );
        assert_eq!(
            read("sf:sparkle").unwrap().theme.search_icon,
            SearchIcon::Symbol("sparkle".into())
        );
        assert_eq!(read("λ").unwrap().theme.search_icon, SearchIcon::Text("λ".into()));
        assert_eq!(
            read("🔎").unwrap().theme.search_icon,
            SearchIcon::Text("🔎".into())
        );
        for value in ["", "   ", "sf:", "sf:moon fill", "sf:../moon", "far too long"] {
            assert!(read(value).is_err(), "{value}");
        }
        let config = read("sf:moon.fill").unwrap();
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
    }

    #[test]
    fn font_family_names_installed_fonts() {
        use super::FontFamily;
        let family = |v: &str| {
            Config::parse(&format!("[theme]\nfont_family = \"{v}\""))
                .unwrap()
                .theme
                .font_family
        };
        assert_eq!(family("system"), FontFamily::System);
        assert_eq!(family("monospace"), FontFamily::Monospace);
        assert_eq!(
            family("JetBrains Mono"),
            FontFamily::Named("JetBrains Mono".into())
        );
        assert_eq!(
            family("Iosevka, JetBrains Mono"),
            FontFamily::Named("Iosevka, JetBrains Mono".into())
        );
        let config = Config::parse("[theme]\nfont_family = \"JetBrains Mono\"").unwrap();
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
    }
}
