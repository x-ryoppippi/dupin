use super::catalog::Script;
use dupin_core::script::{ScriptRun, split_args};
use rune::TextEdit;

pub struct Entry {
    pub key: String,
    pub script: Script,
    pub query: TextEdit,
}

impl Entry {
    pub fn submit(
        entry: &mut Option<Self>,
        session: &mut dupin_core::Session,
        catalog: &super::catalog::Catalog,
    ) -> Vec<dupin_core::Effect> {
        let Some(command) = entry
            .as_ref()
            .filter(|entry| entry.is_current(session, catalog))
            .and_then(|entry| entry.command().ok())
        else {
            return Vec::new();
        };
        let entry = entry.take().unwrap();
        session.run_key_followup(
            &entry.key,
            super::provider::ARGUMENTS,
            super::provider::run_action(command),
        )
    }

    fn command(&self) -> Result<ScriptRun, String> {
        let args = split_args(&self.query.text)?;
        self.script.header.bind_args(&args)?;
        Ok(ScriptRun {
            path: self.script.path.clone(),
            title: self.script.header.title.clone(),
            args,
            mode: self.script.header.mode,
        })
    }

    pub fn is_current(&self, session: &dupin_core::Session, catalog: &super::catalog::Catalog) -> bool {
        catalog.scripts.get(&self.script.path) == Some(&self.script)
            && session.items().iter().enumerate().any(|(i, item)| {
                session.result_key(i) == self.key
                    && item.action(super::provider::ARGUMENTS)
                        == Some(&dupin_core::Action::ScriptArguments(self.script.path.clone()))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_entry(
        mode: dupin_core::script::OutputMode,
    ) -> (Option<Entry>, dupin_core::Session, super::super::catalog::Catalog) {
        use dupin_core::{
            CommandId, CommandRegistry, CommandSpec, Delivery, Effect, History, Provider, QueryPresentation,
            Request, Session,
        };
        let (mut header, _) =
            dupin_core::script::Header::parse(b"# @dupin.argument value", "Fixture").unwrap();
        header.mode = mode;
        let script = Script {
            path: "/fixture/script".into(),
            header,
        };
        let mut catalog = super::super::catalog::Catalog::default();
        catalog.scripts.insert(script.path.clone(), script.clone());
        let provider = super::super::provider::ScriptsProvider(std::sync::Arc::new(std::sync::RwLock::new(
            catalog.clone(),
        )));
        let mut rows = Vec::new();
        provider
            .search(&Request::detached("Fixture", 8), &mut |items| rows.extend(items))
            .unwrap();
        let id = CommandId::new("scripts.search").unwrap();
        let mut registry = CommandRegistry::new(id.clone(), None);
        registry
            .register(CommandSpec::new(
                id.clone(),
                8,
                QueryPresentation {
                    placeholder: "Scripts".into(),
                    chip: None,
                    empty_hint: None,
                    no_results_hint: None,
                    show_searching: false,
                    backdrop: 0.0,
                },
            ))
            .unwrap();
        let mut session = Session::new(History::default(), registry).unwrap();
        let generation = session
            .set_query("Fixture")
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Search { generation, .. } => Some(generation),
                _ => None,
            })
            .unwrap();
        assert!(session.deliver_from(
            id.clone(),
            Delivery::Batch {
                generation,
                items: rows
            }
        ));
        assert!(session.deliver_from(id, Delivery::Done { generation }));
        let entry = Entry {
            key: session.result_key(0),
            script,
            query: TextEdit::new("'hello world'"),
        };
        (Some(entry), session, catalog)
    }

    #[test]
    fn argument_submission_runs_once_with_the_modes_history_disposition() {
        use dupin_core::{Effect, script::OutputMode};
        for mode in [OutputMode::Inline, OutputMode::Silent, OutputMode::Full] {
            let (mut entry, mut session, catalog) = active_entry(mode);
            entry.as_mut().unwrap().query = TextEdit::new("'unfinished");
            assert!(Entry::submit(&mut entry, &mut session, &catalog).is_empty());
            assert!(entry.is_some());
            entry.as_mut().unwrap().query = TextEdit::new("'hello world'");
            let effects = Entry::submit(&mut entry, &mut session, &catalog);
            assert!(
                matches!(&effects[0],Effect::Run(dupin_core::Action::RunScript(run)) if run.args==["hello world"])
            );
            assert_eq!(
                effects.iter().any(|effect| matches!(effect, Effect::Hide)),
                mode != OutputMode::Inline
            );
            assert!(effects.contains(&Effect::SaveHistory));
            assert!(entry.is_none());
            assert!(Entry::submit(&mut entry, &mut session, &catalog).is_empty());
        }
    }

    #[test]
    fn reloaded_or_stale_entries_cannot_submit() {
        use dupin_core::script::OutputMode;
        for change in [0, 1, 2] {
            let (mut entry, mut session, mut catalog) = active_entry(OutputMode::Inline);
            match change {
                0 => {
                    catalog.scripts.clear();
                }
                1 => {
                    catalog.scripts.values_mut().next().unwrap().header.title = "Changed".into();
                }
                _ => {
                    session.refresh();
                }
            }
            assert!(!entry.as_ref().unwrap().is_current(&session, &catalog));
            assert!(Entry::submit(&mut entry, &mut session, &catalog).is_empty());
            assert!(entry.is_some());
        }
    }

    #[test]
    fn entry_previews_literal_argv_and_refuses_incomplete_arguments() {
        let (header, _) = dupin_core::script::Header::parse(b"# @dupin.argument values+", "Fixture").unwrap();
        let mut entry = Entry {
            key: "fixture".into(),
            script: Script {
                path: "/fixture/script".into(),
                header,
            },
            query: TextEdit::new(""),
        };
        assert!(entry.command().is_err());
        entry.query = TextEdit::new("'hello world' $HOME ~ *.rs");
        assert_eq!(
            entry.command().unwrap().args,
            ["hello world", "$HOME", "~", "*.rs"]
        );
        entry.query = TextEdit::new("'unfinished");
        assert!(entry.command().is_err());
    }
}
