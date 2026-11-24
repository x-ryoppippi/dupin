use serde::{Deserialize, Serialize};

/// The clipboard history. Off by default: it records what you copy, which can be private.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, try_from = "ClipboardValues")]
pub struct Clipboard {
    pub enabled: bool,
    pub max_entries: usize,
    /// Larger copies are skipped rather than truncated, so a copied-back entry is never partial.
    pub(crate) max_entry_bytes: usize,
    /// How long an entry is kept after it was copied.
    pub retention: Retention,
    pub include_images: bool,
    pub include_files: bool,
    /// The largest image kept, after converting it to PNG.
    pub(crate) max_image_bytes: usize,
    /// Bundle identifiers of apps whose copies are never recorded.
    pub(crate) ignored_apps: Vec<String>,
}

const CLIPBOARD_ENTRIES: std::ops::RangeInclusive<usize> = 1..=10_000;
const CLIPBOARD_ENTRY_BYTES: std::ops::RangeInclusive<usize> = 1024..=1024 * 1024;
const CLIPBOARD_IMAGE_BYTES: std::ops::RangeInclusive<usize> = 64 * 1024..=32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Retention {
    #[serde(rename = "1d")]
    Day,
    #[default]
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "90d")]
    Quarter,
    #[serde(rename = "forever")]
    Forever,
}

impl Retention {
    pub const ALL: [Retention; 5] = [
        Retention::Day,
        Retention::Week,
        Retention::Month,
        Retention::Quarter,
        Retention::Forever,
    ];

    /// How long an entry lives, in seconds; `None` keeps it until it is deleted or pushed out.
    pub(crate) fn seconds(self) -> Option<u64> {
        let days = match self {
            Retention::Day => 1,
            Retention::Week => 7,
            Retention::Month => 30,
            Retention::Quarter => 90,
            Retention::Forever => return None,
        };
        Some(days * 86_400)
    }

    pub fn label(self) -> &'static str {
        match self {
            Retention::Day => "1 day",
            Retention::Week => "7 days",
            Retention::Month => "30 days",
            Retention::Quarter => "90 days",
            Retention::Forever => "Forever",
        }
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: 200,
            max_entry_bytes: 64 * 1024,
            retention: Retention::default(),
            include_images: true,
            include_files: true,
            max_image_bytes: 4 * 1024 * 1024,
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
    retention: Retention,
    include_images: bool,
    include_files: bool,
    max_image_bytes: usize,
    ignored_apps: Vec<String>,
}

impl Default for ClipboardValues {
    fn default() -> Self {
        let Clipboard {
            enabled,
            max_entries,
            max_entry_bytes,
            retention,
            include_images,
            include_files,
            max_image_bytes,
            ignored_apps,
        } = Clipboard::default();
        Self {
            enabled,
            max_entries,
            max_entry_bytes,
            retention,
            include_images,
            include_files,
            max_image_bytes,
            ignored_apps,
        }
    }
}

impl TryFrom<ClipboardValues> for Clipboard {
    type Error = String;
    fn try_from(v: ClipboardValues) -> Result<Self, String> {
        if !CLIPBOARD_ENTRIES.contains(&v.max_entries)
            || !CLIPBOARD_ENTRY_BYTES.contains(&v.max_entry_bytes)
            || !CLIPBOARD_IMAGE_BYTES.contains(&v.max_image_bytes)
        {
            return Err(format!(
                "clipboard limits out of range: max_entries {}..={}, max_entry_bytes {}..={}, max_image_bytes {}..={}",
                CLIPBOARD_ENTRIES.start(),
                CLIPBOARD_ENTRIES.end(),
                CLIPBOARD_ENTRY_BYTES.start(),
                CLIPBOARD_ENTRY_BYTES.end(),
                CLIPBOARD_IMAGE_BYTES.start(),
                CLIPBOARD_IMAGE_BYTES.end(),
            ));
        }
        Ok(Self {
            enabled: v.enabled,
            max_entries: v.max_entries,
            max_entry_bytes: v.max_entry_bytes,
            retention: v.retention,
            include_images: v.include_images,
            include_files: v.include_files,
            max_image_bytes: v.max_image_bytes,
            ignored_apps: v.ignored_apps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Clipboard, Retention};
    use crate::config::Config;

    #[test]
    fn clipboard_history_is_off_by_default_and_round_trips() {
        let defaults = Config::parse("").unwrap().clipboard;
        assert!(!defaults.enabled);
        assert_eq!(defaults, Clipboard::default());
        assert_eq!(defaults.retention, Retention::Week);
        assert!(defaults.include_images && defaults.include_files);
        assert!(
            defaults
                .ignored_apps
                .iter()
                .any(|id| id == "com.apple.keychainaccess")
        );

        let config = Config::parse(
            "[clipboard]\nenabled = true\nmax_entries = 50\nmax_entry_bytes = 4096\nretention = \"forever\"\ninclude_images = false\ninclude_files = false\nmax_image_bytes = 65536\nignored_apps = [\"com.example.vault\"]",
        )
        .unwrap();
        assert!(config.clipboard.enabled);
        assert_eq!(config.clipboard.max_entries, 50);
        assert_eq!(config.clipboard.max_entry_bytes, 4096);
        assert_eq!(config.clipboard.retention, Retention::Forever);
        assert!(!config.clipboard.include_images && !config.clipboard.include_files);
        assert_eq!(config.clipboard.max_image_bytes, 65536);
        assert_eq!(config.clipboard.ignored_apps, ["com.example.vault"]);
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);
    }

    #[test]
    fn retention_reads_every_documented_period() {
        for (value, retention, seconds) in [
            ("1d", Retention::Day, Some(86_400)),
            ("7d", Retention::Week, Some(7 * 86_400)),
            ("30d", Retention::Month, Some(30 * 86_400)),
            ("90d", Retention::Quarter, Some(90 * 86_400)),
            ("forever", Retention::Forever, None),
        ] {
            let config = Config::parse(&format!("[clipboard]\nretention = \"{value}\"")).unwrap();
            assert_eq!(config.clipboard.retention, retention);
            assert_eq!(retention.seconds(), seconds);
        }
        assert_eq!(Retention::ALL.len(), 5);
    }

    #[test]
    fn clipboard_limits_and_unknown_keys_are_rejected() {
        for field in [
            "max_entries = 0",
            "max_entries = 10001",
            "max_entry_bytes = 1023",
            "max_entry_bytes = 1048577",
            "max_entries = -1",
            "max_image_bytes = 65535",
            "max_image_bytes = 33554433",
            "retention = \"2d\"",
            "retention = 7",
            "history = true",
        ] {
            assert!(
                Config::parse(&format!("[clipboard]\n{field}")).is_err(),
                "{field}"
            );
        }
    }
}
