//! Text snippets, from config.toml and from Dupin's encrypted storage: which ones are usable,
//! what stops one from being saved, and how their placeholders are filled in when one is used.
//! The bundled `snippets` extension lists them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::Snippet as ConfigSnippet;

const MAX_KEYWORD_BYTES: usize = 32;
const MAX_NAME_CHARS: usize = 80;
const MAX_TEXT_BYTES: usize = 64 * 1024;
/// As many rows as one extension's catalogue holds.
pub const MAX_SNIPPETS: usize = 10_000;
/// Every snippet's keyword, name and text together: the package gets them all with each
/// catalogue it is asked for, and Dupin holds each text once for copying and again for
/// pasting. Within this and [`MAX_SNIPPETS`], the rows they become fit in one catalogue.
const MAX_TOTAL_TEXT_BYTES: usize = 1024 * 1024;

/// The wall-clock time where the user is, as placeholders show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placeholder {
    Date,
    Time,
    DateTime,
    Clipboard,
}

const CLIPBOARD_PLACEHOLDER: &str = "{clipboard}";
const PLACEHOLDERS: [(&str, Placeholder); 4] = [
    ("{date}", Placeholder::Date),
    ("{time}", Placeholder::Time),
    ("{datetime}", Placeholder::DateTime),
    (CLIPBOARD_PLACEHOLDER, Placeholder::Clipboard),
];

/// The next placeholder at or after `from`: its byte range and kind. Braces that don't spell
/// one are ordinary text.
fn next_placeholder(text: &str, from: usize) -> Option<(usize, usize, Placeholder)> {
    text[from..].match_indices('{').find_map(|(offset, _)| {
        let start = from + offset;
        PLACEHOLDERS
            .iter()
            .find(|(token, _)| text[start..].starts_with(token))
            .map(|(token, kind)| (start, start + token.len(), *kind))
    })
}

/// Whether the text holds a placeholder, so what it becomes depends on when it is used.
pub fn has_placeholders(text: &str) -> bool {
    next_placeholder(text, 0).is_some()
}

/// Whether expanding the snippet requires clipboard text.
pub fn needs_clipboard(text: &str) -> bool {
    text.contains(CLIPBOARD_PLACEHOLDER)
}

/// Fill the placeholders using the activation time and any clipboard text already read.
pub fn expand(text: &str, now: LocalTime, clipboard: &str) -> String {
    let date = format!("{:04}-{:02}-{:02}", now.year, now.month, now.day);
    let time = format!("{:02}:{:02}", now.hour, now.minute);
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some((start, end, kind)) = next_placeholder(text, from) {
        out.push_str(&text[from..start]);
        match kind {
            Placeholder::Date => out.push_str(&date),
            Placeholder::Time => out.push_str(&time),
            Placeholder::DateTime => {
                out.push_str(&date);
                out.push(' ');
                out.push_str(&time);
            }
            Placeholder::Clipboard => out.push_str(clipboard),
        }
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

/// A snippet the user stored in Dupin, as its record holds it. A field a later version adds is
/// ignored, so its records still list here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub keyword: String,
    #[serde(default)]
    pub name: Option<String>,
    pub text: String,
}

impl Snippet {
    fn size(&self) -> usize {
        self.keyword.len() + self.name.as_ref().map_or(0, String::len) + self.text.len()
    }

    fn valid(&self) -> bool {
        keyword_problem(&self.keyword).is_none()
            && self
                .name
                .as_deref()
                .is_none_or(|name| name_problem(name).is_none())
            && text_problem(&self.text).is_none()
    }

    /// The snippet as it is stored: its name trimmed, and none when blank.
    pub fn normalized(mut self) -> Self {
        self.name = self
            .name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        self
    }
}

/// Where a listed snippet comes from: config.toml, which only its own file changes, or Dupin's
/// encrypted storage, which Settings and the launcher edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Config,
    Stored,
}

/// What is wrong with a snippet the user is saving, by field, as a form shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Problems {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Problems {
    pub fn is_empty(&self) -> bool {
        self.keyword.is_none() && self.name.is_none() && self.text.is_none()
    }
}

/// The snippets Dupin lists, in keyword order, and why any were left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub snippets: Vec<(Snippet, Source)>,
    pub notices: Vec<String>,
}

/// config.toml's `declared` snippets and the `stored` ones as one list in keyword order. A
/// keyword both hold is config.toml's, the declared base that a stored snippet never silently
/// replaces, and a notice names it. Invalid snippets, and those past [`MAX_SNIPPETS`] or the
/// text budget in keyword order, are left out with a notice.
pub fn listing(declared: &BTreeMap<String, ConfigSnippet>, stored: &[Snippet]) -> Listing {
    let mut merged: BTreeMap<String, (Snippet, Source)> = BTreeMap::new();
    let (mut invalid_declared, mut invalid_stored) = (false, false);
    for snippet in stored {
        if snippet.valid() {
            merged.insert(snippet.keyword.clone(), (snippet.clone(), Source::Stored));
        } else {
            invalid_stored = true;
        }
    }
    let mut shadowed = Vec::new();
    for (keyword, snippet) in declared {
        let snippet = Snippet {
            keyword: keyword.clone(),
            name: snippet.name.clone(),
            text: snippet.text.clone(),
        };
        if !snippet.valid() {
            invalid_declared = true;
            continue;
        }
        if merged
            .insert(keyword.clone(), (snippet, Source::Config))
            .is_some()
        {
            shadowed.push(format!("`{keyword}`"));
        }
    }
    let mut listing = Listing::default();
    let (mut too_many, mut over) = (false, false);
    let mut total = 0;
    for (snippet, source) in merged.into_values() {
        if listing.snippets.len() == MAX_SNIPPETS {
            too_many = true;
            continue;
        }
        let size = snippet.size();
        if total + size > MAX_TOTAL_TEXT_BYTES {
            over = true;
            continue;
        }
        total += size;
        listing.snippets.push((snippet, source));
    }
    let ignored =
        |reason: &str, review: &str| format!("Some snippets were ignored because {reason}. {review}");
    if invalid_declared {
        listing.notices.push(ignored(
            "a keyword, name or text is invalid",
            "Review snippets in config.toml.",
        ));
    }
    if invalid_stored {
        listing.notices.push(ignored(
            "a stored keyword, name or text is invalid",
            "Review them in Settings › Snippets.",
        ));
    }
    if too_many {
        listing.notices.push(ignored(
            "there are more than 10,000",
            "Delete some in Settings › Snippets.",
        ));
    }
    if over {
        listing.notices.push(ignored(
            "together they hold more than 1 MiB of text",
            "Delete some in Settings › Snippets.",
        ));
    }
    if !shadowed.is_empty() {
        let more = shadowed.len().saturating_sub(3);
        shadowed.truncate(3);
        let mut named = shadowed.join(", ");
        if more > 0 {
            named.push_str(&format!(" and {more} more"));
        }
        listing.notices.push(format!(
            "config.toml's snippets take the place of stored ones with the same keyword: {named}. Rename the stored ones in Settings › Snippets."
        ));
    }
    listing
}

/// What the bundled `snippets` extension reads as `ctx.preferences`: the usable snippets in
/// keyword order, each with where it comes from, and whether Enter pastes them. Dupin hands
/// over the snippets rather than the package reading config.toml or storage, so it checks
/// them here and the package lists what it gets.
pub fn preferences(
    declared: &BTreeMap<String, ConfigSnippet>,
    stored: &[Snippet],
    paste: bool,
) -> (serde_json::Map<String, serde_json::Value>, Option<String>) {
    let listing = listing(declared, stored);
    let listed: Vec<serde_json::Value> = listing
        .snippets
        .iter()
        .map(|(snippet, source)| {
            serde_json::json!({
                "keyword": snippet.keyword,
                "name": snippet.name,
                "text": snippet.text,
                "source": source,
            })
        })
        .collect();
    let mut preferences = serde_json::Map::new();
    preferences.insert("snippets".into(), listed.into());
    preferences.insert("paste".into(), paste.into());
    let notice = (!listing.notices.is_empty()).then(|| listing.notices.join("\n"));
    (preferences, notice)
}

/// What stops `draft`, already [`Snippet::normalized`], from being stored in place of the
/// stored snippet `replaces`, or as a new one without it, beside config.toml's `declared`
/// snippets and the `stored` ones: an invalid field, a keyword another snippet has, a snippet
/// no longer there to replace, or going past what Dupin lists.
pub fn check_save(
    declared: &BTreeMap<String, ConfigSnippet>,
    stored: &[Snippet],
    draft: &Snippet,
    replaces: Option<&str>,
) -> Problems {
    if let Some(replaced) = replaces
        && !stored.iter().any(|snippet| snippet.keyword == replaced)
    {
        return Problems {
            keyword: Some("The snippet you were editing is no longer stored".into()),
            ..Problems::default()
        };
    }
    let mut problems = Problems {
        keyword: keyword_problem(&draft.keyword),
        name: draft.name.as_deref().and_then(name_problem),
        text: text_problem(&draft.text),
    };
    if problems.keyword.is_none() {
        if declared.contains_key(&draft.keyword) {
            problems.keyword = Some("config.toml already has a snippet with this keyword".into());
        } else if replaces != Some(draft.keyword.as_str())
            && stored.iter().any(|snippet| snippet.keyword == draft.keyword)
        {
            problems.keyword = Some("Another snippet has this keyword".into());
        }
    }
    if !problems.is_empty() {
        return problems;
    }
    let declared_total: usize = declared
        .iter()
        .map(|(keyword, snippet)| {
            keyword.len() + snippet.name.as_ref().map_or(0, String::len) + snippet.text.len()
        })
        .sum();
    let (count, total) = stored
        .iter()
        .filter(|snippet| Some(snippet.keyword.as_str()) != replaces)
        .fold(
            (declared.len() + 1, declared_total + draft.size()),
            |(count, total), snippet| (count + 1, total + snippet.size()),
        );
    if count > MAX_SNIPPETS {
        problems.keyword = Some("Dupin keeps at most 10,000 snippets; delete one first".into());
    } else if total > MAX_TOTAL_TEXT_BYTES {
        problems.text =
            Some("Snippets together hold at most 1 MiB of text; shorten this or delete another".into());
    }
    problems
}

fn keyword_problem(keyword: &str) -> Option<String> {
    if keyword.is_empty() {
        Some("Enter a keyword".into())
    } else if keyword.len() > MAX_KEYWORD_BYTES {
        Some(format!("A keyword is at most {MAX_KEYWORD_BYTES} bytes"))
    } else if keyword.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Some("A keyword has no spaces".into())
    } else {
        None
    }
}

fn name_problem(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        Some("A name needs more than spaces".into())
    } else if name.chars().count() > MAX_NAME_CHARS {
        Some(format!("A name is at most {MAX_NAME_CHARS} characters"))
    } else if name.chars().any(char::is_control) {
        Some("A name is one line".into())
    } else {
        None
    }
}

fn text_problem(text: &str) -> Option<String> {
    if text.is_empty() {
        Some("Enter the text".into())
    } else if text.len() > MAX_TEXT_BYTES {
        Some("A snippet's text is at most 64 KiB".into())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::{Config, Snippet};

    const NOW: LocalTime = LocalTime {
        year: 2026,
        month: 1,
        day: 31,
        hour: 9,
        minute: 5,
    };

    fn snippet(name: Option<&str>, text: &str) -> Snippet {
        Snippet {
            name: name.map(str::to_owned),
            text: text.into(),
        }
    }

    fn config(paste: bool) -> Config {
        Config {
            snippets: BTreeMap::from([
                (
                    "sig".into(),
                    snippet(Some("Email signature"), "Best wishes,\n  Dupin"),
                ),
                ("addr".into(), snippet(Some("Home address"), "1 Baker Street")),
                ("today".into(), snippet(None, "{date}")),
            ]),
            snippet_paste: paste,
            ..Config::default()
        }
    }

    #[test]
    fn fills_in_date_and_time_placeholders() {
        assert_eq!(expand("{date}", NOW, ""), "2026-01-31");
        assert_eq!(expand("at {time}", NOW, ""), "at 09:05");
        assert_eq!(expand("{datetime}.", NOW, ""), "2026-01-31 09:05.");
        let year = LocalTime { year: 987, ..NOW };
        assert_eq!(expand("{date}", year, ""), "0987-01-31");
    }

    #[test]
    fn clipboard_placeholders_share_the_same_text() {
        let template = "{clipboard} and {{clipboard}}";
        assert!(needs_clipboard(template));
        assert!(!needs_clipboard("fn main() {} {date} {Clipboard} {clipboard"));
        assert_eq!(expand(template, NOW, "pasted"), "pasted and {pasted}");
        assert_eq!(expand("[{clipboard}]", NOW, ""), "[]");
    }

    #[test]
    fn other_braces_stay_as_written() {
        assert_eq!(expand("fn main() {}", NOW, ""), "fn main() {}");
        assert_eq!(expand("{{date}}", NOW, ""), "{2026-01-31}");
        assert_eq!(expand("{Date} {date", NOW, ""), "{Date} {date");
        assert_eq!(expand("日本{time}語", NOW, ""), "日本09:05語");
        assert_eq!(expand("", NOW, ""), "");
    }

    #[test]
    fn invalid_snippets_are_skipped_with_a_notice() {
        let mut config = config(false);
        config.snippets.insert("two words".into(), snippet(None, "x"));
        config.snippets.insert("empty".into(), snippet(None, ""));
        config.snippets.insert("blank".into(), snippet(Some("  "), "x"));
        config
            .snippets
            .insert("huge".into(), snippet(None, &"x".repeat(MAX_TEXT_BYTES + 1)));
        let (preferences, notice) = super::preferences(&config.snippets, &[], false);
        assert!(notice.unwrap().contains("config.toml"));
        assert_eq!(preferences["snippets"].as_array().unwrap().len(), 3);
        assert!(
            super::preferences(&self::config(false).snippets, &[], false)
                .1
                .is_none()
        );

        let broken = stored("two words", None, "x");
        let (preferences, notice) = super::preferences(&BTreeMap::new(), &[broken], false);
        assert!(notice.unwrap().contains("Settings › Snippets"));
        assert!(preferences["snippets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn preferences_hand_over_the_valid_snippets_in_keyword_order() {
        let mut config = config(true);
        config.snippets.insert("two words".into(), snippet(None, "x"));
        let (preferences, notice) = super::preferences(
            &config.snippets,
            &[stored("note", Some("Meeting note"), "Notes for {date}")],
            config.snippet_paste,
        );
        assert!(notice.unwrap().contains("invalid"));
        assert_eq!(
            serde_json::Value::Object(preferences),
            serde_json::json!({
                "snippets": [
                    { "keyword": "addr", "name": "Home address", "text": "1 Baker Street", "source": "config" },
                    { "keyword": "note", "name": "Meeting note", "text": "Notes for {date}", "source": "stored" },
                    { "keyword": "sig", "name": "Email signature", "text": "Best wishes,\n  Dupin", "source": "config" },
                    { "keyword": "today", "name": null, "text": "{date}", "source": "config" },
                ],
                "paste": true,
            })
        );
        assert_eq!(
            serde_json::Value::Object(super::preferences(&BTreeMap::new(), &[], false).0),
            serde_json::json!({ "snippets": [], "paste": false })
        );
    }

    #[test]
    fn config_toml_wins_a_keyword_both_hold_and_says_so() {
        let config = config(false);
        let listing = listing(
            &config.snippets,
            &[
                stored("sig", Some("Stored signature"), "Cheers"),
                stored("zip", None, "NW1 6XE"),
            ],
        );
        let sources: Vec<_> = listing
            .snippets
            .iter()
            .map(|(snippet, source)| (snippet.keyword.as_str(), snippet.text.as_str(), *source))
            .collect();
        assert_eq!(
            sources,
            [
                ("addr", "1 Baker Street", Source::Config),
                ("sig", "Best wishes,\n  Dupin", Source::Config),
                ("today", "{date}", Source::Config),
                ("zip", "NW1 6XE", Source::Stored),
            ]
        );
        assert_eq!(listing.notices.len(), 1);
        assert!(listing.notices[0].contains("`sig`"), "{:?}", listing.notices);
    }

    #[test]
    fn preferences_leave_out_what_goes_past_the_text_budget() {
        let big = "x".repeat(MAX_TEXT_BYTES - "big00".len());
        let mut declared = BTreeMap::new();
        for index in 0..MAX_TOTAL_TEXT_BYTES / MAX_TEXT_BYTES + 1 {
            declared.insert(format!("big{index:02}"), snippet(None, &big));
        }
        let (preferences, notice) =
            super::preferences(&declared, &[stored("named", Some("Named"), "x")], false);
        assert!(notice.unwrap().contains("1 MiB"));
        let listed = preferences["snippets"].as_array().unwrap();
        assert_eq!(listed.len(), MAX_TOTAL_TEXT_BYTES / MAX_TEXT_BYTES);
        assert_eq!(listed.last().unwrap()["keyword"], "big15");
        assert!(
            listed.iter().all(|snippet| snippet["keyword"] != "named"),
            "a keyword and name count toward the budget too"
        );
    }

    #[test]
    fn preferences_leave_out_snippets_past_what_a_catalogue_holds() {
        let declared: BTreeMap<_, _> = (0..MAX_SNIPPETS)
            .map(|index| (format!("s{index:05}"), snippet(None, "x")))
            .collect();
        let (preferences, notice) = super::preferences(&declared, &[stored("t", None, "x")], false);
        assert!(notice.unwrap().contains("more than 10,000"));
        let listed = preferences["snippets"].as_array().unwrap();
        assert_eq!(listed.len(), MAX_SNIPPETS);
        assert_eq!(listed.last().unwrap()["keyword"], "s09999");
    }

    fn stored(keyword: &str, name: Option<&str>, text: &str) -> super::Snippet {
        super::Snippet {
            keyword: keyword.into(),
            name: name.map(str::to_owned),
            text: text.into(),
        }
    }

    #[test]
    fn saving_checks_each_field_as_config_toml_does() {
        let declared = BTreeMap::new();
        let check = |draft: super::Snippet| check_save(&declared, &[], &draft.normalized(), None);
        assert!(check(stored("sig", Some("  Signature "), "x")).is_empty());
        assert_eq!(
            stored("sig", Some("  Signature "), "x")
                .normalized()
                .name
                .as_deref(),
            Some("Signature")
        );
        assert_eq!(stored("sig", Some("   "), "x").normalized().name, None);
        assert_eq!(
            check(stored("", None, "x")).keyword.as_deref(),
            Some("Enter a keyword")
        );
        assert!(check(stored("two words", None, "x")).keyword.is_some());
        assert!(
            check(stored(&"k".repeat(MAX_KEYWORD_BYTES + 1), None, "x"))
                .keyword
                .is_some()
        );
        assert!(check(stored("k", Some("a\nb"), "x")).name.is_some());
        assert!(
            check(stored("k", Some(&"n".repeat(MAX_NAME_CHARS + 1)), "x"))
                .name
                .is_some()
        );
        assert_eq!(
            check(stored("k", None, "")).text.as_deref(),
            Some("Enter the text")
        );
        assert!(
            check(stored("k", None, &"x".repeat(MAX_TEXT_BYTES + 1)))
                .text
                .is_some()
        );
        let problems = check(stored("two words", None, ""));
        assert_eq!(
            serde_json::to_value(problems).unwrap(),
            serde_json::json!({ "keyword": "A keyword has no spaces", "text": "Enter the text" }),
            "each field's problem, as a form shows it, and no field without one"
        );
    }

    #[test]
    fn saving_refuses_a_keyword_another_snippet_has() {
        let config = config(false);
        let library = [stored("note", None, "x"), stored("todo", None, "y")];
        let save = |draft: super::Snippet, replaces: Option<&str>| {
            check_save(&config.snippets, &library, &draft, replaces).keyword
        };
        assert_eq!(
            save(stored("sig", None, "x"), None).as_deref(),
            Some("config.toml already has a snippet with this keyword")
        );
        assert_eq!(
            save(stored("todo", None, "x"), None).as_deref(),
            Some("Another snippet has this keyword")
        );
        assert_eq!(
            save(stored("todo", None, "x"), Some("note")).as_deref(),
            Some("Another snippet has this keyword"),
            "renaming onto another snippet's keyword"
        );
        assert_eq!(save(stored("note", None, "changed"), Some("note")), None);
        assert_eq!(save(stored("memo", None, "renamed"), Some("note")), None);
        assert_eq!(
            save(stored("memo", None, "x"), Some("sig")).as_deref(),
            Some("The snippet you were editing is no longer stored"),
            "config.toml's snippets are never edited as stored ones"
        );
        assert!(save(stored("memo", None, "x"), Some("gone")).is_some());
    }

    #[test]
    fn saving_keeps_within_what_dupin_lists() {
        let declared: BTreeMap<_, _> = (0..MAX_SNIPPETS - 1)
            .map(|index| (format!("s{index:05}"), snippet(None, "x")))
            .collect();
        let library = [stored("last", None, "x")];
        assert!(
            check_save(&declared, &library, &stored("more", None, "x"), None)
                .keyword
                .unwrap()
                .contains("10,000")
        );
        assert!(check_save(&declared, &library, &stored("lost", None, "y"), Some("last")).is_empty());

        let big = "x".repeat(MAX_TEXT_BYTES - 8);
        let library: Vec<_> = (0..MAX_TOTAL_TEXT_BYTES / MAX_TEXT_BYTES)
            .map(|index| stored(&format!("big{index:02}"), None, &big))
            .collect();
        let problems = check_save(&BTreeMap::new(), &library, &stored("more", None, &big), None);
        assert!(problems.text.unwrap().contains("1 MiB"));
        assert!(
            check_save(
                &BTreeMap::new(),
                &library,
                &stored("big00", None, "short"),
                Some("big00")
            )
            .is_empty(),
            "replacing a snippet frees its text"
        );
    }
}
