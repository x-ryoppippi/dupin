use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

use dupin_core::config::Config;
use dupin_core::{Action, Item, ItemAction, Payload, Provider, ProviderError, Request, View};
use objc2_foundation::{NSString, NSURL};

const MAX_KEYWORD_BYTES: usize = 32;
const MAX_NAME_CHARS: usize = 80;
const MAX_TEMPLATE_BYTES: usize = 4096;
const MAX_FALLBACKS: usize = 20;
const QUERY_MARKER: &str = "dupinquery";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebCatalog {
    searches: BTreeMap<String, ValidatedSearch>,
    fallbacks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ValidatedSearch {
    name: String,
    url: String,
}

impl WebCatalog {
    pub fn receivers(&self, query: &str) -> impl Iterator<Item = ItemAction> {
        self.searches.iter().filter_map(move |(keyword, search)| {
            let url = fill_template(&search.url, query)?;
            Some(ItemAction::new(
                &format!("web:{keyword}"),
                &format!("Search {} ({keyword})", search.name),
                Action::OpenUrl(url),
            ))
        })
    }

    pub fn compile(config: &Config, is_reserved: impl Fn(&str) -> bool) -> (Self, Option<String>) {
        let mut catalog = Self::default();
        let mut problems = Vec::new();

        for (keyword, search) in &config.web_searches {
            if !valid_keyword(keyword)
                || is_reserved(keyword)
                || !valid_name(&search.name)
                || !valid_template(&search.url)
            {
                problems.push("Some web searches were ignored because a keyword, name, or URL template is invalid or conflicts with another command.");
                continue;
            }
            catalog.searches.insert(
                keyword.clone(),
                ValidatedSearch {
                    name: search.name.clone(),
                    url: search.url.clone(),
                },
            );
        }

        let mut seen = HashSet::new();
        for keyword in &config.web_fallbacks {
            if catalog.fallbacks.len() == MAX_FALLBACKS {
                problems.push("Web fallbacks after the first 20 were ignored.");
                break;
            }
            if !seen.insert(keyword) {
                problems.push("Duplicate web fallbacks were ignored.");
                continue;
            }
            if catalog.searches.contains_key(keyword) {
                catalog.fallbacks.push(keyword.clone());
            } else {
                problems.push("Web fallbacks without a valid configured search were ignored.");
            }
        }

        let notice = (!problems.is_empty()).then(|| {
            let mut unique = Vec::new();
            for problem in problems {
                if !unique.contains(&problem) {
                    unique.push(problem);
                }
            }
            unique.join(" ")
        });
        (catalog, notice)
    }
}

pub struct WebProvider {
    catalog: Arc<RwLock<WebCatalog>>,
    fallback: bool,
}

impl WebProvider {
    pub fn new(catalog: Arc<RwLock<WebCatalog>>, fallback: bool) -> Self {
        Self { catalog, fallback }
    }
}

impl Provider for WebProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.is_cancelled() || request.limit == 0 {
            return Ok(());
        }
        let catalog = {
            let Ok(catalog) = self.catalog.read() else {
                return Ok(());
            };
            catalog.clone()
        };

        let items = if self.fallback {
            fallback_items(&catalog, &request.query, request.limit, || request.is_cancelled())
        } else {
            keyword_item(&catalog, &request.query).into_iter().collect()
        };
        if !items.is_empty() && !request.is_cancelled() {
            emit(items);
        }
        Ok(())
    }
}

fn valid_keyword(keyword: &str) -> bool {
    !keyword.is_empty()
        && keyword.len() <= MAX_KEYWORD_BYTES
        && keyword
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= MAX_NAME_CHARS && !name.chars().any(char::is_control)
}

fn valid_template(template: &str) -> bool {
    if template.len() > MAX_TEMPLATE_BYTES
        || template.matches("{query}").count() != 1
        || template.chars().any(char::is_whitespace)
    {
        return false;
    }
    let Some((scheme, after_scheme)) = template.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains("{query}") || authority.contains('@') {
        return false;
    }

    let probe = template.replace("{query}", QUERY_MARKER);
    let Some(url) = NSURL::URLWithString(&NSString::from_str(&probe)) else {
        return false;
    };
    let Some(parsed_scheme) = url.scheme() else {
        return false;
    };
    let Some(host) = url.host() else {
        return false;
    };
    (parsed_scheme.to_string().eq_ignore_ascii_case("http")
        || parsed_scheme.to_string().eq_ignore_ascii_case("https"))
        && !host.to_string().is_empty()
        && url.user().is_none()
        && url.password().is_none()
}

fn keyword_item(catalog: &WebCatalog, raw: &str) -> Option<Item> {
    let raw = raw.trim_start();
    let boundary = raw.find(char::is_whitespace).unwrap_or(raw.len());
    let keyword = &raw[..boundary];
    let search = catalog.searches.get(keyword)?;
    let query = raw[boundary..].trim();
    search_item(
        keyword,
        search,
        query,
        query.is_empty().then(|| format!("{keyword} ")),
    )
}

fn fallback_items(
    catalog: &WebCatalog,
    raw: &str,
    limit: usize,
    mut is_cancelled: impl FnMut() -> bool,
) -> Vec<Item> {
    let query = raw.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut items = Vec::new();
    for keyword in catalog.fallbacks.iter().take(limit) {
        if is_cancelled() {
            break;
        }
        let Some(search) = catalog.searches.get(keyword) else {
            continue;
        };
        if let Some(item) = search_item(keyword, search, query, None) {
            items.push(item);
        }
    }
    items
}

fn fill_template(template: &str, query: &str) -> Option<String> {
    let encoded_len = query.bytes().try_fold(0usize, |size, byte| {
        size.checked_add(
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                1
            } else {
                3
            },
        )
    })?;
    let output_len = template
        .len()
        .checked_sub("{query}".len())?
        .checked_add(encoded_len)?;
    if output_len > MAX_TEMPLATE_BYTES {
        return None;
    }
    let encoded = percent_encode(query);
    Some(template.replace("{query}", &encoded))
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    encoded
}

fn search_item(
    keyword: &str,
    search: &ValidatedSearch,
    query: &str,
    completion: Option<String>,
) -> Option<Item> {
    let url = fill_template(&search.url, query)?;
    let (title, payload) = if query.is_empty() {
        (format!("Search {}", search.name), Payload::default())
    } else {
        (
            format!("Search {} for {query}", search.name),
            Payload::text(query),
        )
    };
    Some(Item {
        id: format!("web:{keyword}"),
        payload,
        view: View::Text {
            title,
            subtitle: keyword.to_owned(),
        },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion,
        score: None,
        primary: Some("open".into()),
        alternate: None,
        actions: vec![ItemAction::new("open", "Open in Browser", Action::OpenUrl(url))],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_core::config::WebSearch;

    fn config() -> Config {
        Config {
            web_fallbacks: vec!["g".into(), "gh".into(), "yt".into()],
            ..Config::default()
        }
    }

    fn compile(config: &Config) -> (WebCatalog, Option<String>) {
        WebCatalog::compile(config, |keyword| {
            keyword == "in"
                || config
                    .dictionary_keywords
                    .iter()
                    .any(|reserved| reserved == keyword)
        })
    }

    fn compiled(config: &Config) -> (Arc<RwLock<WebCatalog>>, Option<String>) {
        let (catalog, notice) = compile(config);
        (Arc::new(RwLock::new(catalog)), notice)
    }

    #[test]
    fn reserved_keywords_come_from_the_hosts_registered_commands() {
        let (catalog, notice) = WebCatalog::compile(&Config::default(), |keyword| keyword == "gh");
        assert!(notice.is_some());
        assert!(!catalog.searches.contains_key("gh"));
        assert!(catalog.searches.contains_key("g"));
    }

    fn collect(provider: &WebProvider, query: &str, limit: usize) -> Vec<Item> {
        let request = Request::detached(query, limit);
        let mut results = Vec::new();
        provider
            .search(&request, &mut |batch| results.extend(batch))
            .unwrap();
        results
    }

    fn url(item: &Item) -> &str {
        match item.action("open").unwrap() {
            Action::OpenUrl(url) => url,
            _ => panic!("web search should open a URL"),
        }
    }

    #[test]
    fn search_keywords_are_case_sensitive_and_tab_completes_only_keyword_alone() {
        let (catalog, _) = compiled(&Config::default());
        let provider = WebProvider::new(catalog, false);

        let item = collect(&provider, "g", 5).pop().unwrap();
        assert_eq!(item.display_title(), "Search Google");
        assert!(item.payload.is_empty());
        assert_eq!(item.completion.as_deref(), Some("g "));
        assert_eq!(url(&item), "https://www.google.com/search?q=");

        let item = collect(&provider, "g ", 5).pop().unwrap();
        assert_eq!(item.completion.as_deref(), Some("g "));
        assert!(item.payload.is_empty());

        let item = collect(&provider, "g  rust lifetimes", 5).pop().unwrap();
        assert_eq!(item.display_title(), "Search Google for rust lifetimes");
        assert_eq!(item.payload.nonblank_text(), Some("rust lifetimes"));
        assert_eq!(item.completion, None);
        assert_eq!(url(&item), "https://www.google.com/search?q=rust%20lifetimes");
        assert!(collect(&provider, "", 5).is_empty());
        assert!(collect(&provider, "G rust", 5).is_empty());
        assert!(collect(&provider, "google rust", 5).is_empty());
    }

    #[test]
    fn query_uses_rfc3986_percent_encoding_for_unicode_and_reserved_characters() {
        let (catalog, _) = compiled(&Config::default());
        let provider = WebProvider::new(catalog, false);
        let item = collect(&provider, "g 日本語 +&/#?", 5).pop().unwrap();
        assert_eq!(
            url(&item),
            "https://www.google.com/search?q=%E6%97%A5%E6%9C%AC%E8%AA%9E%20%2B%26%2F%23%3F"
        );
    }

    #[test]
    fn configured_search_receivers_use_the_same_templates_as_keyword_search() {
        let (catalog, _) = compile(&Config::default());
        let query = "日本語 +&/#?";
        let receivers: Vec<_> = catalog.receivers(query).collect();
        assert_eq!(receivers.len(), catalog.searches.len());
        for receiver in receivers {
            let keyword = receiver.id.strip_prefix("web:").unwrap();
            let item = keyword_item(&catalog, &format!("{keyword} {query}")).unwrap();
            assert_eq!(
                receiver,
                ItemAction::new(&receiver.id, &receiver.title, Action::OpenUrl(url(&item).into()),)
            );
        }
        assert_eq!(catalog.receivers(&"x".repeat(MAX_TEMPLATE_BYTES)).count(), 0);
    }

    #[test]
    fn provider_results_have_host_actions_and_do_not_record_history() {
        let (catalog, _) = compiled(&Config::default());
        let provider = WebProvider::new(catalog, false);
        let item = collect(&provider, "gh dupin", 5).pop().unwrap();
        assert!(!item.record_usage);
        assert_eq!(item.primary.as_deref(), Some("open"));
        assert_eq!(item.alternate, None);
        assert_eq!(item.score, None);
        assert!(item.icon.is_none());
        assert_eq!(item.actions.len(), 1);
    }

    #[test]
    fn invalid_entries_and_keyword_collisions_are_skipped_with_static_notice() {
        let mut config = config();
        config.dictionary_keywords.push("lookup".into());
        config.web_searches.insert(
            "lookup".into(),
            WebSearch {
                name: "Private Engine".into(),
                url: "https://search.example/?q={query}".into(),
            },
        );
        config.web_searches.insert(
            "bad".into(),
            WebSearch {
                name: "Bad".into(),
                url: "file:///tmp/{query}".into(),
            },
        );
        let (catalog, notice) = compiled(&config);
        assert!(notice.is_some());
        let provider = WebProvider::new(catalog, false);
        assert!(collect(&provider, "lookup word", 5).is_empty());
        assert!(collect(&provider, "bad word", 5).is_empty());
        assert_eq!(collect(&provider, "g word", 5).len(), 1);
    }

    #[test]
    fn unsafe_templates_are_rejected() {
        let invalid_templates = [
            "file:///tmp/{query}".to_owned(),
            "https:///search?q={query}".to_owned(),
            "https://user:pass@example.com/?q={query}".to_owned(),
            "https://{query}.example/search".to_owned(),
            "https://example.com/search".to_owned(),
            "https://example.com/search?q={query}&q={query}".to_owned(),
            "https://example.com/{query}".to_owned() + &"x".repeat(MAX_TEMPLATE_BYTES),
        ];
        for template in invalid_templates {
            let config = Config {
                web_searches: BTreeMap::from([(
                    "test".into(),
                    WebSearch {
                        name: "Test".into(),
                        url: template,
                    },
                )]),
                ..Config::default()
            };
            let (catalog, notice) = compile(&config);
            assert!(catalog.searches.is_empty());
            assert!(notice.is_some());
        }
    }

    #[test]
    fn fallback_results_keep_configured_order_obey_limits_and_skip_empty_queries() {
        let (catalog, _) = compiled(&config());
        let provider = WebProvider::new(catalog, true);
        let all = collect(&provider, "rust", 10);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].display_title(), "Search Google for rust");
        assert_eq!(all[1].display_title(), "Search GitHub for rust");
        assert_eq!(all[2].display_title(), "Search YouTube for rust");
        assert!(
            all.iter()
                .all(|item| item.payload.nonblank_text() == Some("rust"))
        );
        assert_eq!(collect(&provider, "rust", 2).len(), 2);
        assert!(collect(&provider, "  ", 10).is_empty());
        assert!(collect(&provider, "rust", 0).is_empty());
    }

    #[test]
    fn fallback_stops_iterating_as_soon_as_the_request_is_cancelled() {
        let (catalog, _) = compile(&config());
        let mut checks = 0;
        let items = fallback_items(&catalog, "rust", 3, || {
            checks += 1;
            checks > 1
        });
        assert_eq!(items.len(), 1);
        assert_eq!(checks, 2);
    }

    #[test]
    fn provider_observes_replaced_shared_catalogue() {
        let (catalog, _) = compiled(&Config::default());
        let provider = WebProvider::new(catalog.clone(), false);
        let fallback_provider = WebProvider::new(catalog.clone(), true);
        assert_eq!(
            url(&collect(&provider, "g term", 5)[0]),
            "https://www.google.com/search?q=term"
        );
        assert!(collect(&fallback_provider, "term", 5).is_empty());

        let config = Config {
            web_searches: BTreeMap::from([(
                "g".into(),
                WebSearch {
                    name: "Example".into(),
                    url: "https://example.com/find?term={query}".into(),
                },
            )]),
            web_fallbacks: vec!["g".into()],
            ..Config::default()
        };
        let (replacement, _) = compile(&config);
        *catalog.write().unwrap() = replacement;

        assert_eq!(
            url(&collect(&provider, "g term", 5)[0]),
            "https://example.com/find?term=term"
        );
        assert!(collect(&provider, "gh term", 5).is_empty());
        assert_eq!(
            url(&collect(&fallback_provider, "term", 5)[0]),
            "https://example.com/find?term=term"
        );
    }

    #[test]
    fn fallback_configuration_skips_unknown_and_duplicate_keys_and_caps_length() {
        let mut config = config();
        config.web_fallbacks = vec!["g".into(), "missing".into(), "g".into()];
        config
            .web_fallbacks
            .extend(std::iter::repeat_n("gh".into(), MAX_FALLBACKS + 2));
        let (catalog, notice) = compile(&config);
        assert!(notice.is_some());
        assert_eq!(catalog.fallbacks.len(), 2);
        assert_eq!(catalog.fallbacks[0], "g");
        assert_eq!(catalog.fallbacks[1], "gh");
    }
}
