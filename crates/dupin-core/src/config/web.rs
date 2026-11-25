use serde::{Deserialize, Serialize};

/// One entry of `web_searches`. The bundled web-search extension lists the defaults and these
/// replace them.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearch {
    pub name: String,
    pub url: String,
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn web_searches_are_unset_until_written_and_custom_entries_round_trip() {
        let defaults = Config::default();
        assert_eq!(defaults.web_searches, None);
        assert!(defaults.web_fallbacks.is_empty());

        let config = Config::parse(
            r#"
web_searches."docs".name = "Docs"
web_searches."docs".url = "https://docs.example/search?q={query}"
web_fallbacks = ["docs", "g"]
"#,
        )
        .unwrap();
        let searches = config.web_searches.as_ref().unwrap();
        assert_eq!(searches["docs"].name, "Docs");
        assert_eq!(searches.len(), 1);
        assert_eq!(
            Config::parse("web_searches = {}").unwrap().web_searches,
            Some(Default::default())
        );
        assert_eq!(config.web_fallbacks, ["docs", "g"]);

        assert!(
            Config::parse(
                r#"
[web_searches.docs]
name = "Docs"
url = "https://docs.example/search?q={query}"
unexpected = true
"#
            )
            .is_err()
        );

        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);
        let encoded = serde_json::to_string(&defaults).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), defaults);
    }
}
