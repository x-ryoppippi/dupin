use serde::{Deserialize, Serialize};

/// `[sync]`: this Mac's sync settings. The whole table is device-local and never syncs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
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
}

impl Sync {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
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
}
