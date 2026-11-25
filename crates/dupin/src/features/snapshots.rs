use std::sync::Arc;

use dupin_core::{
    Action, CommandError, CommandSpec, Config, Item, ItemAction, Payload, Provider, ProviderError,
    QueryEligibility, Request, View,
};

use super::Feature;
use crate::commands::{Configure, Providers, id, presentation};
use crate::control::SnapshotSummary;

pub(crate) const SNAPSHOTS: &str = "snapshots.list";
const KEYWORD: &str = "snap";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    ..Feature::EMPTY
};

fn configure(_: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(SNAPSHOTS);
    let mut spec = CommandSpec::new(
        command.clone(),
        100,
        presentation(
            "Search snapshots",
            Some("Snapshots"),
            Some("No snapshots yet: save a settled search with ⌘⇧S  ·  ⌫ back to apps"),
            Some("No matching snapshots"),
            false,
            0.0,
        ),
    )
    .with_query_eligibility(QueryEligibility::Any);
    spec.private_results = true;
    commands.registry.register(spec)?;
    if commands.registry.add_keyword(&command, KEYWORD).is_err() {
        commands.notices.push(format!(
            "Snapshots has no keyword: `{KEYWORD}` already reaches another command."
        ));
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(SNAPSHOTS, Arc::new(SnapshotsProvider));
}

/// The saved snapshots, by title, from the metadata the Snapshots window lists, which the
/// keyword reads from the archive itself only if the window hasn't. It answers only its own
/// keyword, so the main search never reads it.
struct SnapshotsProvider;

impl Provider for SnapshotsProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let (entries, sources) = {
            let mut listing = crate::snapshots::listing();
            let rows = rows(listing.entries(), &request.query, request.limit);
            (rows, listing.sources.clone())
        };
        let items: Vec<Item> = entries
            .into_iter()
            .map(|(summary, matched)| item(summary, matched, &sources))
            .collect();
        if !items.is_empty() && !request.is_cancelled() {
            emit(items);
        }
        Ok(())
    }
}

/// The snapshots whose titles match `query`, best first, or all of them newest first.
fn rows(entries: &[SnapshotSummary], query: &str, limit: usize) -> Vec<(SnapshotSummary, Vec<usize>)> {
    let query = query.trim();
    let mut found: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(order, summary)| {
            if query.is_empty() {
                return Some((0, order, summary, Vec::new()));
            }
            let found = dupin_core::fuzzy::score(query, &summary.title)?;
            Some((found.score, order, summary, found.positions))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found
        .into_iter()
        .take(limit)
        .map(|(_, _, summary, matched)| (summary.clone(), matched))
        .collect()
}

fn item(summary: SnapshotSummary, matched: Vec<usize>, sources: &[String]) -> Item {
    let mut subtitle = format!(
        "{} · {}",
        crate::snapshots::saved_date(summary.saved_at),
        crate::snapshots::source_name(&summary.mode)
    );
    if !summary.query.is_empty() {
        subtitle.push_str(" · ");
        subtitle.push_str(&summary.query);
    }
    let mut actions = vec![ItemAction::new(
        "open",
        "Open in Snapshots",
        Action::OpenSnapshots(Some(summary.id.clone())),
    )];
    // Resuming runs the search again, which only a command that still exists can.
    let resumes = sources.contains(&summary.mode);
    if resumes {
        actions.push(
            ItemAction::new(
                "resume",
                "Resume Search",
                Action::ResumeSnapshot(summary.id.clone()),
            )
            .keep_open(),
        );
    }
    Item {
        id: summary.id,
        view: View::Text {
            title: summary.title,
            subtitle,
        },
        payload: Payload::default(),
        record_usage: false,
        icon: None,
        matched,
        completion: None,
        score: None,
        primary: Some("open".into()),
        alternate: resumes.then(|| "resume".into()),
        actions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: &str, title: &str, saved_at: u64, mode: &str) -> SnapshotSummary {
        SnapshotSummary {
            id: id.into(),
            saved_at,
            mode: mode.into(),
            title: title.into(),
            query: "rust".into(),
        }
    }

    #[test]
    fn snapshots_list_newest_first_and_filter_by_title() {
        let entries = [
            summary("b", "Rust lifetimes", 2, "web.keywords"),
            summary("a", "Quarterly report", 1, "extension.gone"),
        ];
        let ids = |query: &str| -> Vec<String> {
            rows(&entries, query, 10)
                .into_iter()
                .map(|(summary, _)| summary.id)
                .collect()
        };
        assert_eq!(ids(""), ["b", "a"]);
        assert_eq!(ids("quart"), ["a"]);
        assert!(ids("zzz").is_empty());
        assert_eq!(ids("").len(), rows(&entries, "", 1).len() + 1);
    }

    #[test]
    fn enter_opens_the_window_and_only_an_available_command_resumes() {
        let sources = ["web.keywords".to_owned()];
        let row = item(
            summary("b", "Rust lifetimes", 0, "web.keywords"),
            Vec::new(),
            &sources,
        );
        assert_eq!(row.action("open"), Some(&Action::OpenSnapshots(Some("b".into()))));
        assert_eq!(row.alternate.as_deref(), Some("resume"));
        assert_eq!(row.action("resume"), Some(&Action::ResumeSnapshot("b".into())));
        let View::Text { subtitle, .. } = &row.view else {
            panic!("a text row");
        };
        assert_eq!(subtitle, "1970-01-01 00:00 UTC · Web search · rust");

        let gone = item(summary("a", "Report", 0, "extension.gone"), Vec::new(), &sources);
        assert_eq!(gone.alternate, None);
        assert!(gone.action("resume").is_none());
        assert!(!gone.record_usage);
    }
}
