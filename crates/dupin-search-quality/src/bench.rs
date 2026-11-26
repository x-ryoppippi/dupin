//! Types each case into a launcher session built as the app builds its main search, over the
//! corpus, and reads where the wanted row is after every keystroke.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use dupin_core::{
    CommandId, CommandRegistry, CommandSpec, Delivery, Effect, History, Item, Payload, Provider,
    ProviderError, QueryPresentation, Request, Session, View,
};
use dupin_mac::AppsProvider;
use dupin_mac::kinds::Rows;
use dupin_mac::system_commands::SystemProvider;

use crate::fixture::{Case, Corpus, NOW};

// The app's command IDs and result limits, as crates/dupin/src/features registers them.
const APPS: (&str, usize) = ("apps.search", 50);
const SYSTEM: &str = "system.commands";
const WEB: &str = "web.keywords";
const FILES: &str = "files.names";

pub struct Bench {
    corpus: Corpus,
    registry: CommandRegistry,
    providers: HashMap<CommandId, Arc<dyn Provider>>,
}

fn id(value: &str) -> Result<CommandId, String> {
    CommandId::new(value).map_err(|error| format!("{value}: {error}"))
}

fn presentation(placeholder: &str) -> QueryPresentation {
    QueryPresentation {
        placeholder: placeholder.into(),
        chip: None,
        empty_hint: None,
        no_results_hint: None,
        show_searching: false,
        backdrop: 0.0,
    }
}

impl Bench {
    /// The main search over `corpus`: its apps with the native matcher, the system commands,
    /// and a row for a web search whose alias is typed, pinned first, as in the app; the rows
    /// that name commands, web searches and aliased apps come from the session itself.
    pub fn new(corpus: Corpus, rows: Rows) -> Result<Self, String> {
        let error = |error: dupin_core::CommandError| error.to_string();
        let mut registry = CommandRegistry::new(id(APPS.0)?, Some(id(FILES)?));
        registry
            .register(CommandSpec::new(id(APPS.0)?, APPS.1, presentation("Search apps")))
            .map_err(error)?;
        registry
            .register(CommandSpec::new(
                id(SYSTEM)?,
                dupin_mac::system_commands::COMMANDS.len(),
                presentation("Search system commands"),
            ))
            .map_err(error)?;
        registry
            .register(CommandSpec::new(id(WEB)?, 1, presentation("Web search")))
            .map_err(error)?;
        registry
            .register(CommandSpec::new(id(FILES)?, 50, presentation("Search files")).with_title("Files"))
            .map_err(error)?;
        for command in &corpus.commands {
            let command_id = id(&command.id)?;
            registry
                .register(
                    CommandSpec::new(command_id.clone(), 10, presentation(&command.title))
                        .with_title(&command.title),
                )
                .map_err(error)?;
            for keyword in &command.keywords {
                registry.add_keyword(&command_id, keyword).map_err(error)?;
            }
        }
        registry
            .set_default_providers(vec![id(APPS.0)?, id(SYSTEM)?, id(WEB)?], Some(id(WEB)?))
            .map_err(error)?;
        for (alias, name) in &corpus.web {
            registry
                .add_web_alias(alias, format!("Search {name}"))
                .map_err(error)?;
        }
        for (alias, app) in &corpus.aliases {
            registry
                .add_path_alias(alias, corpus.app_path(app)?, app)
                .map_err(error)?;
        }
        let providers: HashMap<CommandId, Arc<dyn Provider>> = HashMap::from([
            (
                id(APPS.0)?,
                Arc::new(AppsProvider::fixed(corpus.apps(), rows)) as Arc<dyn Provider>,
            ),
            (id(SYSTEM)?, Arc::new(SystemProvider::new(false))),
            (id(WEB)?, Arc::new(WebAliases(corpus.web.clone()))),
        ]);
        Ok(Self {
            corpus,
            registry,
            providers,
        })
    }

    /// The ID of the row `intent` wants.
    fn target(&self, intent: &str) -> Result<String, String> {
        let (kind, name) = intent
            .split_once(':')
            .ok_or_else(|| format!("{intent} names no kind of row"))?;
        Ok(match kind {
            "app" => self.corpus.app_path(name)?.to_string_lossy().into_owned(),
            "command" if self.corpus.commands.iter().any(|command| command.id == name) => intent.into(),
            "web" if self.corpus.web.contains_key(name) => intent.into(),
            "system" => name.into(),
            _ => return Err(format!("{intent} is no row the corpus has")),
        })
    }

    /// Where the row `case` wants is after each character of its query, from 1, or `None`
    /// while it is not among the results.
    pub fn ranks(&self, case: &Case) -> Result<Vec<Option<usize>>, String> {
        let fail = |error: String| format!("{}: {error}", case.id);
        let target = self.target(&case.intent).map_err(fail)?;
        let mut history = History::default();
        for (time, item, query) in self.corpus.picks(&case.history).map_err(fail)? {
            history.record(&item, &query, time);
        }
        let mut session = Session::with_clock(history, self.registry.clone(), || NOW)
            .map_err(|error| fail(error.to_string()))?;
        let typed: Vec<usize> = case
            .query
            .char_indices()
            .map(|(at, ch)| at + ch.len_utf8())
            .collect();
        let mut ranks = Vec::with_capacity(typed.len());
        for end in typed {
            for effect in session.set_query(&case.query[..end]) {
                let Effect::Search {
                    command,
                    generation,
                    query,
                    ..
                } = effect
                else {
                    continue;
                };
                if let Some(provider) = self.providers.get(&command) {
                    let limit = session
                        .provider_spec(&command)
                        .map_or(0, CommandSpec::candidate_limit);
                    let mut batches = Vec::new();
                    provider
                        .search_batches(&Request::detached(query, limit), &mut |batch| batches.push(batch))
                        .map_err(|ProviderError| fail(format!("{command} failed")))?;
                    for batch in batches {
                        session.deliver_from(command.clone(), Delivery::Presented { generation, batch });
                    }
                }
                session.deliver_from(command, Delivery::Done { generation });
            }
            ranks.push(
                session
                    .items()
                    .iter()
                    .position(|item| item.id == target)
                    .map(|index| index + 1),
            );
        }
        Ok(ranks)
    }
}

/// The row the app's web search shows for a web search whose alias is the query's first word,
/// pinned above the other results. The search itself runs in the bundled web-search
/// extension, which the benchmark leaves out; only the row's place matters here.
struct WebAliases(BTreeMap<String, String>);

impl Provider for WebAliases {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let typed = request.query.split_whitespace().next().unwrap_or_default();
        if let Some(name) = self.0.get(typed) {
            emit(vec![Item {
                id: format!("web:{typed}"),
                view: View::Text {
                    title: format!("Search {name}"),
                    subtitle: "Web search".into(),
                },
                payload: Payload::default(),
                record_usage: false,
                icon: None,
                matched: Vec::new(),
                completion: None,
                score: None,
                primary: None,
                alternate: None,
                actions: Vec::new(),
            }]);
        }
        Ok(())
    }
}
