//! Subsequence matching for short names (app names, file names), ranked the way launchers
//! are expected to rank. A match falls in one of four classes, each ranked above the next by
//! more than the usage history can add, so usage orders matches within a class but never
//! across: the query is the whole name; the query runs unbroken from a word start; the query's
//! runs start the name's words in order from the first (its initials, `am` for Activity
//! Monitor); its letters are anywhere else. Initials rank by the words they skip and leave;
//! within the other classes, a prefix beats a word start, which beats a run in the middle,
//! which beats letters scattered across the name.

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
/// What initials lose for each word they skip: `am` skips AdGuard Mini's Guard.
const SKIPPED_WORD: i32 = WORD_START;
/// What initials lose for each word after their last, less than a skipped word since typing
/// more initials reaches it: `am` leaves Audio MIDI Setup's Setup.
const TRAILING_WORD: i32 = 4;
/// The distance between two classes of match: more than [`crate::history::MAX_BOOST`] plus the
/// widest gap between two matches' scores within a class, so usage orders the matches of a
/// class among themselves but never lifts one over a better class ("ar" in "Calendar" over
/// "Ardia"): an app the user can't see can't be picked, so a boost that hid it would only grow.
const CLASS: i32 = 2048;
/// The query's runs start the name's words in order from the first: "vsc" in "Visual Studio
/// Code", "ar" in "Apple Radio".
const INITIALS: i32 = CLASS;
/// The query runs unbroken from a word start: "ar" in "Ardia", "time" in "FaceTime".
const RUN_FROM_WORD_START: i32 = 2 * CLASS;
/// The query is the whole name, ignoring case, spaces and a trailing ".app": "line" for LINE,
/// whatever usage Linear has.
pub(crate) const EXACT: i32 = 3 * CLASS;

/// Matches `query` against `name`, ignoring case and whitespace in the query.
/// None when the query's characters don't all appear in order.
pub fn score(query: &str, name: &str) -> Option<Match> {
    score_prepared(&Query::new(query)?, &Name::new(name))
}

/// A query folded once, to match against many names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Query(Vec<char>);

impl Query {
    /// None for a query of only whitespace, which matches nothing.
    pub(crate) fn new(query: &str) -> Option<Self> {
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
pub(crate) struct Name {
    chars: Vec<char>,
    lower: Vec<char>,
    /// Whether a word starts at each character.
    starts: Vec<bool>,
    /// For each character, how many words start at or before it, counting only words that
    /// start with a letter or digit: a word's index plus one.
    words: Vec<i32>,
    /// `lower` without whitespace and a trailing ".app": what a query equals to match exactly.
    bare: Vec<char>,
}

impl Name {
    pub(crate) fn new(name: &str) -> Self {
        let chars: Vec<char> = name.chars().collect();
        let lower: Vec<char> = chars
            .iter()
            .map(|c| c.to_lowercase().next().unwrap_or(*c))
            .collect();
        let starts: Vec<bool> = (0..chars.len()).map(|j| starts_word(&chars, j)).collect();
        let words = (0..chars.len())
            .scan(0, |count, j| {
                *count += i32::from(starts[j] && chars[j].is_alphanumeric());
                Some(*count)
            })
            .collect();
        let stem = match lower.strip_suffix(&['.', 'a', 'p', 'p']) {
            Some(stem) if !stem.is_empty() => stem,
            _ => &lower,
        };
        let bare = stem.iter().copied().filter(|c| !c.is_whitespace()).collect();
        Self {
            chars,
            lower,
            starts,
            words,
            bare,
        }
    }
}

/// [`score`] for a query and name folded beforehand.
pub(crate) fn score_prepared(query: &Query, name: &Name) -> Option<Match> {
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
pub(crate) struct Scratch {
    best: Vec<Option<i32>>,
    next: Vec<Option<i32>>,
    back: Vec<usize>,
}

/// The score [`score_prepared`] gives, without the positions it matched, which a long list
/// only needs for the rows it shows.
pub(crate) fn score_only(query: &Query, name: &Name, scratch: &mut Scratch) -> Option<i32> {
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
    let (lower, starts) = (&name.lower, &name.starts);
    if !is_subsequence(query, lower) {
        return None;
    }

    // best[j]: the best score with the query so far matched and its last character at name
    // index j. A gap's cost grows linearly with its length, so the best earlier position is a
    // running maximum.
    let n = lower.len();
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
            let bonus = MATCH + if starts[j] { WORD_START } else { 0 };
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
    let length = LENGTH * n as i32;
    let score = if name.bare == *query {
        EXACT + total
    } else if runs_from_word_start(query, name) {
        RUN_FROM_WORD_START + total
    } else if let Some(initials) = initials(query, name, best, next) {
        // How far apart the initials are says nothing about how well they match ("am" is as
        // good a match for Activity Monitor as for Apple Music); which words they start does.
        INITIALS + initials
    } else {
        total
    };
    Some((j, score - length))
}

fn runs_from_word_start(query: &[char], name: &Name) -> bool {
    let (lower, starts) = (&name.lower, &name.starts);
    lower.len() >= query.len()
        && (0..=lower.len() - query.len())
            .any(|start| starts[start] && lower[start..start + query.len()] == *query)
}

/// When the query is the name's initials, what the words it skips and leaves after its last
/// cost, at the least: when its characters can be matched in runs that each begin a word at
/// its first letter or digit, the first run the name's first word, and each run as long as the
/// name and the query go on alike ("rightwards arrow" skips the squiggle in "Rightwards
/// squiggle arrow", however its last "s" could start that word). The buffers are spare room.
fn initials(
    query: &[char],
    name: &Name,
    cost: &mut Vec<Option<i32>>,
    next: &mut Vec<Option<i32>>,
) -> Option<i32> {
    let (chars, lower, starts, words) = (&name.chars, &name.lower, &name.starts, &name.words);
    let n = lower.len();
    let initial = |j: usize| starts[j] && chars[j].is_alphanumeric();
    if (0..n)
        .find(|&j| initial(j))
        .is_none_or(|first| lower[first] != query[0])
    {
        return None;
    }
    // cost[j]: the least the words skipped so far cost, with the query so far matched and its
    // last character at j. The word at j is the `words[j]`th, so a run beginning at j after a
    // character at k skips `words[j] - words[k] - 1` words: a running minimum of
    // `cost[k] - SKIPPED_WORD * words[k]` over the k a run may end at.
    cost.clear();
    cost.resize(n, None);
    for (i, &q) in query.iter().enumerate() {
        next.clear();
        next.resize(n, None);
        let mut earlier: Option<i32> = None;
        for j in 0..n {
            if j >= 1
                && lower[j] != q
                && let Some(c) = cost[j - 1]
            {
                let lifted = c - SKIPPED_WORD * words[j - 1];
                earlier = Some(earlier.map_or(lifted, |e| e.min(lifted)));
            }
            if lower[j] != q {
                continue;
            }
            next[j] = if i == 0 {
                (initial(j) && words[j] == 1).then_some(0)
            } else {
                let run = (j >= 1).then(|| cost[j - 1]).flatten();
                let jump = earlier
                    .filter(|_| initial(j))
                    .map(|e| e + SKIPPED_WORD * (words[j] - 1));
                match (run, jump) {
                    (Some(run), Some(jump)) => Some(run.min(jump)),
                    (run, jump) => run.or(jump),
                }
            };
        }
        std::mem::swap(cost, next);
    }
    let total = *words.last()?;
    cost.iter()
        .zip(words)
        .filter_map(|(c, &word)| c.map(|c| c + TRAILING_WORD * (total - word)))
        .min()
        .map(|cost| -cost)
}

fn is_subsequence(query: &[char], name: &[char]) -> bool {
    let mut it = name.iter();
    query.iter().all(|q| it.any(|c| c == q))
}

/// Word starts: after a separator, or a capital after a lower-case letter ("FaceTime").
fn starts_word(chars: &[char], j: usize) -> bool {
    if j == 0 {
        return true;
    }
    let (prev, here) = (chars[j - 1], chars[j]);
    !prev.is_alphanumeric()
        || (prev.is_lowercase() && here.is_uppercase())
        || (!prev.is_numeric() && here.is_numeric())
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

    /// The class a short query's match falls in: 3 exact, 2 a run from a word start, 1
    /// initials, 0 anything else.
    fn class_of(query: &str, name: &str) -> i32 {
        (score(query, name).unwrap().score + CLASS / 2).div_euclid(CLASS)
    }

    #[test]
    fn a_query_typed_from_a_word_start_outranks_every_other_match_by_a_class() {
        assert_eq!(class_of("ar", "Ardia"), 2);
        assert_eq!(class_of("time", "FaceTime"), 2);
        assert_eq!(class_of("6", "Alfred 6"), 2);
        assert_eq!(class_of("ar", "Calendar"), 0);
        assert_eq!(class_of("ar", "Apple Radio"), 1);
        assert_eq!(class_of("vsc", "Visual Studio Code"), 1);
        assert_eq!(
            rank("ar", &["Spark", "Apple Radio", "Ardia", "Calendar"]),
            ["Ardia", "Apple Radio", "Spark", "Calendar"]
        );
    }

    #[test]
    fn the_whole_name_ranks_a_class_above_a_longer_name_it_starts() {
        assert_eq!(class_of("line", "LINE"), 3);
        assert_eq!(class_of("safari", "Safari.app"), 3);
        assert_eq!(class_of("system settings", "System Settings"), 3);
        assert_eq!(class_of(".app", ".app"), 3, "a name of only .app keeps it");
        assert_eq!(class_of("line", "Linear"), 2);
        assert_eq!(class_of("pages", "Pages Creator Studio"), 2);
        assert_eq!(rank("line", &["Linear", "LINE"]), ["LINE", "Linear"]);
    }

    #[test]
    fn initials_start_the_words_in_order_from_the_first() {
        for (query, name) in [
            ("am", "Activity Monitor"),
            ("btt", "BetterTouchTool"),
            ("ipm", "iPhone Mirroring"),
            ("vscode", "Visual Studio Code"),
            ("sysset", "System Settings"),
            ("kvm", ".Karabiner-VirtualHIDDevice-Manager"),
        ] {
            assert_eq!(class_of(query, name), 1, "{query:?} in {name:?}");
        }
        assert_eq!(
            class_of("ms", "Audio MIDI Setup"),
            0,
            "the first word is left out"
        );
        assert_eq!(class_of("ms", "Microsoft Word"), 0, "the s is inside a word");
        assert_eq!(
            rank(
                "am",
                &["Games", "AdGuard Mini", "Audio MIDI Setup", "Activity Monitor"]
            ),
            ["Activity Monitor", "Audio MIDI Setup", "AdGuard Mini", "Games"],
            "initials that leave a word after them, then those that skip one, follow those that start every word"
        );
    }

    #[test]
    fn initials_rank_by_the_words_they_start_not_how_far_apart() {
        let am = |name| score("am", name).unwrap().score;
        assert_eq!(
            am("Apple Music") - am("Activity Monitor"),
            LENGTH * 5,
            "only the length differs"
        );
        assert_eq!(
            rank(
                "rightwards arrow",
                &["Rightwards squiggle arrow", "Rightwards arrow bar"]
            ),
            ["Rightwards arrow bar", "Rightwards squiggle arrow"],
            "a run goes on while the name does, so the last s doesn't start squiggle"
        );
    }

    /// Every way of matching the query, for [`initials`].
    fn naive_initials(query: &[char], name: &Name) -> Option<i32> {
        fn walk(query: &[char], name: &Name, picked: &mut Vec<usize>, best: &mut Option<i32>) {
            let Name {
                chars,
                lower,
                starts,
                words,
                ..
            } = name;
            if picked.len() == query.len() {
                let initial = |j: usize| starts[j] && chars[j].is_alphanumeric();
                let mut skipped = 0;
                for (i, &j) in picked.iter().enumerate() {
                    if i == 0 {
                        if !initial(j) || words[j] != 1 {
                            return;
                        }
                        continue;
                    }
                    let k = picked[i - 1];
                    if k + 1 == j {
                        continue;
                    }
                    if !initial(j) || lower[k + 1] == query[i] {
                        return;
                    }
                    skipped += words[j] - words[k] - 1;
                }
                let trailing = words.last().unwrap() - words[*picked.last().unwrap()];
                let value = -(SKIPPED_WORD * skipped + TRAILING_WORD * trailing);
                *best = Some(best.map_or(value, |b| b.max(value)));
                return;
            }
            let at = picked.last().map_or(0, |&k| k + 1);
            for (j, &c) in lower.iter().enumerate().skip(at) {
                if c == query[picked.len()] {
                    picked.push(j);
                    walk(query, name, picked, best);
                    picked.pop();
                }
            }
        }
        let mut best = None;
        walk(query, name, &mut Vec::new(), &mut best);
        best
    }

    /// The quadratic search the running maximum replaced, and every way of matching initials.
    fn naive(query: &str, name: &str) -> Option<Match> {
        let query = Query::new(query)?.0;
        let folded = Name::new(name);
        let Name { lower, starts, .. } = &folded;
        if !is_subsequence(&query, lower) {
            return None;
        }
        let n = lower.len();
        let mut best: Vec<Option<i32>> = vec![None; n];
        let mut from: Vec<Vec<usize>> = Vec::new();
        for (i, &q) in query.iter().enumerate() {
            let mut next = vec![None; n];
            let mut back = vec![usize::MAX; n];
            for j in 0..n {
                if lower[j] != q {
                    continue;
                }
                let bonus = MATCH + if starts[j] { WORD_START } else { 0 };
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
        let (mut j, total) = best
            .iter()
            .enumerate()
            .filter_map(|(j, s)| s.map(|s| (j, s)))
            .max_by_key(|&(j, s)| (s, std::cmp::Reverse(j)))?;
        let score = if folded.bare == query {
            EXACT + total
        } else if runs_from_word_start(&query, &folded) {
            RUN_FROM_WORD_START + total
        } else if let Some(initials) = naive_initials(&query, &folded) {
            INITIALS + initials
        } else {
            total
        } - LENGTH * n as i32;
        let mut positions = vec![0; query.len()];
        for i in (0..query.len()).rev() {
            positions[i] = j;
            if i > 0 {
                j = from[i][j];
            }
        }
        Some(Match { score, positions })
    }

    #[test]
    fn the_running_searches_score_exactly_as_trying_every_match() {
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
            "Activity Monitor",
            "Audio MIDI Setup",
            "AdGuard Mini",
            "BetterTouchTool",
            "iPhone Mirroring",
            ".Karabiner-VirtualHIDDevice-Manager",
            "AirDrop & Continuity",
            "Rightwards squiggle arrow",
        ];
        let queries = [
            "s", "saf", "ss", "aba", "abab", "ab", "vsc", "ré", "ssi", "ipi", "dupin 2", "aaa", "am", "ams",
            "agm", "btt", "ipm", "im", "kvm", "kvhm", "sys", "adc", "ac", "rsa", "rwsa",
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
