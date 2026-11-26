use serde::{Deserialize, Serialize};

/// `[sync]`: this Mac's sync settings. The whole table is device-local and never syncs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Sync {
    /// The folder to sync through, such as one in iCloud Drive. Set here, as Home Manager
    /// does, it wins over the folder chosen with Turn On Sync; empty leaves the choice to it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub folder: String,
    /// Accept a folder on NFS, which is refused by default because its rename may not be
    /// atomic.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub allow_nfs: bool,
    /// Whether this Mac syncs each kind of data while sync is on. Turning one off leaves it:
    /// this Mac stops sending and receiving it, and keeps what it has.
    #[serde(skip_serializing_if = "is_true")]
    pub usage: bool,
    #[serde(skip_serializing_if = "is_true")]
    pub snippets: bool,
    #[serde(skip_serializing_if = "is_true")]
    pub settings: bool,
}

fn is_true(value: &bool) -> bool {
    *value
}

impl Default for Sync {
    fn default() -> Self {
        Self {
            folder: String::new(),
            allow_nfs: false,
            usage: true,
            snippets: true,
            settings: true,
        }
    }
}

impl Sync {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether this Mac syncs the built-in namespace `name`, by its switch.
    pub fn includes(&self, name: &str) -> bool {
        match name {
            "usage" => self.usage,
            "snippets" => self.snippets,
            "settings" => self.settings,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn sync_is_off_and_unwritten_by_default() {
        let config = Config::default();
        assert!(config.sync.is_empty());
        assert!(!toml::to_string(&config).unwrap().contains("[sync]"));
        let set = Config::parse("[sync]\nfolder = \"/Users/me/Sync\"").unwrap();
        assert_eq!(set.sync.folder, "/Users/me/Sync");
        assert!(Config::parse("[sync]\nunknown = 1").is_err());
    }

    #[test]
    fn every_namespace_syncs_until_its_switch_is_off() {
        let config = Config::default();
        assert!(
            ["usage", "snippets", "settings"]
                .iter()
                .all(|name| config.sync.includes(name))
        );
        assert!(!config.sync.includes("snapshots"));
        let off = Config::parse("[sync]\nsnippets = false").unwrap();
        assert!(!off.sync.includes("snippets"));
        assert!(off.sync.includes("usage"));
        assert!(toml::to_string(&off).unwrap().contains("snippets = false"));
    }
}
