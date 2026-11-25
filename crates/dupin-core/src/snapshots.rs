use crate::View;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// How many snapshots can be kept.
pub const MAX_ENTRIES: usize = 100;
/// How many bytes of snapshots, as JSON, can be kept in all.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// How large one snapshot can be, as JSON.
pub const MAX_ENTRY_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    pub id: String,
    pub saved_at: u64,
    pub mode: String,
    pub explicit: bool,
    pub query: String,
    pub title: String,
    pub views: Vec<View>,
    pub state: Option<dupin_plugin_protocol::State>,
}

impl Snapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.title.is_empty()
            || self.title.len() > 4096
            || self.query.len() > 4096
            || crate::CommandId::new(self.mode.clone()).is_err()
            || self.views.is_empty()
            || self.views.len() > 500
            || serde_json::to_vec(self)
                .map_err(|_| "Cannot encode snapshot.")?
                .len()
                > MAX_ENTRY_BYTES
        {
            return Err("Snapshot exceeds the supported size or schema.".into());
        }
        if let Some(state) = &self.state
            && (state.version == 0 || serde_json::to_vec(state).map_err(|_| "Invalid state.")?.len() > 8192)
        {
            return Err("Snapshot state exceeds the supported size or schema.".into());
        }
        if self.id.len() > 128 {
            return Err("Snapshot identifier is invalid.".into());
        }
        Ok(())
    }

    fn content(&self) -> String {
        self.views
            .iter()
            .map(|v| match v {
                View::Text { title, subtitle } => format!("{title}\n{subtitle}"),
                View::Detail { title, body } => format!("{title}\n{body}"),
                View::Pair { source, result } => format!("{source}\n{result}"),
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn search_text(&self) -> String {
        normalize(&format!("{}\n{}\n{}", self.title, self.query, self.content()))
    }
}

pub fn normalize(s: &str) -> String {
    s.nfkc().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(snapshot: &Snapshot, query: &str) -> bool {
        snapshot.search_text().contains(&normalize(query))
    }

    fn fixture() -> Snapshot {
        Snapshot {
            version: 1,
            id: String::new(),
            saved_at: 0,
            mode: "sample.search".into(),
            explicit: false,
            query: "query".into(),
            title: "Café 東京".into(),
            views: vec![
                View::Text {
                    title: "row".into(),
                    subtitle: "subtitle".into(),
                },
                View::Detail {
                    title: "Definition".into(),
                    body: "保存された本文".into(),
                },
                View::Pair {
                    source: "2 + 2".into(),
                    result: "4".into(),
                },
            ],
            state: Some(dupin_plugin_protocol::State {
                version: 1,
                data: serde_json::json!({"page": 2}),
            }),
        }
    }

    #[test]
    fn search_text_matches_titles_queries_and_content_in_any_normal_form() {
        let saved = fixture();
        assert!(matches(&saved, "CAFE\u{301}"));
        assert!(matches(&saved, "本文"));
        assert!(matches(&saved, "2 + 2"));
        assert!(!matches(&saved, "missing"));
    }

    #[test]
    fn dto_versions_fields_and_sizes_are_checked() {
        let saved = fixture();
        let bytes = serde_json::to_vec(&saved).unwrap();
        assert_eq!(serde_json::from_slice::<Snapshot>(&bytes).unwrap(), saved);
        let mut value = serde_json::to_value(&saved).unwrap();
        value["actions"] = serde_json::json!(["execute"]);
        assert!(serde_json::from_value::<Snapshot>(value).is_err());
        let mut unsupported = saved.clone();
        unsupported.version = 2;
        assert!(unsupported.validate().is_err());
        unsupported = saved.clone();
        unsupported.state.as_mut().unwrap().version = 0;
        assert!(unsupported.validate().is_err());
        unsupported = saved.clone();
        unsupported.id = "x".repeat(129);
        assert!(unsupported.validate().is_err());
        let mut oversized = saved;
        oversized.views.push(View::Detail {
            title: "large".into(),
            body: "x".repeat(MAX_ENTRY_BYTES),
        });
        assert!(oversized.validate().is_err());
    }
}
