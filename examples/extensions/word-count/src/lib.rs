//! A Dupin extension that counts the words and characters of the query, with a list of its
//! words and a statistics page as views of their own.

use dupin_plugin_protocol::{
    Effect, ErrorCode, MAX_CONTEXT_BYTES, Outcome, PluginAction, PluginItem, ProtocolError, Request,
    Response, VERSION, View,
};
use serde_json::json;

pub fn handle(bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let request = Request::decode(bytes)?;
    let outcome = match request.view.as_deref() {
        None => {
            let text = request.query.trim();
            let items = if text.is_empty() || request.limit == 0 {
                Vec::new()
            } else {
                vec![count(text)]
            };
            Outcome::Results { items, state: None }
        }
        Some(view) => match (view, viewed_text(&request)) {
            (_, None) => error(ErrorCode::InvalidQuery, "This view lost the text it counts"),
            ("words", Some(text)) => Outcome::Results {
                items: words(text, &request.query, request.limit as usize),
                state: None,
            },
            ("stats", Some(text)) => statistics(text),
            _ => error(ErrorCode::UnsupportedCommand, "Unknown view"),
        },
    };
    Response {
        version: VERSION,
        request_id: request.request_id.clone(),
        generation: request.generation.clone(),
        outcome,
    }
    .encode(&request)
}

fn error(code: ErrorCode, message: &str) -> Outcome {
    Outcome::Error {
        code,
        message: message.into(),
    }
}

/// The counted text, which the views get back from the context their push carried.
fn viewed_text(request: &Request) -> Option<&str> {
    request.context.as_ref()?.get("text")?.as_str()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn count(text: &str) -> PluginItem {
    let words = text.split_whitespace().count();
    let characters = text.chars().count();
    let result = format!(
        "{} · {}",
        plural(words, "word", "words"),
        plural(characters, "character", "characters")
    );
    let mut actions = vec![PluginAction {
        id: "copy".into(),
        title: "Copy count".into(),
        effect: Effect::Copy { text: result.clone() },
    }];
    let context = json!({ "text": text });
    // The protocol refuses an oversized context, which would fail the whole response.
    if serde_json::to_vec(&context).is_ok_and(|bytes| bytes.len() <= MAX_CONTEXT_BYTES) {
        actions.push(PluginAction {
            id: "words".into(),
            title: "List words".into(),
            effect: Effect::Push {
                view: "words".into(),
                title: "Words".into(),
                context: Some(context.clone()),
            },
        });
        actions.push(PluginAction {
            id: "stats".into(),
            title: "Show statistics".into(),
            effect: Effect::Push {
                view: "stats".into(),
                title: "Statistics".into(),
                context: Some(context),
            },
        });
    }
    if words == 1 {
        actions.push(wiktionary(text));
    }
    PluginItem {
        id: "count".into(),
        view: View::Pair {
            source: text.into(),
            result,
        },
        actions,
    }
}

/// Distinct words in order of first use, with how often each occurs, filtered by `filter`.
fn words(text: &str, filter: &str, limit: usize) -> Vec<PluginItem> {
    let mut counted: Vec<(&str, usize)> = Vec::new();
    for word in text.split_whitespace() {
        match counted.iter_mut().find(|(seen, _)| *seen == word) {
            Some((_, n)) => *n += 1,
            None => counted.push((word, 1)),
        }
    }
    let filter = filter.trim().to_lowercase();
    counted
        .into_iter()
        .enumerate()
        .filter(|(_, (word, _))| word.to_lowercase().contains(&filter))
        .take(limit)
        .map(|(index, (word, n))| PluginItem {
            id: format!("word-{index}"),
            view: View::Text {
                title: word.into(),
                subtitle: plural(n, "time", "times"),
            },
            actions: vec![
                PluginAction {
                    id: "copy".into(),
                    title: "Copy word".into(),
                    effect: Effect::Copy { text: word.into() },
                },
                wiktionary(word),
            ],
        })
        .collect()
}

fn statistics(text: &str) -> Outcome {
    let words: Vec<&str> = text.split_whitespace().collect();
    let characters = text.chars().count();
    let letters = text.chars().filter(|ch| !ch.is_whitespace()).count();
    // `max_by_key` keeps the last of equals; reversing makes ties go to the first word.
    let longest = words
        .iter()
        .rev()
        .max_by_key(|word| word.chars().count())
        .copied()
        .unwrap_or("");
    let average = if words.is_empty() {
        0.0
    } else {
        letters as f64 / words.len() as f64
    };
    let body = format!(
        "Words: {}\nCharacters: {characters}\nCharacters excluding spaces: {letters}\nLongest word: {longest}\nAverage word length: {average:.1}",
        words.len()
    );
    Outcome::Detail {
        title: plural(words.len(), "word", "words"),
        actions: vec![PluginAction {
            id: "copy".into(),
            title: "Copy statistics".into(),
            effect: Effect::Copy { text: body.clone() },
        }],
        body,
    }
}

fn wiktionary(word: &str) -> PluginAction {
    PluginAction {
        id: "define".into(),
        title: "Look up in Wiktionary".into(),
        effect: Effect::OpenUrl {
            url: format!("https://en.wiktionary.org/wiki/{}", encode(word)),
        },
    }
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

// The ABI Dupin calls; see docs/extensions.md. Buffers live until the host drops this
// request's instance, so nothing is freed here.
#[cfg(target_arch = "wasm32")]
mod abi {
    use dupin_plugin_protocol::{MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES};

    #[unsafe(no_mangle)]
    pub extern "C" fn dupin_abi_version() -> i32 {
        1
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn dupin_alloc(len: i32) -> i32 {
        if len <= 0 || len as usize > MAX_RESPONSE_BYTES {
            return 0;
        }
        Box::into_raw(vec![0_u8; len as usize].into_boxed_slice()) as *mut u8 as i32
    }

    /// # Safety
    /// The host passes disjoint buffers from `dupin_alloc` with these lengths.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn dupin_query(input: i32, len: i32, output: i32, cap: i32) -> i32 {
        if input <= 0 || output <= 0 || len <= 0 || len as usize > MAX_REQUEST_BYTES || cap <= 0 {
            return -1;
        }
        // SAFETY: the host keeps both buffers alive for this call.
        let bytes = unsafe { std::slice::from_raw_parts(input as *const u8, len as usize) };
        let Ok(response) = super::handle(bytes) else {
            return -2;
        };
        if response.len() > cap as usize {
            return -3;
        }
        // SAFETY: the output buffer holds `cap` bytes, at least the response's length.
        unsafe { std::ptr::copy_nonoverlapping(response.as_ptr(), output as *mut u8, response.len()) };
        response.len() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask_view(query: &str, view: Option<(&str, serde_json::Value)>) -> Outcome {
        let request = Request {
            version: VERSION,
            request_id: "1".into(),
            generation: "1".into(),
            command: "word-count".into(),
            query: query.into(),
            limit: 10,
            state: None,
            view: view.as_ref().map(|(view, _)| (*view).into()),
            context: view.map(|(_, context)| context),
        };
        Response::decode(&handle(&request.encode().unwrap()).unwrap(), &request)
            .unwrap()
            .outcome
    }

    fn ask(query: &str) -> Vec<PluginItem> {
        let Outcome::Results { items, .. } = ask_view(query, None) else {
            panic!("expected results");
        };
        items
    }

    #[test]
    fn counts_words_and_characters() {
        let items = ask("  two words ");
        assert_eq!(
            items[0].view,
            View::Pair {
                source: "two words".into(),
                result: "2 words · 9 characters".into()
            }
        );
        assert_eq!(
            items[0]
                .actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            ["copy", "words", "stats"]
        );
        assert!(ask(" ").is_empty());
    }

    #[test]
    fn a_single_word_links_to_wiktionary() {
        let items = ask("café");
        assert_eq!(
            items[0].actions[3].effect,
            Effect::OpenUrl {
                url: "https://en.wiktionary.org/wiki/caf%C3%A9".into()
            }
        );
    }

    #[test]
    fn views_get_the_counted_text_back_from_their_context() {
        let items = ask("the cat and the hat");
        let Effect::Push { view, context, .. } = &items[0].actions[1].effect else {
            panic!("expected a push");
        };
        let context = context.clone().unwrap();
        let Outcome::Results { items: words, .. } = ask_view("", Some((view, context.clone()))) else {
            panic!("expected a list");
        };
        let titles: Vec<_> = words
            .iter()
            .map(|item| match &item.view {
                View::Text { title, subtitle } => format!("{title} {subtitle}"),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(titles, ["the 2 times", "cat 1 time", "and 1 time", "hat 1 time"]);
        let Outcome::Results { items: filtered, .. } = ask_view("AT", Some(("words", context.clone())))
        else {
            panic!("expected a list");
        };
        assert_eq!(
            filtered.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
            [words[1].id.as_str(), words[3].id.as_str()]
        );

        let Outcome::Detail { title, body, .. } = ask_view("", Some(("stats", context.clone()))) else {
            panic!("expected a detail");
        };
        assert_eq!(title, "5 words");
        assert!(body.contains("Characters: 19") && body.contains("Longest word: the"));

        assert!(matches!(
            ask_view("", Some(("missing", context))),
            Outcome::Error {
                code: ErrorCode::UnsupportedCommand,
                ..
            }
        ));
        assert!(matches!(
            ask_view("", Some(("words", json!({})))),
            Outcome::Error {
                code: ErrorCode::InvalidQuery,
                ..
            }
        ));
    }

    #[test]
    fn text_too_long_for_a_context_offers_no_views() {
        let items = ask(&"a\u{1} ".repeat(1000));
        assert_eq!(items[0].actions.len(), 1);
    }
}
