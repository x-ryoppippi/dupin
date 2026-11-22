//! Filename, recency and path signals for file search. This module only scores supplied data;
//! it does not inspect the filesystem or read platform configuration.

use std::path::{Component, Path, PathBuf};

use crate::fuzzy;

const LAST_USED_MAX_BONUS: i32 = 48;
const LAST_USED_HALF_LIFE: u64 = 7 * 24 * 60 * 60;
const MODIFIED_MAX_BONUS: i32 = 24;
const MODIFIED_HALF_LIFE: u64 = 30 * 24 * 60 * 60;
const DEPTH_PENALTY: i32 = 2;
const MAX_DEPTH_PENALTY: i32 = 48;
const NOISE_DEMOTION: i32 = 320;
const MAX_SCORE: i32 = 16_384;

/// File timestamps in seconds since the Unix epoch, when available.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileDates {
    pub last_used: Option<u64>,
    pub modified: Option<u64>,
}

/// Score a file from its display title, timestamps and lexical path.
///
/// Query words are matched independently, so they may occur in any order. A word absent from the
/// title contributes no filename score but does not reject the result. `demotions` must already
/// have any platform-specific home-directory expansion applied.
pub fn score(
    query: &str,
    title: &str,
    path: &Path,
    dates: FileDates,
    now: u64,
    demotions: &[PathBuf],
) -> fuzzy::Match {
    let mut total = 0_i32;
    let mut positions = Vec::new();
    let mut seen_words = Vec::<String>::new();

    for word in query.split_whitespace() {
        let normalized = word.to_lowercase();
        if seen_words.iter().any(|seen| seen == &normalized) {
            continue;
        }
        seen_words.push(normalized);

        if let Some(found) = fuzzy::score(word, title) {
            total = total.saturating_add(found.score);
            positions.extend(found.positions);
        }
    }

    total = total.saturating_add(recency_bonus(
        dates.last_used,
        now,
        LAST_USED_MAX_BONUS,
        LAST_USED_HALF_LIFE,
    ));
    total = total.saturating_add(recency_bonus(
        dates.modified,
        now,
        MODIFIED_MAX_BONUS,
        MODIFIED_HALF_LIFE,
    ));
    total = total.saturating_sub(depth_penalty(path));
    if !demotions.is_empty() && is_demoted(path, demotions) {
        total = total.saturating_sub(NOISE_DEMOTION);
    }

    positions.sort_unstable();
    positions.dedup();

    fuzzy::Match {
        score: total.clamp(-MAX_SCORE, MAX_SCORE),
        positions,
    }
}

fn recency_bonus(timestamp: Option<u64>, now: u64, maximum: i32, half_life: u64) -> i32 {
    let Some(timestamp) = timestamp else {
        return 0;
    };
    let age = now.saturating_sub(timestamp);
    let numerator = u64::from(maximum as u32).saturating_mul(half_life);
    (numerator / age.saturating_add(half_life)) as i32
}

fn depth_penalty(path: &Path) -> i32 {
    let directories = path
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter(|component| matches!(component, Component::Normal(_) | Component::ParentDir))
        .count();
    (directories.min((MAX_DEPTH_PENALTY / DEPTH_PENALTY) as usize) as i32) * DEPTH_PENALTY
}

fn is_demoted(path: &Path, demotions: &[PathBuf]) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let directories = parent.components().collect::<Vec<_>>();

    demotions.iter().any(|rule| {
        if rule.as_os_str().is_empty() {
            return false;
        }
        if rule.is_absolute() {
            return parent.starts_with(rule);
        }

        let pattern = rule.components().collect::<Vec<_>>();
        !pattern.is_empty()
            && pattern.len() <= directories.len()
            && directories.windows(pattern.len()).any(|window| {
                window
                    .iter()
                    .zip(&pattern)
                    .all(|(directory, pattern)| components_equal(*directory, *pattern))
            })
    })
}

fn components_equal(left: Component<'_>, right: Component<'_>) -> bool {
    let left = left.as_os_str();
    let right = right.as_os_str();
    match (left.to_str(), right.to_str()) {
        (Some(left), Some(right)) => left.to_lowercase() == right.to_lowercase(),
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn rank(query: &str, title: &str, path: &str, demotions: &[PathBuf]) -> fuzzy::Match {
        score(
            query,
            title,
            Path::new(path),
            FileDates::default(),
            NOW,
            demotions,
        )
    }

    #[test]
    fn preserves_fuzzy_prefix_and_word_boundary_quality() {
        let prefix = rank("saf", "Safari", "/work/a", &[]);
        let scattered_prefix = rank("saf", "Hush Safe", "/work/a", &[]);
        let boundary = rank("vsc", "Visual Studio Code", "/work/a", &[]);
        let scattered = rank("vsc", "Avs Cleaner", "/work/a", &[]);
        assert!(prefix.score > scattered_prefix.score);
        assert!(boundary.score > scattered.score);
        assert_eq!(boundary.positions, [0, 7, 14]);
    }

    #[test]
    fn matches_words_in_any_order_and_unions_unicode_character_positions() {
        let first = rank("note 日本語", "🗃 日本語 Note", "/work/a", &[]);
        let reversed = rank("日本語 note", "🗃 日本語 Note", "/work/a", &[]);
        assert_eq!(first.score, reversed.score);
        assert_eq!(first.positions, [2, 3, 4, 6, 7, 8, 9]);
        assert_eq!(reversed.positions, first.positions);
    }

    #[test]
    fn repeated_query_words_only_contribute_once() {
        assert_eq!(
            rank("note note", "Note", "/work/a", &[]),
            rank("note", "Note", "/work/a", &[])
        );
    }

    #[test]
    fn unmatched_content_words_do_not_reject_a_file_title_match() {
        let complete = rank("meeting notes", "Meeting Notes.md", "/work/a", &[]);
        let partial = rank("meeting from report", "Meeting Notes.md", "/work/a", &[]);
        assert!(!complete.positions.is_empty());
        assert_eq!(partial.positions, [0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn last_used_and_modified_recency_are_bounded_and_handle_missing_or_future_dates() {
        let evaluate = |dates| score("", "file", Path::new("/work/file"), dates, NOW, &[]).score;
        let absent = evaluate(FileDates::default());
        let fresh = evaluate(FileDates {
            last_used: Some(NOW),
            modified: Some(NOW),
        });
        let future = evaluate(FileDates {
            last_used: Some(NOW + 10_000),
            modified: Some(NOW + 10_000),
        });
        let old = evaluate(FileDates {
            last_used: Some(NOW - 14 * 24 * 60 * 60),
            modified: Some(NOW - 60 * 24 * 60 * 60),
        });

        assert_eq!(absent, -2);
        assert_eq!(future, fresh);
        assert!(fresh > old);
        assert!(old > absent);
        assert!(fresh - absent <= LAST_USED_MAX_BONUS + MODIFIED_MAX_BONUS);

        let recent_use = evaluate(FileDates {
            last_used: Some(NOW),
            modified: None,
        });
        let recent_edit = evaluate(FileDates {
            last_used: None,
            modified: Some(NOW),
        });
        let older_use = evaluate(FileDates {
            last_used: Some(NOW - 14 * 24 * 60 * 60),
            modified: None,
        });
        let older_edit = evaluate(FileDates {
            last_used: None,
            modified: Some(NOW - 60 * 24 * 60 * 60),
        });
        let future_use = evaluate(FileDates {
            last_used: Some(NOW + 10_000),
            modified: None,
        });
        let future_edit = evaluate(FileDates {
            last_used: None,
            modified: Some(NOW + 10_000),
        });
        assert!(recent_use > older_use && older_use > absent);
        assert!(recent_edit > older_edit && older_edit > absent);
        assert_eq!(future_use, recent_use);
        assert_eq!(future_edit, recent_edit);
        assert!(recent_use > recent_edit);
    }

    #[test]
    fn path_depth_has_a_modest_bounded_penalty() {
        let shallow = rank("", "file", "/Users/alice/file", &[]);
        let deep = rank("", "file", "/Users/alice/a/b/c/d/e/f/g/file", &[]);
        assert!(shallow.score > deep.score);
        assert!(shallow.score - deep.score <= MAX_DEPTH_PENALTY);
    }

    #[test]
    fn default_noise_directories_are_demoted() {
        let rules = [
            PathBuf::from("/Users/alice/Library"),
            PathBuf::from("Caches"),
            PathBuf::from("node_modules"),
            PathBuf::from(".git"),
            PathBuf::from("target"),
            PathBuf::from("build"),
            PathBuf::from("dist"),
        ];
        let clean = rank("draft", "Draft.md", "/Users/alice/Documents/Draft.md", &rules);
        for path in [
            "/Users/alice/Library/Logs/Draft.md",
            "/Users/alice/Projects/Caches/Draft.md",
            "/Users/alice/Projects/node_modules/pkg/Draft.md",
            "/Users/alice/Projects/.git/objects/Draft.md",
            "/Users/alice/Projects/target/debug/Draft.md",
            "/Users/alice/Projects/build/output/Draft.md",
            "/Users/alice/Projects/dist/Draft.md",
        ] {
            let without_rules = rank("draft", "Draft.md", path, &[]);
            assert!(
                rank("draft", "Draft.md", path, &rules).score < without_rules.score,
                "{path}"
            );
        }
        assert!(clean.score > rank("draft", "Draft.md", "/Users/alice/Library/Draft.md", &rules).score);
    }

    #[test]
    fn custom_rules_match_exact_directory_components_without_matching_prefixes_or_files() {
        let rules = [PathBuf::from("Private/Archive"), PathBuf::from("BUILD")];
        let clean = rank("draft", "Draft.md", "/Users/alice/Documents/Draft.md", &rules);
        assert!(
            rank(
                "draft",
                "Draft.md",
                "/Users/alice/Private/Archive/Draft.md",
                &rules
            )
            .score
                < clean.score
        );
        assert!(rank("draft", "Draft.md", "/Users/alice/other/build/Draft.md", &rules).score < clean.score);
        let prefix_lookalike = rank(
            "draft",
            "Draft.md",
            "/Users/alice/Private/Archiver/Draft.md",
            &rules,
        );
        assert_eq!(
            prefix_lookalike,
            rank("draft", "Draft.md", "/Users/alice/Private/Archiver/Draft.md", &[])
        );
        let file_named_build = rank("draft", "Draft.md", "/Users/alice/Documents/build", &rules);
        assert_eq!(
            file_named_build,
            rank("draft", "Draft.md", "/Users/alice/Documents/build", &[])
        );
    }

    #[test]
    fn absolute_rules_are_component_prefixes_and_empty_rules_can_disable_demotions() {
        let rules = [PathBuf::from("/Users/alice/Library"), PathBuf::new()];
        let clean = rank("draft", "Draft.md", "/Users/alice/LibraryExtra/Draft.md", &rules);
        let exact = rank("draft", "Draft.md", "/Users/alice/Library/Draft.md", &rules);
        assert!(exact.score < clean.score);

        let baseline = rank("draft", "Draft.md", "/Users/alice/Library/Draft.md", &[]);
        assert_eq!(
            rank(
                "draft",
                "Draft.md",
                "/Users/alice/Library/Draft.md",
                &[PathBuf::new()]
            ),
            baseline
        );
    }
}
