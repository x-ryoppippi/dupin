//! Text snippets from config.toml: which ones are usable, and how their placeholders are
//! filled in when one is used. The bundled `snippets` extension lists them.

use crate::config::Config;

const MAX_KEYWORD_BYTES: usize = 32;
const MAX_NAME_CHARS: usize = 80;
const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Every snippet's text together: the package gets them all with each catalogue it is asked
/// for, and Dupin holds each text once for copying and again for pasting.
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

/// What the bundled `snippets` extension reads as `ctx.preferences`: the usable snippets in
/// keyword order, and whether Enter pastes them. Dupin hands over the settings rather than the
/// package reading config.toml, so it checks them here and the package lists what it gets.
pub fn preferences(config: &Config) -> (serde_json::Map<String, serde_json::Value>, Option<String>) {
    let mut listed = Vec::new();
    let (mut invalid, mut over) = (false, false);
    let mut total = 0;
    for (keyword, snippet) in &config.snippets {
        let name = snippet.name.as_deref();
        if !valid_keyword(keyword) || !name.is_none_or(valid_name) || !valid_text(&snippet.text) {
            invalid = true;
            continue;
        }
        if total + snippet.text.len() > MAX_TOTAL_TEXT_BYTES {
            over = true;
            continue;
        }
        total += snippet.text.len();
        listed.push(serde_json::json!({ "keyword": keyword, "name": name, "text": snippet.text }));
    }
    let notice = match (invalid, over) {
        (false, false) => None,
        (true, _) => Some(
            "Some snippets were ignored because a keyword, name or text is invalid. Review snippets in config.toml."
                .to_owned(),
        ),
        (false, true) => Some(
            "Some snippets were ignored because together they hold more than 1 MiB of text. Review snippets in config.toml."
                .to_owned(),
        ),
    };
    let mut preferences = serde_json::Map::new();
    preferences.insert("snippets".into(), listed.into());
    preferences.insert("paste".into(), config.snippet_paste.into());
    (preferences, notice)
}

fn valid_keyword(keyword: &str) -> bool {
    !keyword.is_empty()
        && keyword.len() <= MAX_KEYWORD_BYTES
        && !keyword.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= MAX_NAME_CHARS && !name.chars().any(char::is_control)
}

fn valid_text(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_TEXT_BYTES
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::Snippet;

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
        let (preferences, notice) = preferences(&config);
        assert!(notice.unwrap().contains("snippets"));
        assert_eq!(preferences["snippets"].as_array().unwrap().len(), 3);
        assert!(super::preferences(&self::config(false)).1.is_none());
    }

    #[test]
    fn preferences_hand_over_the_valid_snippets_in_keyword_order() {
        let mut config = config(true);
        config.snippets.insert("two words".into(), snippet(None, "x"));
        let (preferences, notice) = preferences(&config);
        assert!(notice.unwrap().contains("invalid"));
        assert_eq!(
            serde_json::Value::Object(preferences),
            serde_json::json!({
                "snippets": [
                    { "keyword": "addr", "name": "Home address", "text": "1 Baker Street" },
                    { "keyword": "sig", "name": "Email signature", "text": "Best wishes,\n  Dupin" },
                    { "keyword": "today", "name": null, "text": "{date}" },
                ],
                "paste": true,
            })
        );
        assert_eq!(
            serde_json::Value::Object(super::preferences(&Config::default()).0),
            serde_json::json!({ "snippets": [], "paste": false })
        );
    }

    #[test]
    fn preferences_leave_out_what_goes_past_the_text_budget() {
        let big = "x".repeat(MAX_TEXT_BYTES);
        let mut config = Config::default();
        for index in 0..MAX_TOTAL_TEXT_BYTES / MAX_TEXT_BYTES + 1 {
            config
                .snippets
                .insert(format!("big{index:02}"), snippet(None, &big));
        }
        config.snippets.insert("small".into(), snippet(None, "fits"));
        let (preferences, notice) = preferences(&config);
        assert!(notice.unwrap().contains("1 MiB"));
        let listed = preferences["snippets"].as_array().unwrap();
        assert_eq!(listed.len(), MAX_TOTAL_TEXT_BYTES / MAX_TEXT_BYTES);
        assert_eq!(listed.last().unwrap()["keyword"], "big15");
        assert!(listed.iter().all(|snippet| snippet["keyword"] != "small"));
    }
}
