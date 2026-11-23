use dupin_core::{Action, Item, ItemAction, Provider, ProviderError, Request, SystemCommand, View, fuzzy};

mod native;
pub use native::run;

pub type Runner = fn(SystemCommand) -> Result<(), String>;

pub const COMMANDS: [SystemCommand; 8] = [
    SystemCommand::Sleep,
    SystemCommand::LockScreen,
    SystemCommand::SleepDisplays,
    SystemCommand::Restart,
    SystemCommand::ShutDown,
    SystemCommand::LogOut,
    SystemCommand::EmptyTrash,
    SystemCommand::ToggleDarkMode,
];

fn labels(command: SystemCommand) -> (&'static str, &'static str, Option<&'static str>) {
    use SystemCommand::*;
    match command {
        Sleep => ("Sleep", "Put this Mac to sleep", None),
        LockScreen => ("Lock Screen", "Lock this session", None),
        SleepDisplays => (
            "Sleep Displays",
            "Turn off displays while the Mac stays awake",
            None,
        ),
        Restart => ("Restart", "Restart this Mac", Some("Restart this Mac?")),
        ShutDown => ("Shut Down", "Turn off this Mac", Some("Shut down this Mac?")),
        LogOut => (
            "Log Out",
            "End this login session",
            Some("Log out of this account?"),
        ),
        EmptyTrash => (
            "Empty Trash",
            "Permanently delete items in the Trash",
            Some("Permanently delete all items in the Trash?"),
        ),
        ToggleDarkMode => ("Toggle Dark Mode", "Switch the macOS appearance", None),
    }
}

fn needs_automation(command: SystemCommand) -> bool {
    matches!(
        command,
        SystemCommand::Restart | SystemCommand::ShutDown | SystemCommand::LogOut | SystemCommand::EmptyTrash
    )
}

pub fn execute(
    command: SystemCommand,
    automation_enabled: bool,
    runner: impl FnOnce(SystemCommand) -> Result<(), String>,
) -> Result<(), String> {
    if needs_automation(command) && !automation_enabled {
        return Err(
            "Enable System Command Automation in Dupin Settings → General to run this command.".into(),
        );
    }
    runner(command).map_err(|error| format!("{} failed. {error}", labels(command).0))
}

pub struct SystemProvider;

impl Provider for SystemProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let query = request.query.trim();
        if request.is_cancelled() || request.limit == 0 || query.chars().count() < 2 {
            return Ok(());
        }
        let mut items: Vec<_> = COMMANDS
            .into_iter()
            .filter_map(|command| {
                let (title, subtitle, prompt) = labels(command);
                let found = fuzzy::score(query, title)?;
                let mut action = ItemAction::new("run", title, Action::System(command));
                if let Some(prompt) = prompt {
                    action = action.confirm(prompt, title);
                }
                Some(Item {
                    id: title.into(),
                    view: View::Text {
                        title: title.into(),
                        subtitle: subtitle.into(),
                    },
                    record_usage: false,
                    icon: None,
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
    use dupin_core::{CommandId, Config, Delivery, Effect, History, Intent, Session};

    #[test]
    fn normal_search_dispatches_each_command_through_its_expected_activation() {
        for command in COMMANDS {
            let config = crate::commands::configured(&Config::default()).unwrap();
            let origin = CommandId::new(crate::commands::SYSTEM).unwrap();
            let mut session = Session::new(History::default(), config.registry).unwrap();
            let (title, _, confirmation) = labels(command);
            let generation = session
                .set_query(title)
                .into_iter()
                .find_map(|effect| match effect {
                    Effect::Search {
                        command, generation, ..
                    } if command == origin => Some(generation),
                    _ => None,
                })
                .unwrap();
            SystemProvider
                .search(&Request::detached(title, COMMANDS.len()), &mut |items| {
                    session.deliver_from(origin.clone(), Delivery::Batch { generation, items });
                })
                .unwrap();
            assert_eq!(session.selected_item().unwrap().display_title(), title);
            let key = session.result_key(0);
            let opened = session.intent(Intent::Open).unwrap();
            let effects = if confirmation.is_some() {
                assert_eq!(
                    opened,
                    [Effect::ShowActions {
                        id: key.clone(),
                        generation,
                        confirmation: Some("run".into()),
                    }]
                );
                assert!(session.run_key_action(&key, "run").is_empty());
                session.run_confirmed_key_action(&key, "run")
            } else {
                assert!(session.run_confirmed_key_action(&key, "run").is_empty());
                opened
            };
            let [Effect::Run(Action::System(picked)), ..] = effects.as_slice() else {
                panic!("expected a system command, got {effects:?}");
            };
            execute(*picked, true, |picked| {
                assert_eq!(picked, command);
                Ok(())
            })
            .unwrap();
            assert!(!effects.contains(&Effect::SaveHistory));
        }
    }

    #[test]
    fn machine_sleep_precedes_display_sleep_and_short_queries_stay_quiet() {
        for query in ["", " ", "s"] {
            SystemProvider
                .search(&Request::detached(query, COMMANDS.len()), &mut |_| {
                    panic!("unexpected system command for {query:?}");
                })
                .unwrap();
        }
        let mut results = Vec::new();
        SystemProvider
            .search(&Request::detached("sleep", COMMANDS.len()), &mut |items| {
                results.extend(items);
            })
            .unwrap();
        assert_eq!(
            results.iter().map(Item::display_title).collect::<Vec<_>>(),
            ["Sleep", "Sleep Displays"]
        );
    }

    #[test]
    fn automation_is_opt_in_and_runner_failures_name_the_command() {
        assert!(!Config::default().system_automation);
        for command in [
            SystemCommand::Restart,
            SystemCommand::ShutDown,
            SystemCommand::LogOut,
            SystemCommand::EmptyTrash,
        ] {
            let error = execute(command, false, |_| panic!("Automation is disabled")).unwrap_err();
            assert!(error.contains("Dupin Settings"));
        }
        let error = execute(SystemCommand::LockScreen, false, |_| Err("Fixture error".into())).unwrap_err();
        assert_eq!(error, "Lock Screen failed. Fixture error");
    }
}
