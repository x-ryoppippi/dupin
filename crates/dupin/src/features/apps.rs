use dupin_core::{CommandError, CommandSpec, Config};

use super::{Feature, Source};
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const APPS: &str = "apps.search";
/// The installed apps, which the main search matches on every key.
pub(crate) const SOURCE: &str = "apps";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    sources: &[Source {
        name: SOURCE,
        command: APPS,
        matcher: Some(dupin_core::filter::FUZZY),
        rows: Some(dupin_broker::RowKind::App),
    }],
    ..Feature::EMPTY
};

fn configure(_: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(APPS);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        50,
        presentation("Search apps", None, None, Some("No matches"), false, 0.0),
    ))?;
    commands.defaults.push(command);
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add_source(SOURCE, providers.apps.clone());
}
