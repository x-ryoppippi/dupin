use serde::{Deserialize, Serialize};

pub(super) const DEFAULT_PASTE: bool = false;

pub(super) fn default_keywords() -> Vec<String> {
    vec!["snip".into()]
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snippet {
    /// Shown in results instead of the keyword.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn snippets_default_to_none_and_round_trip() {
        let defaults = Config::default();
        assert!(defaults.snippets.is_empty());
        assert_eq!(defaults.snippet_keywords, ["snip"]);
        assert!(!defaults.snippet_paste);

        let config = Config::parse(
            r#"
snippet_keywords = ["s"]
snippet_paste = true

[snippets.sig]
name = "Signature"
text = "Best wishes,\nDupin"

[snippets.today]
text = "{date}"
"#,
        )
        .unwrap();
        assert_eq!(config.snippet_keywords, ["s"]);
        assert!(config.snippet_paste);
        assert_eq!(config.snippets["sig"].name.as_deref(), Some("Signature"));
        assert_eq!(config.snippets["sig"].text, "Best wishes,\nDupin");
        assert_eq!(config.snippets["today"].name, None);
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);

        assert!(Config::parse("[snippets.sig]\nname = \"Signature\"").is_err());
        assert!(Config::parse("[snippets.sig]\ntext = \"x\"\nkeyword = \"y\"").is_err());
    }
}
