//! `filters[name](input, ctx)`: a package's matcher or sorter, which Dupin composes onto one of
//! its own sources. It sees only the candidates' titles, and the query for a matcher, and
//! answers with their positions in the batch, so the host keeps every row's identity, payload
//! and actions.

use serde::{Deserialize, Serialize};

use crate::Diagnostic;

/// The most candidates one call is handed.
pub const MAX_CANDIDATES: usize = 1000;
/// A title longer than this is cut to it, at a character boundary, before the guest sees it.
const MAX_TITLE_BYTES: usize = 512;
const MAX_POSITIONS: usize = 128;
const MAX_SCORE: i64 = 1_000_000;
/// The JSON a call may answer with: a match for every candidate, each with its positions.
const MAX_ANSWER_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterStage {
    /// Picks the candidates the query finds, with a score and the characters it matched.
    Matcher,
    /// Puts every candidate in an order.
    Sorter,
}

impl FilterStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matcher => "matcher",
            Self::Sorter => "sorter",
        }
    }
}

/// One batch of a source's candidates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterInput {
    stage: FilterStage,
    /// What was typed; a sorter is never handed it.
    query: Option<String>,
    pub(crate) titles: Vec<String>,
}

impl FilterInput {
    /// `titles` as a call takes them: at most [`MAX_CANDIDATES`], each cut to
    /// [`MAX_TITLE_BYTES`].
    pub fn new(stage: FilterStage, query: Option<String>, titles: impl IntoIterator<Item = String>) -> Self {
        let titles = titles
            .into_iter()
            .take(MAX_CANDIDATES)
            .map(|mut title| {
                if title.len() > MAX_TITLE_BYTES {
                    let mut end = MAX_TITLE_BYTES;
                    while !title.is_char_boundary(end) {
                        end -= 1;
                    }
                    title.truncate(end);
                }
                title
            })
            .collect();
        Self {
            stage,
            query: (stage == FilterStage::Matcher).then(|| query.unwrap_or_default()),
            titles,
        }
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        let candidates: Vec<_> = self
            .titles
            .iter()
            .map(|title| serde_json::json!({ "title": title }))
            .collect();
        match &self.query {
            Some(query) => serde_json::json!({ "query": query, "candidates": candidates }),
            None => serde_json::json!({ "candidates": candidates }),
        }
    }
}

/// A candidate the filter kept, by its index in the batch. A sorter's hits come in its order,
/// one for every candidate, without scores or positions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterHit {
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i32>,
    /// Character indices in the title that matched, ascending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestMatch {
    index: u64,
    #[serde(default)]
    score: Option<i64>,
    #[serde(default)]
    positions: Vec<u64>,
}

fn invalid(entry: &str, message: impl Into<String>, hint: &str) -> Diagnostic {
    Diagnostic::new("invalid-filter-answer", message)
        .file(entry)
        .hint(hint)
}

const MATCHER_HINT: &str = "Return an array of `{ index, score?, positions? }`, one for each candidate the query finds: `index` is its place in `candidates`, `score` a whole number up to 1,000,000 either way, and `positions` the ascending character indices of its title that matched.";
const SORTER_HINT: &str = "Return every index of `candidates` once, in the order to show them.";

/// Reads what `filters[name]` answered for `input`, as JSON text.
pub(crate) fn read(
    entry: &str,
    input: &FilterInput,
    json: Option<&str>,
) -> Result<Vec<FilterHit>, Diagnostic> {
    let count = input.titles.len();
    let hint = match input.stage {
        FilterStage::Matcher => MATCHER_HINT,
        FilterStage::Sorter => SORTER_HINT,
    };
    let Some(json) = json else {
        return Err(invalid(entry, "must return an array", hint));
    };
    if json.len() > MAX_ANSWER_BYTES {
        return Err(invalid(
            entry,
            format!("answered with more than {MAX_ANSWER_BYTES} bytes"),
            hint,
        ));
    }
    let mut seen = vec![false; count];
    let mut take = |index: u64| -> Result<u32, Diagnostic> {
        let slot = usize::try_from(index)
            .ok()
            .filter(|&index| index < count)
            .ok_or_else(|| invalid(entry, format!("index {index} isn't a candidate"), hint))?;
        if std::mem::replace(&mut seen[slot], true) {
            return Err(invalid(entry, format!("index {index} is answered twice"), hint));
        }
        Ok(slot as u32)
    };
    match input.stage {
        FilterStage::Sorter => {
            let order: Vec<u64> = serde_json::from_str(json)
                .map_err(|_| invalid(entry, "must return an array of indices", hint))?;
            let hits = order
                .into_iter()
                .map(|index| {
                    take(index).map(|index| FilterHit {
                        index,
                        score: None,
                        positions: Vec::new(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if hits.len() != count {
                return Err(invalid(
                    entry,
                    format!("ordered {} of {count} candidates", hits.len()),
                    hint,
                ));
            }
            Ok(hits)
        }
        FilterStage::Matcher => {
            let matches: Vec<GuestMatch> = serde_json::from_str(json)
                .map_err(|_| invalid(entry, "must return an array of matches", hint))?;
            matches
                .into_iter()
                .map(|found| {
                    let index = take(found.index)?;
                    let score = found
                        .score
                        .map(|score| {
                            (-MAX_SCORE..=MAX_SCORE)
                                .contains(&score)
                                .then_some(score as i32)
                                .ok_or_else(|| invalid(entry, format!("score {score} is out of range"), hint))
                        })
                        .transpose()?;
                    let length = input.titles[index as usize].chars().count() as u64;
                    let ascending = found.positions.windows(2).all(|pair| pair[0] < pair[1]);
                    if found.positions.len() > MAX_POSITIONS
                        || !ascending
                        || found.positions.last().is_some_and(|&last| last >= length)
                    {
                        return Err(invalid(
                            entry,
                            format!(
                                "the positions of candidate {index} aren't ascending characters of its title"
                            ),
                            hint,
                        ));
                    }
                    Ok(FilterHit {
                        index,
                        score,
                        positions: found
                            .positions
                            .into_iter()
                            .map(|position| position as u32)
                            .collect(),
                    })
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(titles: &[&str]) -> FilterInput {
        FilterInput::new(
            FilterStage::Matcher,
            Some("q".into()),
            titles.iter().map(|title| (*title).to_owned()),
        )
    }

    #[test]
    fn a_batch_is_bounded_and_a_sorter_never_sees_the_query() {
        let long = "é".repeat(MAX_TITLE_BYTES);
        let input = FilterInput::new(
            FilterStage::Sorter,
            Some("secret".into()),
            std::iter::repeat_n(long, MAX_CANDIDATES + 5),
        );
        assert_eq!(input.query, None);
        assert_eq!(input.titles.len(), MAX_CANDIDATES);
        assert_eq!(input.titles[0].len(), MAX_TITLE_BYTES);
        assert!(!input.json().as_object().unwrap().contains_key("query"));
        assert_eq!(matcher(&["a"]).json()["query"], "q");
    }

    #[test]
    fn answers_name_each_candidate_once_within_its_title() {
        let input = matcher(&["Safari", "Notes"]);
        assert_eq!(
            read(
                "main.js",
                &input,
                Some(r#"[{"index":1,"score":-3,"positions":[0,4]},{"index":0}]"#)
            )
            .unwrap(),
            [
                FilterHit {
                    index: 1,
                    score: Some(-3),
                    positions: vec![0, 4]
                },
                FilterHit {
                    index: 0,
                    score: None,
                    positions: Vec::new()
                },
            ]
        );
        for answer in [
            None,
            Some("{}"),
            Some(r#"[{"index":2}]"#),
            Some(r#"[{"index":-1}]"#),
            Some(r#"[{"index":0},{"index":0}]"#),
            Some(r#"[{"index":0,"positions":[6]}]"#),
            Some(r#"[{"index":0,"positions":[2,1]}]"#),
            Some(r#"[{"index":0,"score":1000001}]"#),
            Some(r#"[{"index":0,"title":"forged"}]"#),
            Some(r#"[{"index":0.5}]"#),
        ] {
            assert!(read("main.js", &input, answer).is_err(), "{answer:?}");
        }
        let sorter = FilterInput::new(FilterStage::Sorter, None, ["a".into(), "b".into(), "c".into()]);
        assert_eq!(
            read("main.js", &sorter, Some("[2,0,1]"))
                .unwrap()
                .iter()
                .map(|hit| hit.index)
                .collect::<Vec<_>>(),
            [2, 0, 1]
        );
        for answer in ["[0,1]", "[0,1,1]", "[0,1,2,3]", r#"[{"index":0}]"#] {
            assert!(read("main.js", &sorter, Some(answer)).is_err(), "{answer}");
        }
        let huge = format!("[{}]", "0,".repeat(MAX_ANSWER_BYTES / 2));
        assert!(read("main.js", &sorter, Some(&huge)).is_err());
    }
}
