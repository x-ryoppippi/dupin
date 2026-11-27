use dupin_core::config::DisplaySelection;
use dupin_mac::privacy::Permission;

use super::{Capability, Feature, PermissionUse};

pub(super) const FEATURE: Feature = Feature {
    permissions: &[
        PermissionUse {
            capability: Capability::Permission(Permission::Accessibility),
            label: "focused window display",
            enabled: |config| config.display == DisplaySelection::Active,
            request_on_enable: false,
        },
        PermissionUse {
            capability: Capability::Permission(Permission::Accessibility),
            label: "selected-text search",
            enabled: |config| !config.selection_hotkey.trim().is_empty(),
            request_on_enable: true,
        },
    ],
    ..Feature::EMPTY
};
