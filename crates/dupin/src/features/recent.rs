use std::sync::Arc;

use dupin_core::{CommandError, CommandSpec, Config, QueryEligibility};
use dupin_mac::RecentProvider;

use super::{Feature, Source};
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const RECENT: &str = "recent.items";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    sources: &[Source {
        name: "recent",
        command: RECENT,
        matcher: None,
    }],
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    if config.show_recent {
        let command = id(RECENT);
        commands.registry.register(
            CommandSpec::new(
                command.clone(),
                10,
                presentation("Recent items", None, None, None, false, 0.0),
            )
            .with_query_eligibility(QueryEligibility::EmptyOnly),
        )?;
        commands.defaults.push(command);
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    let recent = RecentProvider::new(
        providers.apps.clone(),
        providers.registration.session.recent_history(),
        providers.kinds.clone(),
    );
    providers.add_source("recent", Arc::new(recent));
}
