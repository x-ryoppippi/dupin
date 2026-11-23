//! A Dupin extension that counts the words and characters of the query.

use dupin_plugin_protocol::{
    Effect, Outcome, PluginAction, PluginItem, ProtocolError, Request, Response, VERSION, View,
};

pub fn handle(bytes: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let request = Request::decode(bytes)?;
    let text = request.query.trim();
    let items = if text.is_empty() || request.limit == 0 {
        Vec::new()
    } else {
        vec![count(text)]
    };
    Response {
        version: VERSION,
        request_id: request.request_id.clone(),
        generation: request.generation.clone(),
        outcome: Outcome::Results { items, state: None },
    }
    .encode(&request)
}

fn count(text: &str) -> PluginItem {
    let words = text.split_whitespace().count();
    let characters = text.chars().count();
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
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
    if words == 1 {
        actions.push(PluginAction {
            id: "define".into(),
            title: "Look up in Wiktionary".into(),
            effect: Effect::OpenUrl {
                url: format!("https://en.wiktionary.org/wiki/{}", encode(text)),
            },
        });
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

    fn ask(query: &str) -> Vec<PluginItem> {
        let request = Request {
            version: VERSION,
            request_id: "1".into(),
            generation: "1".into(),
            command: "word-count".into(),
            query: query.into(),
            limit: 10,
            state: None,
        };
        let response = Response::decode(&handle(&request.encode().unwrap()).unwrap(), &request).unwrap();
        let Outcome::Results { items, .. } = response.outcome else {
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
        assert_eq!(items[0].actions.len(), 1);
        assert!(ask(" ").is_empty());
    }

    #[test]
    fn a_single_word_links_to_wiktionary() {
        let items = ask("café");
        assert_eq!(
            items[0].actions[1].effect,
            Effect::OpenUrl {
                url: "https://en.wiktionary.org/wiki/caf%C3%A9".into()
            }
        );
    }
}
