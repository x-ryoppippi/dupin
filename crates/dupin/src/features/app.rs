use dupin_core::{
    Action, AppCommand, CommandError, CommandSpec, Config, Icon, Item, ItemAction, Provider, ProviderError,
    Request, View, fuzzy,
};

use super::Feature;
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const APP: &str = "app.commands";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    ..Feature::EMPTY
};

fn configure(_: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(APP);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        2,
        presentation("Search Dupin commands", None, None, None, false, 0.0),
    ))?;
    commands.defaults.push(command);
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(
        APP,
        std::sync::Arc::new(AppProvider::new(crate::update::offered())),
    );
}

/// Dupin's commands on itself. Whether this build updates itself is fixed for its lifetime.
pub(crate) struct AppProvider {
    updates: bool,
}

impl AppProvider {
    pub(crate) fn new(updates: bool) -> Self {
        Self { updates }
    }
}

fn labels(command: AppCommand) -> (&'static str, &'static str, &'static str) {
    match command {
        AppCommand::Relaunch => ("Restart Dupin", "Relaunch Dupin", "arrow.clockwise.circle"),
        AppCommand::Update => (
            "Update Dupin",
            "Check for updates, or restart into one",
            "arrow.down.circle",
        ),
    }
}

impl Provider for AppProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let query = request.query.trim();
        if request.is_cancelled() || request.limit == 0 || query.chars().count() < 2 {
            return Ok(());
        }
        let mut items: Vec<_> = [AppCommand::Relaunch, AppCommand::Update]
            .into_iter()
            .filter(|command| self.updates || *command != AppCommand::Update)
            .filter_map(|command| {
                let (title, subtitle, symbol) = labels(command);
                let found = fuzzy::score(query, title)?;
                let action = ItemAction::new("run", title, Action::App(command));
                Some(Item {
                    payload: Default::default(),
                    id: title.into(),
                    view: View::Text {
                        title: title.into(),
                        subtitle: subtitle.into(),
                    },
                    record_usage: false,
                    icon: Icon::symbol(symbol),
                    matched: found.positions,
                    completion: None,
                    score: Some(found.score),
                    primary: Some(action.id.clone()),
                    alternate: None,
                    actions: vec![action],
                })
            })
            .collect();
        items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        items.truncate(request.limit);
        if !request.is_cancelled() && !items.is_empty() {
            emit(items);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(updates: bool, query: &str) -> Vec<Item> {
        let mut results = Vec::new();
        AppProvider::new(updates)
            .search(&Request::detached(query, 8), &mut |items| results.extend(items))
            .unwrap();
        results
    }

    #[test]
    fn restart_is_always_offered_and_update_only_where_the_build_updates() {
        for (updates, expected) in [(true, vec!["Update Dupin"]), (false, vec![])] {
            let rows = found(updates, "update dupin");
            let titles: Vec<_> = rows.iter().map(Item::display_title).collect();
            assert_eq!(titles, expected);
        }
        for updates in [true, false] {
            let rows = found(updates, "restart dupin");
            assert_eq!(rows[0].display_title(), "Restart Dupin");
            assert_eq!(rows[0].action("run"), Some(&Action::App(AppCommand::Relaunch)));
            assert_eq!(rows[0].icon, Icon::symbol("arrow.clockwise.circle"));
        }
        let update = &found(true, "update dupin")[0];
        assert_eq!(update.action("run"), Some(&Action::App(AppCommand::Update)));
        assert_eq!(update.icon, Icon::symbol("arrow.down.circle"));
    }

    #[test]
    fn short_queries_stay_quiet_and_every_icon_is_a_symbol_the_system_draws() {
        assert!(found(true, "u").is_empty());
        for command in [AppCommand::Relaunch, AppCommand::Update] {
            let (_, _, symbol) = labels(command);
            assert!(
                dupin_mac::symbol::rgba(symbol, 32, 20, [0, 0, 0, 255]).is_some(),
                "{symbol}"
            );
        }
    }
}
