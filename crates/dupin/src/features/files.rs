use std::sync::Arc;

use dupin_core::{CommandError, CommandId, CommandSpec, Config, QueryEligibility};
use dupin_mac::{FolderProvider, SpotlightProvider, Target};

use super::{Feature, Source};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub const FILES: &str = "files.names";
pub const CONTENTS: &str = "files.contents";
pub(crate) const BROWSE: &str = "files.browse";
pub(crate) const MAX_FILE_CANDIDATES: usize = 10_000;

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    reload,
    sources: &[
        Source {
            name: "files",
            command: FILES,
            matcher: None,
            rows: None,
        },
        Source {
            name: "files.contents",
            command: CONTENTS,
            matcher: None,
            rows: None,
        },
        Source {
            name: "folder",
            command: BROWSE,
            matcher: Some(dupin_core::filter::FUZZY),
            rows: Some(dupin_broker::RowKind::File),
        },
    ],
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let file_limit = config.file_limit.min(MAX_FILE_CANDIDATES);
    let files_id = id(FILES);
    let contents_id = id(CONTENTS);
    let registry = &mut commands.registry;
    for command in [
        CommandSpec::new(
            files_id.clone(),
            file_limit,
            presentation(
                "Search files",
                Some("Files"),
                Some("Type to search files with Spotlight  ·  ⌫ back to apps"),
                Some("No matches"),
                true,
                1.0,
            ),
        )
        .with_candidate_limit(file_limit.saturating_mul(4).min(MAX_FILE_CANDIDATES))
        .with_icon(dupin_core::Icon::Symbol("folder".into())),
        CommandSpec::new(
            contents_id.clone(),
            file_limit,
            presentation(
                "Search inside files",
                Some("In files"),
                Some("Type to search inside files with Spotlight  ·  ⌫ back to apps"),
                Some("No matches"),
                true,
                2.0,
            ),
        )
        .with_title("File Contents")
        .with_icon(dupin_core::Icon::Symbol("doc.text.magnifyingglass".into())),
        CommandSpec::new(
            id(BROWSE),
            file_limit,
            presentation("Filter folder", None, None, Some("No matches"), false, 1.0),
        )
        .with_query_eligibility(QueryEligibility::Any)
        .with_candidate_limit(MAX_FILE_CANDIDATES),
    ] {
        registry.register(command)?;
    }
    registry.set_destination_search(files_id)?;
    registry.set_browser(id(BROWSE))?;
    registry.add_keyword(&contents_id, "in")
}

fn start(providers: &mut Providers) {
    let names = SpotlightProvider::with_demotions(
        Target::Names,
        providers.registration.file_demotions.clone(),
        providers.kinds.clone(),
    );
    providers.add_source("files", Arc::new(names));
    let contents = SpotlightProvider::new(Target::Contents, providers.kinds.clone());
    providers.add_source("files.contents", Arc::new(contents));
    let folder = FolderProvider::new(providers.rows("folder"));
    providers.add_source("folder", Arc::new(folder));
}

fn reload(ui: &mut Ui, config: &Config, _: &mut ConfiguredCommands) -> bool {
    let mut rules = ui.file_demotions.write().unwrap();
    let changed = *rules != config.file_demotions;
    if changed {
        rules.clone_from(&config.file_demotions);
    }
    changed && ui.session.command_id().as_str() == FILES
}

fn parsed_file_query(
    command: &CommandId,
    raw: &str,
) -> Option<Result<dupin_core::file_query::FileQuery, dupin_core::file_query::FileQueryError>> {
    matches!(command.as_str(), FILES | CONTENTS).then(|| dupin_core::file_query::FileQuery::parse(raw))
}

pub fn query_qualifier_ranges(command: &CommandId, raw: &str) -> Vec<std::ops::Range<usize>> {
    parsed_file_query(command, raw)
        .and_then(Result::ok)
        .map(|query| query.qualifier_ranges)
        .unwrap_or_default()
}

pub fn query_problem(command: &CommandId, raw: &str) -> Option<&'static str> {
    match parsed_file_query(command, raw)? {
        Err(error) => Some(error.message()),
        Ok(query) if command.as_str() == CONTENTS && query.terms.is_empty() && !raw.trim().is_empty() => {
            Some("Type a word to search file contents.")
        }
        _ => None,
    }
}
