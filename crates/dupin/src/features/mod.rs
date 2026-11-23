pub(super) mod files;
pub(super) mod web;

use dupin_core::{CommandError, Config};

use crate::commands::{self, Configure, ConfiguredCommands, Providers};
use crate::ui::Ui;

pub(super) struct Feature {
    pub configure: fn(&Config, &mut Configure) -> Result<(), CommandError>,
    pub prepare: fn(&Config, &mut Configure),
    pub start: fn(&mut Providers),
    pub reload: fn(&mut Ui, &Config, &mut ConfiguredCommands) -> bool,
}

impl Feature {
    pub(super) const EMPTY: Self = Self {
        configure: |_, _| Ok(()),
        prepare: |_, _| {},
        start: |_| {},
        reload: |_, _, _| false,
    };
}

pub(super) const MANIFEST: &[Feature] = &[
    files::FEATURE,
    commands::LEGACY,
    web::FEATURE,
    commands::LEGACY_TAIL,
];

pub(super) fn reload(ui: &mut Ui, config: &Config, commands: &mut ConfiguredCommands) -> bool {
    let mut refresh = false;
    for feature in MANIFEST {
        refresh |= (feature.reload)(ui, config, commands);
    }
    refresh
}

fn default_query(ui: &Ui) -> bool {
    ui.session.command_id().as_str() == commands::APPS && !ui.session.query().trim().is_empty()
}
