use serde::{Deserialize, Serialize};

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
mod tests {
    use super::Clipboard;
    use crate::config::Config;

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
