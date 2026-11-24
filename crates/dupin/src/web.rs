//! Web searches: an alias typed first in the main search, then a query Dupin fills into a URL
//! and opens in the browser or Maps. Extensions list them in their `items()` catalogues, as the
//! bundled web-search package lists config.toml's, and Dupin matches the alias and fills the
//! URL itself, so typing calls into no package.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use dupin_core::config::Config;
use dupin_core::{Action, Item, ItemAction, Payload, Provider, ProviderError, Request, View};
use dupin_js_runtime::Preferences;
use dupin_js_runtime::web_search::{self, PLACEHOLDER, WebSearch};
use serde_json::{Value, json};

use crate::extensions::{Extension, QUERY_DEADLINE};

/// The bundled package that lists config.toml's web searches, as the broker knows it.
pub const WEB_SEARCH_PACKAGE: &str = "bundled:web-search";
const MAX_NAME_CHARS: usize = 80;
const MAX_FALLBACKS: usize = 20;

/// `web_searches` and `web_fallbacks` from config.toml as the web-search package's preferences:
/// only what is set, and only entries it can list, with a notice for the rest.
pub fn preferences(config: &Config, is_reserved: impl Fn(&str) -> bool) -> (Preferences, Option<String>) {
    let mut preferences = Preferences::new();
    let mut problems = Vec::new();
    let mut valid = HashSet::new();
    if let Some(searches) = &config.web_searches {
        let mut listed = serde_json::Map::new();
        for (keyword, search) in searches {
            if web_search::check_alias(keyword).is_ok()
                && !is_reserved(keyword)
                && valid_name(&search.name)
                && web_search::check_template(&search.url).is_ok()
            {
                valid.insert(keyword.as_str());
                listed.insert(keyword.clone(), json!({ "name": search.name, "url": search.url }));
            } else {
                problems.push("Some web searches were ignored because a keyword, name, or URL template is invalid or conflicts with another command.");
            }
        }
        preferences.insert("webSearches".into(), Value::Object(listed));
    }

    let mut fallbacks = Vec::new();
    let mut seen = HashSet::new();
    for keyword in &config.web_fallbacks {
        if fallbacks.len() == MAX_FALLBACKS {
            problems.push("Web fallbacks after the first 20 were ignored.");
            break;
        }
        if !seen.insert(keyword) {
            problems.push("Duplicate web fallbacks were ignored.");
            continue;
        }
        if config.web_searches.is_none() || valid.contains(keyword.as_str()) {
            fallbacks.push(Value::from(keyword.as_str()));
        } else {
            problems.push("Web fallbacks without a valid configured search were ignored.");
        }
    }
    if !fallbacks.is_empty() {
        preferences.insert("webFallbacks".into(), Value::Array(fallbacks));
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
    (preferences, notice)
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
        && !name.contains(PLACEHOLDER)
}

/// The extensions whose catalogues hold web searches, in the order they register, and the
/// keywords that take precedence over an alias.
#[derive(Clone, Debug, Default)]
pub struct WebSearches {
    sources: Vec<Arc<Extension>>,
    reserved: HashSet<String>,
}

impl PartialEq for WebSearches {
    fn eq(&self, other: &Self) -> bool {
        self.reserved == other.reserved
            && self.sources.len() == other.sources.len()
            && self
                .sources
                .iter()
                .zip(&other.sources)
                .all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

impl WebSearches {
    pub fn new(sources: Vec<Arc<Extension>>, reserved: HashSet<String>) -> Self {
        Self { sources, reserved }
    }

    /// Each source's searches, asking a package for its catalogue when it must, without the
    /// ones whose alias is a keyword or an earlier package's alias.
    fn searches(&self, deadline: Instant) -> Vec<(Item, WebSearch)> {
        self.usable(self.sources.iter().flat_map(|extension| {
            let listed = extension.check(deadline).and_then(|exports| {
                if exports.items {
                    extension.web_searches(deadline)
                } else {
                    Ok(Vec::new())
                }
            });
            listed.unwrap_or_else(|error| {
                if let dupin_js_runtime::RunError::Failed(diagnostic) = error {
                    crate::log::warn(&format!("extension {}: {diagnostic}", extension.id));
                }
                Vec::new()
            })
        }))
    }

    /// The searches Dupin holds now, without asking any package.
    fn held(&self) -> Vec<(Item, WebSearch)> {
        self.usable(
            self.sources
                .iter()
                .flat_map(|extension| extension.held_web_searches()),
        )
    }

    fn usable(&self, searches: impl Iterator<Item = (Item, WebSearch)>) -> Vec<(Item, WebSearch)> {
        let mut seen = HashSet::new();
        searches
            .filter(|(_, search)| !self.reserved.contains(&search.alias) && seen.insert(search.alias.clone()))
            .collect()
    }

    /// Every search Dupin holds, filled with `query`: its alias, title and URL.
    pub fn search_urls(&self, query: &str) -> Vec<(String, String, String)> {
        self.held()
            .into_iter()
            .filter_map(|(item, search)| {
                let url = search.url_for(query)?;
                Some((search.alias, item.display_title().to_owned(), url))
            })
            .collect()
    }
}

/// The row a web search shows for `query`: its own title until something is typed, and an
/// action that opens its URL filled with the query. `complete` offers the alias and a space
/// to Tab while nothing is typed after it.
pub(crate) fn item(shown: &Item, search: &WebSearch, query: &str, complete: bool) -> Option<Item> {
    if query.is_empty() && !search.accepts_empty_query() {
        return None;
    }
    let url = search.url_for(query)?;
    let title = if query.is_empty() {
        shown.display_title().to_owned()
    } else {
        search.title_for(query)
    };
    let subtitle = match &shown.view {
        View::Text { subtitle, .. } => subtitle.clone(),
        _ => String::new(),
    };
    let action = if search.opens_maps() {
        "Open in Maps"
    } else {
        "Open in Browser"
    };
    Some(Item {
        id: shown.id.clone(),
        payload: Payload::text(query),
        view: View::Text { title, subtitle },
        record_usage: false,
        icon: shown.icon.clone(),
        matched: Vec::new(),
        completion: (complete && query.is_empty()).then(|| format!("{} ", search.alias)),
        score: None,
        primary: Some("open".into()),
        alternate: None,
        actions: vec![ItemAction::new("open", action, Action::OpenUrl(url))],
    })
}

fn alias_item(searches: &WebSearches, raw: &str, deadline: Instant) -> Option<Item> {
    let raw = raw.trim_start();
    let boundary = raw.find(char::is_whitespace).unwrap_or(raw.len());
    let alias = &raw[..boundary];
    if alias.is_empty() {
        return None;
    }
    let query = raw[boundary..].trim();
    searches
        .searches(deadline)
        .into_iter()
        .find(|(_, search)| search.alias == alias)
        .and_then(|(shown, search)| item(&shown, &search, query, true))
}

fn fallback_items(searches: &WebSearches, raw: &str, limit: usize, deadline: Instant) -> Vec<Item> {
    let query = raw.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut fallbacks: Vec<_> = searches
        .searches(deadline)
        .into_iter()
        .filter_map(|(shown, search)| Some((search.fallback?, shown, search)))
        .collect();
    fallbacks.sort_by_key(|(position, ..)| *position);
    fallbacks
        .iter()
        .filter_map(|(_, shown, search)| item(shown, search, query, false))
        .take(limit)
        .collect()
}

pub struct WebProvider {
    searches: Arc<RwLock<WebSearches>>,
    fallback: bool,
}

impl WebProvider {
    pub fn new(searches: Arc<RwLock<WebSearches>>, fallback: bool) -> Self {
        Self { searches, fallback }
    }
}

impl Provider for WebProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.is_cancelled() || request.limit == 0 {
            return Ok(());
        }
        let searches = {
            let Ok(searches) = self.searches.read() else {
                return Ok(());
            };
            searches.clone()
        };
        let deadline = Instant::now() + QUERY_DEADLINE;
        let items = if self.fallback {
            fallback_items(&searches, &request.query, request.limit, deadline)
        } else {
            alias_item(&searches, &request.query, deadline)
                .into_iter()
                .collect()
        };
        if !items.is_empty() && !request.is_cancelled() {
            emit(items);
        }
        Ok(())
    }
}

/// config.toml's `web_searches` as a table, for tests that write one.
#[cfg(test)]
pub(crate) fn searches(
    entries: &[(&str, &str, &str)],
) -> std::collections::BTreeMap<String, dupin_core::config::WebSearch> {
    entries
        .iter()
        .map(|(keyword, name, url)| {
            (
                (*keyword).to_owned(),
                dupin_core::config::WebSearch {
                    name: (*name).into(),
                    url: (*url).into(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::extensions::{ExtensionProvider, load_folder, preferences as held};

    fn reserved(config: &Config) -> impl Fn(&str) -> bool + '_ {
        |keyword| {
            keyword == "in"
                || config
                    .dictionary_keywords
                    .iter()
                    .any(|reserved| reserved == keyword)
        }
    }

    #[test]
    fn config_reaches_the_package_only_once_set_and_without_entries_it_could_not_list() {
        let (view, notice) = preferences(&Config::default(), |_| false);
        assert!(view.is_empty());
        assert_eq!(notice, None);

        let mut config = Config {
            web_searches: Some(searches(&[
                ("docs", "Docs", "https://docs.example/?q={query}"),
                ("define", "Taken", "https://taken.example/?q={query}"),
                ("bad", "Bad", "file:///tmp/{query}"),
                ("named", "{query}", "https://named.example/?q={query}"),
                ("has space", "Spaced", "https://spaced.example/?q={query}"),
                ("maps", "Apple Maps", web_search::MAPS_TEMPLATE),
            ])),
            web_fallbacks: vec!["docs".into(), "missing".into(), "docs".into(), "maps".into()],
            ..Config::default()
        };
        let (view, notice) = preferences(&config, reserved(&config));
        assert_eq!(
            Value::Object(view),
            json!({
                "webSearches": {
                    "docs": { "name": "Docs", "url": "https://docs.example/?q={query}" },
                    "maps": { "name": "Apple Maps", "url": "maps://?q={query}" },
                },
                "webFallbacks": ["docs", "maps"],
            })
        );
        let notice = notice.unwrap();
        for expected in [
            "Some web searches were ignored",
            "Duplicate web fallbacks were ignored.",
            "Web fallbacks without a valid configured search were ignored.",
        ] {
            assert!(notice.contains(expected), "{notice}");
        }

        config.web_searches = Some(Default::default());
        config.web_fallbacks = (0..MAX_FALLBACKS + 2).map(|index| format!("f{index}")).collect();
        let (view, notice) = preferences(&config, |_| false);
        assert_eq!(Value::Object(view), json!({ "webSearches": {} }));
        assert!(notice.unwrap().contains("without a valid configured search"));

        config.web_searches = None;
        let (view, notice) = preferences(&config, |_| false);
        assert_eq!(view["webFallbacks"].as_array().unwrap().len(), MAX_FALLBACKS);
        assert!(notice.unwrap().contains("after the first 20"));
    }

    fn bundled() -> Arc<Extension> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/web-search");
        load_folder(&root).unwrap()
    }

    fn provider(sources: Vec<Arc<Extension>>, reserved: &[&str], fallback: bool) -> WebProvider {
        let reserved = reserved.iter().map(|keyword| (*keyword).to_owned()).collect();
        WebProvider::new(
            Arc::new(RwLock::new(WebSearches::new(sources, reserved))),
            fallback,
        )
    }

    fn collect(provider: &dyn Provider, query: &str, limit: usize) -> Vec<Item> {
        let mut results = Vec::new();
        provider
            .search(&Request::detached(query, limit), &mut |batch| {
                results.extend(batch)
            })
            .unwrap();
        results
    }

    fn url(item: &Item) -> &str {
        match item.action("open").unwrap() {
            Action::OpenUrl(url) => url,
            _ => panic!("web search should open a URL"),
        }
    }

    fn set(extension: &Extension, config: &Config) {
        let (view, _) = preferences(config, |_| false);
        held::set(extension.package.as_str(), view);
    }

    // One test, so nothing else changes the package's preferences while it runs.
    #[test]
    fn the_bundled_package_answers_aliases_fallbacks_its_keyword_and_send_to() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let extension = bundled();
        assert!(extension.default_search);
        assert_eq!(extension.keyword, "web");
        set(&extension, &Config::default());
        let web = provider(vec![extension.clone()], &["define"], false);

        let item = collect(&web, "g", 5).pop().unwrap();
        assert_eq!(item.display_title(), "Search Google");
        assert!(item.payload.is_empty());
        assert_eq!(item.completion.as_deref(), Some("g "));
        assert_eq!(url(&item), "https://www.google.com/search?q=");
        assert_eq!(collect(&web, "g ", 5)[0].completion.as_deref(), Some("g "));

        let item = collect(&web, "g  rust lifetimes", 5).pop().unwrap();
        assert_eq!(item.display_title(), "Search Google for rust lifetimes");
        assert_eq!(item.payload.nonblank_text(), Some("rust lifetimes"));
        assert_eq!(item.completion, None);
        assert_eq!(item.actions[0].title, "Open in Browser");
        assert_eq!(url(&item), "https://www.google.com/search?q=rust%20lifetimes");
        assert!(!item.record_usage);
        assert_eq!((item.primary.as_deref(), item.score), (Some("open"), None));
        assert!(matches!(&item.view, View::Text { subtitle, .. } if subtitle == "g"));
        assert_eq!(
            url(&collect(&web, "g 日本語 +&/#?", 5)[0]),
            "https://www.google.com/search?q=%E6%97%A5%E6%9C%AC%E8%AA%9E%20%2B%26%2F%23%3F"
        );
        for query in ["", "G rust", "google rust", "safari"] {
            assert!(collect(&web, query, 5).is_empty(), "{query}");
        }
        assert_eq!(
            url(&collect(&web, "gh dupin", 5)[0]),
            "https://github.com/search?q=dupin"
        );
        assert_eq!(
            url(&collect(&web, "yt piano", 5)[0]),
            "https://www.youtube.com/results?search_query=piano"
        );

        assert!(collect(&web, "maps", 5).is_empty());
        assert!(collect(&web, "maps  ", 5).is_empty());
        let item = collect(&web, "maps Baker Street & 東京", 5).pop().unwrap();
        assert_eq!(item.display_title(), "Search Apple Maps for Baker Street & 東京");
        assert_eq!(item.actions[0].title, "Open in Maps");
        assert_eq!(url(&item), "maps://?q=Baker%20Street%20%26%20%E6%9D%B1%E4%BA%AC");

        let fallbacks = provider(vec![extension.clone()], &[], true);
        assert!(collect(&fallbacks, "rust", 10).is_empty());
        let keyword = ExtensionProvider::new(extension.clone());
        let titles = |query: &str| {
            let mut titles = Vec::new();
            keyword
                .search(&Request::detached(query, 50), &mut |batch| {
                    titles.extend(batch.iter().map(|item| item.display_title().to_owned()));
                })
                .unwrap();
            titles
        };
        assert_eq!(
            titles("piano"),
            [
                "Search Google for piano",
                "Search GitHub for piano",
                "Search Apple Maps for piano",
                "Search YouTube for piano",
            ]
        );
        assert_eq!(titles(""), ["Search Google", "Search GitHub", "Search YouTube"]);
        let held = WebSearches::new(vec![extension.clone()], HashSet::new());
        let receivers = crate::receivers::for_payload(&Payload::text("日本語 +&/#?"), &held);
        let receivers: Vec<_> = receivers
            .iter()
            .map(|receiver| (receiver.id.as_str(), receiver.title.as_str()))
            .collect();
        assert_eq!(
            receivers,
            [
                ("copy-text", "Copy"),
                ("web:g", "Search Google (g)"),
                ("web:gh", "Search GitHub (gh)"),
                ("web:maps", "Search Apple Maps (maps)"),
                ("web:yt", "Search YouTube (yt)"),
            ]
        );

        let mut config = Config {
            web_searches: Some(searches(&[
                ("g", "Example", "https://example.com/find?term={query}"),
                ("gh", "GitHub", "https://github.com/search?q={query}"),
                (
                    "yt",
                    "YouTube",
                    "https://www.youtube.com/results?search_query={query}",
                ),
                ("define", "Taken", "https://taken.example/?q={query}"),
            ])),
            web_fallbacks: vec!["yt".into(), "g".into()],
            ..Config::default()
        };
        set(&extension, &config);
        assert_eq!(
            url(&collect(&web, "g term", 5)[0]),
            "https://example.com/find?term=term"
        );
        assert!(collect(&web, "maps term", 5).is_empty());
        assert!(collect(&web, "define term", 5).is_empty());
        let all = collect(&fallbacks, "rust", 10);
        let titles: Vec<_> = all.iter().map(Item::display_title).collect();
        assert_eq!(titles, ["Search YouTube for rust", "Search Example for rust"]);
        assert!(all.iter().all(|item| item.completion.is_none()));
        assert_eq!(collect(&fallbacks, "rust", 1).len(), 1);
        assert!(collect(&fallbacks, "  ", 10).is_empty());
        assert!(collect(&fallbacks, "rust", 0).is_empty());

        config.web_searches = Some(Default::default());
        set(&extension, &config);
        assert!(collect(&web, "g term", 5).is_empty());
        assert!(collect(&fallbacks, "term", 5).is_empty());
        held::set(extension.package.as_str(), Preferences::new());
    }

    #[test]
    fn the_first_package_keeps_an_alias_and_one_without_items_is_skipped() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let folder = tempfile::tempdir().unwrap();
        let package = |name: &str, main: &str| {
            let root = folder.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join("package.json"),
                format!(
                    r#"{{"name": "{name}", "version": "1.0.0", "dupin": {{"title": "{name}", "keyword": "{name}", "defaultSearch": true}}}}"#
                ),
            )
            .unwrap();
            std::fs::write(root.join("main.js"), main).unwrap();
            load_folder(&root).unwrap()
        };
        let search = |name: &str| {
            format!(
                "export function items() {{ return [{{ id: 'g', title: 'Search {name}', webSearch: {{ alias: 'g', url: 'https://{name}.example/?q={{query}}', title: 'Search {name} for {{query}}', fallback: 0 }} }}]; }}"
            )
        };
        let first = package("first", &search("first"));
        let second = package("second", &search("second"));
        let searchless = package("searchless", "export function search() { return []; }");
        let web = provider(vec![searchless, first, second.clone()], &[], false);
        assert_eq!(url(&collect(&web, "g x", 5)[0]), "https://first.example/?q=x");
        let fallbacks = provider(vec![second], &[], true);
        assert_eq!(collect(&fallbacks, "x", 5).len(), 1);
    }
}
