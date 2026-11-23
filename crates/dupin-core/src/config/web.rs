use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MAPS_SEARCH_TEMPLATE: &str = "maps://?q={query}";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearch {
    pub name: String,
    pub url: String,
}

pub(super) fn default_searches() -> BTreeMap<String, WebSearch> {
    BTreeMap::from([
        (
            "g".into(),
            WebSearch {
                name: "Google".into(),
                url: "https://www.google.com/search?q={query}".into(),
            },
        ),
        (
            "gh".into(),
            WebSearch {
                name: "GitHub".into(),
                url: "https://github.com/search?q={query}".into(),
            },
        ),
        (
            "maps".into(),
            WebSearch {
                name: "Apple Maps".into(),
                url: MAPS_SEARCH_TEMPLATE.into(),
            },
        ),
        (
            "yt".into(),
            WebSearch {
                name: "YouTube".into(),
                url: "https://www.youtube.com/results?search_query={query}".into(),
            },
        ),
    ])
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn web_search_defaults_and_custom_entries_round_trip() {
        let defaults = Config::default();
        assert_eq!(defaults.web_searches["g"].name, "Google");
        assert_eq!(
            defaults.web_searches["g"].url,
            "https://www.google.com/search?q={query}"
        );
        assert_eq!(defaults.web_searches["gh"].name, "GitHub");
        assert_eq!(defaults.web_searches["yt"].name, "YouTube");
        assert!(defaults.web_fallbacks.is_empty());

        let config = Config::parse(
            r#"
web_searches."docs".name = "Docs"
web_searches."docs".url = "https://docs.example/search?q={query}"
web_fallbacks = ["docs", "g"]
"#,
        )
        .unwrap();
        assert_eq!(config.web_searches["docs"].name, "Docs");
        assert_eq!(config.web_searches.len(), 1);
        assert!(
            Config::parse("web_searches = {}")
                .unwrap()
                .web_searches
                .is_empty()
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
    }
}
