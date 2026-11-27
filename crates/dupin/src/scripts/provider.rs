use super::catalog::Catalog;
use dupin_core::script::{OutputMode, ScriptRun};
use dupin_core::{Action, Item, ItemAction, Provider, ProviderError, Request, View};
use std::sync::{Arc, RwLock};

pub const ARGUMENTS: &str = "arguments";
pub const RUN: &str = "run";

pub struct ScriptsProvider(pub Arc<RwLock<Catalog>>);

pub fn run_action(command: ScriptRun) -> ItemAction {
    let inline = command.mode == OutputMode::Inline;
    let action = ItemAction::new(RUN, "Run script", Action::RunScript(command));
    if inline { action.keep_open() } else { action }
}

impl Provider for ScriptsProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.is_cancelled() || request.limit == 0 {
            return Ok(());
        }
        let catalog = self.0.read().unwrap();
        let mut items: Vec<_> = catalog
            .scripts
            .values()
            .filter_map(|script| {
                let matched = dupin_core::fuzzy::score(&request.query, &script.header.title)?;
                let action = if script.header.arguments.is_empty() {
                    run_action(script.run(Vec::new()))
                } else {
                    ItemAction::new(
                        ARGUMENTS,
                        "Enter arguments…",
                        Action::ScriptArguments(script.path.clone()),
                    )
                    .keep_open()
                };
                let synopsis = script.header.synopsis();
                Some(Item {
                    payload: Default::default(),
                    id: format!("script:{}", script.path.display()),
                    view: View::Text {
                        title: script.header.title.clone(),
                        subtitle: if synopsis.is_empty() {
                            "Script".into()
                        } else {
                            format!("Script · {synopsis}")
                        },
                    },
                    record_usage: true,
                    icon: Some(match &script.header.icon {
                        Some(icon) => dupin_core::Icon::declared(icon.clone()),
                        None => dupin_core::Icon::File(script.path.clone()),
                    }),
                    matched: matched.positions,
                    completion: None,
                    score: Some(matched.score),
                    primary: Some(action.id.clone()),
                    alternate: None,
                    actions: vec![action],
                })
            })
            .collect();
        items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        items.truncate(request.limit);
        if !request.is_cancelled() {
            emit(items);
        }
        Ok(())
    }
}
