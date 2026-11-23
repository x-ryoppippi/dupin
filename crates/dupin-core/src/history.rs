//! What the user opened, for ranking: frecency (how often, weighted towards recently, as
//! Firefox ranks its address bar) and Alfred-style knowledge of which result was picked for
//! which query.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Item;

/// Visits kept per item; frecency samples these.
const VISITS_KEPT: usize = 10;
/// Queries remembered per item.
const QUERIES_KEPT: usize = 20;
/// Items remembered at all; the least frecent go first.
const ITEMS_KEPT: usize = 500;

const DAY: u64 = 24 * 60 * 60;

/// How much a visit counts by its age, as Firefox's frecency buckets do.
fn weight(age_secs: u64) -> f64 {
    match age_secs / DAY {
        0..=3 => 100.0,
        4..=13 => 70.0,
        14..=30 => 50.0,
        31..=89 => 30.0,
        _ => 10.0,
    }
}

/// The boost for frecency is logarithmic and bounded: it reorders results that match about
/// equally well, but a strong match (a prefix) still beats a weak one (scattered letters)
/// that was opened often. A first visit adds about 55 and a hundred recent ones about 110,
/// where fuzzy scores for a prefix and a scattered match differ by well over 100.
const FRECENCY_SCALE: f64 = 12.0;
/// Picked before for exactly this query: Alfred's knowledge puts it first.
const SAME_QUERY: i32 = 1000;
/// Picked before for a longer query this one begins (picked for "safari", now "saf").
const LONGER_QUERY: i32 = 200;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    items: HashMap<String, Entry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Entry {
    /// Every time the item was opened, not just the visits kept.
    count: u32,
    /// Unix seconds of the latest visits, oldest first.
    visits: Vec<u64>,
    /// How many times the item was opened for each query (trimmed and lowercased).
    queries: HashMap<String, u32>,
}

impl Entry {
    fn frecency(&self, now: u64) -> f64 {
        if self.visits.is_empty() {
            return 0.0;
        }
        let sampled: f64 = self.visits.iter().map(|&at| weight(now.saturating_sub(at))).sum();
        f64::from(self.count) * sampled / self.visits.len() as f64
    }
}

fn normalize(query: &str) -> String {
    query.trim().to_lowercase()
}

impl History {
    pub fn from_json(src: &str) -> serde_json::Result<History> {
        serde_json::from_str(src)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("history serializes")
    }

    /// Return up to `limit` item ids ordered by frecency at `now`.
    pub(crate) fn ranked_ids(&self, limit: usize, now: u64) -> Vec<String> {
        let mut ranked: Vec<_> = self
            .items
            .iter()
            .map(|(id, entry)| {
                (
                    id,
                    entry.frecency(now),
                    entry.visits.iter().copied().max().unwrap_or(0),
                )
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(b.0)));
        ranked
            .into_iter()
            .take(limit)
            .map(|(id, _, _)| id.clone())
            .collect()
    }

    /// The item with this id was opened, after typing `query`, at `now` (Unix seconds).
    pub(crate) fn record(&mut self, id: &str, query: &str, now: u64) {
        let entry = self.items.entry(id.to_owned()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.visits.push(now);
        if entry.visits.len() > VISITS_KEPT {
            entry.visits.remove(0);
        }
        let query = normalize(query);
        if !query.is_empty() {
            *entry.queries.entry(query).or_default() += 1;
            if entry.queries.len() > QUERIES_KEPT {
                // Forget the query picked least often (ties: the alphabetically last).
                if let Some(rarest) = entry
                    .queries
                    .iter()
                    .min_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                    .map(|(q, _)| q.clone())
                {
                    entry.queries.remove(&rarest);
                }
            }
        }
        if self.items.len() > ITEMS_KEPT {
            let stalest = self
                .items
                .iter()
                .min_by(|a, b| a.1.frecency(now).total_cmp(&b.1.frecency(now)).then(b.0.cmp(a.0)))
                .map(|(id, _)| id.clone());
            if let Some(id) = stalest {
                self.items.remove(&id);
            }
        }
    }

    /// What the history adds to an item's match score for `query` at `now`.
    pub(crate) fn boost(&self, id: &str, query: &str, now: u64) -> i32 {
        let Some(entry) = self.items.get(id) else { return 0 };
        let query = normalize(query);
        let knowledge = if entry.queries.contains_key(&query) {
            SAME_QUERY
        } else if !query.is_empty() && entry.queries.keys().any(|q| q.starts_with(&query)) {
            LONGER_QUERY
        } else {
            0
        };
        knowledge + (entry.frecency(now).ln_1p() * FRECENCY_SCALE) as i32
    }

    /// Sort ranked items (all with a `score`) by score plus boost, keeping the providers'
    /// order among equals. Items without scores keep the provider's order.
    pub(crate) fn rerank(&self, items: &mut [Item], query: &str, now: u64) {
        if items.is_empty() || items.iter().any(|item| item.score.is_none()) {
            return;
        }
        items.sort_by_cached_key(|item| {
            let boost = if item.record_usage {
                self.boost(&item.id, query, now)
            } else {
                0
            };
            std::cmp::Reverse(item.score.unwrap_or(0) + boost)
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn app(name: &str, score: i32) -> Item {
        Item {
            score: Some(score),
            ..Item::path(PathBuf::from(format!("/A/{name}.app")), name.into(), vec![])
        }
    }

    fn titles(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.display_title()).collect()
    }

    #[test]
    fn an_item_picked_for_this_query_comes_first() {
        let mut history = History::default();
        history.record("/A/Shareful.app", "s", NOW);
        let mut items = vec![app("Safari", 120), app("Shareful", 80)];
        history.rerank(&mut items, "s", NOW);
        assert_eq!(titles(&items), ["Shareful", "Safari"]);
    }

    #[test]
    fn a_pick_for_a_longer_query_helps_its_prefixes() {
        let mut history = History::default();
        history.record("/A/Terminal.app", "terminal", NOW);
        assert_eq!(
            history.boost("/A/Terminal.app", "TER ", NOW) - history.boost("/A/Terminal.app", "x", NOW),
            LONGER_QUERY
        );
    }

    #[test]
    fn frecency_reorders_close_matches_but_not_a_much_better_one() {
        let mut history = History::default();
        for day in 0..20 {
            history.record("/A/Often.app", "something else", NOW - day * DAY / 2);
        }
        let mut close = vec![app("Rare", 150), app("Often", 140)];
        history.rerank(&mut close, "q", NOW);
        assert_eq!(titles(&close), ["Often", "Rare"]);
        let mut far = vec![app("Prefix", 300), app("Often", 100)];
        history.rerank(&mut far, "q", NOW);
        assert_eq!(titles(&far), ["Prefix", "Often"]);
    }

    #[test]
    fn recent_visits_count_more_than_old_ones() {
        let mut history = History::default();
        history.record("recent", "", NOW - DAY);
        history.record("old", "", NOW - 200 * DAY);
        assert!(history.boost("recent", "q", NOW) > history.boost("old", "q", NOW));
        assert_eq!(history.boost("never", "q", NOW), 0);
    }

    #[test]
    fn ranked_ids_are_frecent_bounded_and_deterministic_without_writing_history() {
        let mut history = History::default();
        history.record("old", "", NOW - 200 * DAY);
        history.record("z", "", NOW - DAY);
        history.record("a", "", NOW - DAY);
        history.record("recent", "", NOW);
        let before = history.clone();

        assert_eq!(history.ranked_ids(3, NOW), ["recent", "a", "z"]);
        assert_eq!(history.ranked_ids(usize::MAX, NOW), ["recent", "a", "z", "old"]);
        assert!(history.ranked_ids(0, NOW).is_empty());
        assert_eq!(history, before);
    }

    #[test]
    fn unranked_streams_keep_their_order() {
        let mut history = History::default();
        history.record("/x/b.txt", "b", NOW);
        let mut items: Vec<Item> = ["a.txt", "b.txt"]
            .iter()
            .map(|n| Item::path(PathBuf::from(format!("/x/{n}")), (*n).into(), vec![]))
            .collect();
        history.rerank(&mut items, "b", NOW);
        assert_eq!(titles(&items), ["a.txt", "b.txt"]);
    }

    #[test]
    fn keeps_a_bounded_number_of_visits_and_queries() {
        let mut history = History::default();
        for i in 0..30 {
            history.record("id", &format!("q{i}"), NOW + i);
        }
        let entry = &history.items["id"];
        assert_eq!(entry.count, 30);
        assert_eq!(entry.visits.len(), VISITS_KEPT);
        assert_eq!(entry.visits.first(), Some(&(NOW + 20)));
        assert_eq!(entry.queries.len(), QUERIES_KEPT);
    }

    #[test]
    fn round_trips_through_json() {
        let mut history = History::default();
        history.record("/A/Safari.app", "saf", NOW);
        assert_eq!(History::from_json(&history.to_json()).unwrap(), history);
    }
    #[test]
    fn opted_out_items_do_not_receive_old_history_boosts() {
        let mut history = History::default();
        history.record("/A/Old.app", "q", NOW);
        let mut old = app("Old", 80);
        old.record_usage = false;
        let mut detail = app("Detail", 100);
        detail.view = crate::View::Detail {
            title: "Detail".into(),
            body: "Body".into(),
        };
        let mut items = vec![old, detail];
        history.rerank(&mut items, "q", NOW);
        assert_eq!(titles(&items), ["Detail", "Old"]);
        history.record("/A/Detail.app", "q", NOW);
        items[0].score = Some(1);
        history.rerank(&mut items, "q", NOW);
        assert_eq!(titles(&items), ["Detail", "Old"]);
    }
}
