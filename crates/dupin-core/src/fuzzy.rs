//! Subsequence matching for short names (app names, file names), ranked the way launchers
//! are expected to rank: a prefix beats a word start, which beats a run in the middle, which
//! beats letters scattered across the name.

/// A match of a query against a name: higher `score` ranks first, and `positions` are the
/// character indices in the name that matched, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    pub positions: Vec<usize>,
}

const MATCH: i32 = 16;
const PREFIX: i32 = 64;
const WORD_START: i32 = 32;
const CONSECUTIVE: i32 = 24;
const GAP: i32 = 3;
const GAP_START: i32 = 6;
/// Shorter names win ties: "Safari" over "Safari Technology Preview" for "saf".
const LENGTH: i32 = 1;

/// Matches `query` against `name`, ignoring case and whitespace in the query.
/// None when the query's characters don't all appear in order.
pub fn score(query: &str, name: &str) -> Option<Match> {
    score_prepared(&Query::new(query)?, &Name::new(name))
}

/// A query folded once, to match against many names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query(Vec<char>);

impl Query {
    /// None for a query of only whitespace, which matches nothing.
    pub fn new(query: &str) -> Option<Self> {
        let chars: Vec<char> = query
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        (!chars.is_empty()).then_some(Self(chars))
    }
}

/// A name folded once, for a catalogue that is matched on every keystroke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    chars: Vec<char>,
    lower: Vec<char>,
}

impl Name {
    pub fn new(name: &str) -> Self {
        let chars: Vec<char> = name.chars().collect();
        let lower = chars
            .iter()
            .map(|c| c.to_lowercase().next().unwrap_or(*c))
            .collect();
        Self { chars, lower }
    }
}

/// [`score`] for a query and name folded beforehand.
pub fn score_prepared(query: &Query, name: &Name) -> Option<Match> {
    let mut from = Vec::with_capacity(query.0.len());
    let (mut j, score) = search(query, name, &mut Scratch::default(), Some(&mut from))?;
    let mut positions = vec![0; query.0.len()];
    for i in (0..query.0.len()).rev() {
        positions[i] = j;
        if i > 0 {
            j = from[i][j];
        }
    }
    Some(Match { score, positions })
}

/// Buffers reused from one name to the next, so ranking a long list allocates nothing per
/// name.
#[derive(Debug, Default)]
pub struct Scratch {
    best: Vec<Option<i32>>,
    next: Vec<Option<i32>>,
    back: Vec<usize>,
}

/// The score [`score_prepared`] gives, without the positions it matched, which a long list
/// only needs for the rows it shows.
pub fn score_only(query: &Query, name: &Name, scratch: &mut Scratch) -> Option<i32> {
    search(query, name, scratch, None).map(|(_, score)| score)
}

/// The best score and the name index of its last matched character. With `from`, also each
/// query character's previous position, for reading the path back.
fn search(
    query: &Query,
    name: &Name,
    scratch: &mut Scratch,
    mut from: Option<&mut Vec<Vec<usize>>>,
) -> Option<(usize, i32)> {
    let query = &query.0;
    let (chars, lower) = (&name.chars, &name.lower);
    if !is_subsequence(query, lower) {
        return None;
    }

    // best[j]: the best score with the query so far matched and its last character at name
    // index j. A gap's cost grows linearly with its length, so the best earlier position is a
    // running maximum.
    let n = chars.len();
    let Scratch { best, next, back } = scratch;
    best.clear();
    best.resize(n, None);
    for (i, &q) in query.iter().enumerate() {
        next.clear();
        next.resize(n, None);
        if from.is_some() {
            back.clear();
            back.resize(n, usize::MAX);
        }
        // The best `best[k] + GAP * (k + 1)` over k < j - 1, preferring the later k on ties.
        let mut gapped: Option<(i32, usize)> = None;
        for j in 0..n {
            if j >= 2
                && let Some(s) = best[j - 2]
            {
                let lifted = s + GAP * (j as i32 - 1);
                if gapped.is_none_or(|(g, _)| lifted >= g) {
                    gapped = Some((lifted, j - 2));
                }
            }
            if lower[j] != q {
                continue;
            }
            let bonus = MATCH + position_bonus(chars, j);
            if i == 0 {
                let lead = if j == 0 {
                    PREFIX
                } else {
                    -GAP_START - (j as i32).min(12)
                };
                next[j] = Some(bonus + lead);
                continue;
            }
            let far = gapped.map(|(g, k)| (g - GAP_START - GAP * j as i32, k));
            let near = (j >= 1)
                .then(|| best[j - 1].map(|s| (s + CONSECUTIVE, j - 1)))
                .flatten();
            let chosen = match (far, near) {
                (Some(far), Some(near)) => Some(if near.0 >= far.0 { near } else { far }),
                (far, near) => far.or(near),
            };
            if let Some((s, k)) = chosen {
                next[j] = Some(s + bonus);
                if from.is_some() {
                    back[j] = k;
                }
            }
        }
        std::mem::swap(best, next);
        if let Some(from) = from.as_deref_mut() {
            from.push(back.clone());
        }
    }

    let (j, total) = best
        .iter()
        .enumerate()
        .filter_map(|(j, s)| s.map(|s| (j, s)))
        .max_by_key(|&(j, s)| (s, std::cmp::Reverse(j)))?;
    Some((j, total - LENGTH * n as i32))
}

fn is_subsequence(query: &[char], name: &[char]) -> bool {
    let mut it = name.iter();
    query.iter().all(|q| it.any(|c| c == q))
}

/// Word starts: after a separator, or a capital after a lower-case letter ("FaceTime").
fn position_bonus(chars: &[char], j: usize) -> i32 {
    if j == 0 {
        return WORD_START;
    }
    let (prev, here) = (chars[j - 1], chars[j]);
    if !prev.is_alphanumeric()
        || (prev.is_lowercase() && here.is_uppercase())
        || (!prev.is_numeric() && here.is_numeric())
    {
        WORD_START
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank<'a>(query: &str, names: &[&'a str]) -> Vec<&'a str> {
        let mut hits: Vec<(i32, &str)> = names
            .iter()
            .filter_map(|n| score(query, n).map(|m| (m.score, *n)))
            .collect();
        hits.sort_by_key(|a| std::cmp::Reverse(a.0));
        hits.into_iter().map(|(_, n)| n).collect()
    }

    #[test]
    fn requires_every_character_in_order() {
        assert!(score("saf", "Safari").is_some());
        assert!(score("fas", "Safari").is_none());
        assert!(score("", "Safari").is_none());
        assert!(score("xyz", "Safari").is_none());
    }

    #[test]
    fn ignores_case_and_query_whitespace() {
        assert_eq!(score("SAF", "safari").unwrap().positions, vec![0, 1, 2]);
        assert_eq!(
            score("sys set", "System Settings").unwrap().positions,
            vec![0, 1, 2, 7, 8, 9]
        );
    }

    #[test]
    fn prefix_beats_middle_and_shorter_wins_ties() {
        assert_eq!(
            rank("saf", &["Safari Technology Preview", "Safari", "Hush Safe"]),
            ["Safari", "Safari Technology Preview", "Hush Safe"]
        );
    }

    #[test]
    fn word_starts_beat_scattered_letters() {
        // "vsc": V(isual) S(tudio) C(ode) against letters buried inside words.
        assert_eq!(
            rank("vsc", &["Visual Studio Code", "Avs Cleaner"])[0],
            "Visual Studio Code"
        );
        assert_eq!(score("ft", "FaceTime").unwrap().positions, vec![0, 4]);
    }

    #[test]
    fn consecutive_run_beats_split_letters() {
        assert_eq!(score("term", "Terminal").unwrap().positions, vec![0, 1, 2, 3]);
        assert_eq!(rank("note", &["Notes", "No Tears Edition"])[0], "Notes");
    }

    /// The quadratic search the running maximum replaced.
    fn naive(query: &str, name: &str) -> Option<Match> {
        let query = Query::new(query)?.0;
        let Name { chars, lower } = Name::new(name);
        if !is_subsequence(&query, &lower) {
            return None;
        }
        let n = chars.len();
        let mut best: Vec<Option<i32>> = vec![None; n];
        let mut from: Vec<Vec<usize>> = Vec::new();
        for (i, &q) in query.iter().enumerate() {
            let mut next = vec![None; n];
            let mut back = vec![usize::MAX; n];
            for j in 0..n {
                if lower[j] != q {
                    continue;
                }
                let bonus = MATCH + position_bonus(&chars, j);
                if i == 0 {
                    let lead = if j == 0 {
                        PREFIX
                    } else {
                        -GAP_START - (j as i32).min(12)
                    };
                    next[j] = Some(bonus + lead);
                    continue;
                }
                let step = |k: usize| {
                    if k + 1 == j {
                        CONSECUTIVE
                    } else {
                        -GAP_START - GAP * (j - k - 1) as i32
                    }
                };
                if let Some((s, k)) = (0..j)
                    .filter_map(|k| best[k].map(|s| (s + step(k), k)))
                    .max_by_key(|&(s, _)| s)
                {
                    next[j] = Some(s + bonus);
                    back[j] = k;
                }
            }
            best = next;
            from.push(back);
        }
        let (mut j, mut total) = best
            .iter()
            .enumerate()
            .filter_map(|(j, s)| s.map(|s| (j, s)))
            .max_by_key(|&(j, s)| (s, std::cmp::Reverse(j)))?;
        total -= LENGTH * n as i32;
        let mut positions = vec![0; query.len()];
        for i in (0..query.len()).rev() {
            positions[i] = j;
            if i > 0 {
                j = from[i][j];
            }
        }
        Some(Match {
            score: total,
            positions,
        })
    }

    #[test]
    fn the_running_maximum_scores_exactly_as_the_quadratic_search() {
        let names = [
            "Safari",
            "Safari Technology Preview",
            "System Settings",
            "aaaa bbbb aaaa",
            "abababababab",
            "Visual Studio Code",
            "Aperçu Résumé",
            "x-ryoppippi/dupin issue 218",
            "mississippi",
        ];
        let queries = [
            "s", "saf", "ss", "aba", "abab", "ab", "vsc", "ré", "ssi", "ipi", "dupin 2", "aaa",
        ];
        let mut scratch = Scratch::default();
        for name in names {
            for query in queries {
                assert_eq!(score(query, name), naive(query, name), "{query:?} in {name:?}");
                assert_eq!(
                    score_only(&Query::new(query).unwrap(), &Name::new(name), &mut scratch),
                    naive(query, name).map(|m| m.score),
                    "{query:?} in {name:?}"
                );
            }
        }
    }

    #[test]
    fn positions_index_characters_not_bytes() {
        assert_eq!(score("ré", "Aperçu Résumé").unwrap().positions, vec![7, 8]);
    }
}
