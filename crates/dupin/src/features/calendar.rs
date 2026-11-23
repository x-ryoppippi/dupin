use dupin_core::{CommandError, CommandSpec, Config, QueryEligibility};

use super::Feature;
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const CALENDAR: &str = "calendar.events";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    if config.calendar.enabled {
        let calendar = id(CALENDAR);
        let mut spec = CommandSpec::new(
            calendar.clone(),
            30,
            presentation(
                "Search calendar events",
                Some("Calendar"),
                Some("No upcoming events"),
                Some("No matching events"),
                true,
                5.0,
            ),
        )
        .with_query_eligibility(QueryEligibility::Any);
        spec.private_results = true;
        commands.registry.register(spec)?;
        commands.registry.add_keyword(&calendar, "cal")?;
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(CALENDAR, providers.calendar.clone());
}
