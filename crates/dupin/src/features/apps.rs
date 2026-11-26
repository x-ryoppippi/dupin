use std::path::PathBuf;

use dupin_core::{CommandError, CommandSpec, Config};

use super::{Feature, Source};
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const APPS: &str = "apps.search";
/// The installed apps, which the main search matches on every key.
pub(crate) const SOURCE: &str = "apps";

/// The app folders a headless launcher scans in place of the standard ones, as a `PATH`-style
/// list, so a render meant for other people shows none of this Mac's own apps: `dupin ext render`
/// sets it empty, and the documentation images to `/System/Applications`.
pub(crate) const HEADLESS_APPS: &str = "DUPIN_HEADLESS_APPS";

/// The folders `HEADLESS_APPS` names, when this is a headless run and it is set.
pub(crate) fn headless_roots() -> Option<Vec<PathBuf>> {
    if !rune::tooling::is_headless() {
        return None;
    }
    let list = std::env::var_os(HEADLESS_APPS)?;
    Some(
        std::env::split_paths(&list)
            .filter(|root| !root.as_os_str().is_empty())
            .collect(),
    )
}

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
