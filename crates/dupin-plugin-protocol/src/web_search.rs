//! A catalogue item's `webSearch`: an alias the main search routes to the item and a URL
//! template Dupin fills with what follows it. Dupin, not the package, fills the template as the
//! user types, so a web search costs no call into the package per keystroke.

use serde::{Deserialize, Serialize};

pub const MAX_ALIAS_BYTES: usize = 32;
pub const MAX_TITLE_BYTES: usize = 512;
pub const MAX_TEMPLATE_BYTES: usize = 4096;
/// The highest `fallback` position.
pub const MAX_FALLBACK: u32 = 1000;
pub const PLACEHOLDER: &str = "{query}";
/// The one template that opens Apple Maps rather than the browser.
pub const MAPS_TEMPLATE: &str = "maps://?q={query}";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearch {
    /// Typed first in the main search, then a space and the query.
    pub alias: String,
    /// An HTTP or HTTPS URL, or [`MAPS_TEMPLATE`], with one [`PLACEHOLDER`].
    pub url: String,
    /// The row's title once a query is typed, with one [`PLACEHOLDER`].
    pub title: String,
    /// Offered, lowest first, when nothing else matches the main search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<u32>,
}

impl WebSearch {
    pub fn validate(&self) -> Result<(), String> {
        check_alias(&self.alias)?;
        check_template(&self.url)?;
        check_title(&self.title)?;
        if self.fallback.is_some_and(|fallback| fallback > MAX_FALLBACK) {
            return Err(format!("`webSearch.fallback` must be from 0 to {MAX_FALLBACK}"));
        }
        Ok(())
    }

    /// Apple Maps needs something to look for; a browser search opens with an empty query.
    pub fn opens_maps(&self) -> bool {
        self.url == MAPS_TEMPLATE
    }

    pub fn accepts_empty_query(&self) -> bool {
        !self.opens_maps()
    }

    /// The template with `query` percent-encoded into it, or none when the URL would be longer
    /// than a template may be.
    pub fn url_for(&self, query: &str) -> Option<String> {
        let encoded = percent_encode(query);
        let length = (self.url.len() - PLACEHOLDER.len()).checked_add(encoded.len())?;
        (length <= MAX_TEMPLATE_BYTES).then(|| self.url.replacen(PLACEHOLDER, &encoded, 1))
    }

    pub fn title_for(&self, query: &str) -> String {
        self.title.replacen(PLACEHOLDER, query, 1)
    }

    pub fn text_bytes(&self) -> usize {
        self.alias.len() + self.url.len() + self.title.len()
    }
}

pub fn check_alias(alias: &str) -> Result<(), String> {
    let valid = !alias.is_empty()
        && alias.len() <= MAX_ALIAS_BYTES
        && alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "`webSearch.alias` must be 1 to {MAX_ALIAS_BYTES} ASCII letters, digits, `_` or `-`"
        ))
    }
}

/// An HTTP or HTTPS URL with a host and no credentials, whose one placeholder is in its path,
/// query or fragment; or exactly [`MAPS_TEMPLATE`].
pub fn check_template(template: &str) -> Result<(), String> {
    if template == MAPS_TEMPLATE {
        return Ok(());
    }
    let invalid = || {
        Err(format!(
            "`webSearch.url` must be an http or https URL of up to {MAX_TEMPLATE_BYTES} bytes with one `{PLACEHOLDER}` after its host, or `{MAPS_TEMPLATE}`"
        ))
    };
    if template.len() > MAX_TEMPLATE_BYTES
        || template.matches(PLACEHOLDER).count() != 1
        || template.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return invalid();
    }
    let Some((scheme, rest)) = template.split_once("://") else {
        return invalid();
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return invalid();
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(authority, |(host, _)| host);
    if host.is_empty() || authority.contains(PLACEHOLDER) || authority.contains('@') {
        return invalid();
    }
    Ok(())
}

fn check_title(title: &str) -> Result<(), String> {
    if title.len() > MAX_TITLE_BYTES
        || title.matches(PLACEHOLDER).count() != 1
        || title.chars().any(char::is_control)
    {
        return Err(format!(
            "`webSearch.title` must be one line of up to {} bytes with one `{PLACEHOLDER}`",
            MAX_TITLE_BYTES
        ));
    }
    Ok(())
}

/// RFC 3986 percent-encoding of the UTF-8 bytes, leaving only unreserved characters as they are.
pub fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(url: &str) -> WebSearch {
        WebSearch {
            alias: "g".into(),
            url: url.into(),
            title: "Search Google for {query}".into(),
            fallback: None,
        }
    }

    #[test]
    fn queries_are_percent_encoded_as_utf8_into_the_one_placeholder() {
        let google = search("https://www.google.com/search?q={query}");
        assert_eq!(
            google.url_for("日本語 +&/#?").as_deref(),
            Some("https://www.google.com/search?q=%E6%97%A5%E6%9C%AC%E8%AA%9E%20%2B%26%2F%23%3F")
        );
        assert_eq!(
            google.url_for("").as_deref(),
            Some("https://www.google.com/search?q=")
        );
        assert_eq!(google.title_for("rust {query}"), "Search Google for rust {query}");
        assert_eq!(google.url_for(&"x".repeat(MAX_TEMPLATE_BYTES)), None);
        let maps = search(MAPS_TEMPLATE);
        assert!(maps.opens_maps() && !maps.accepts_empty_query());
        assert_eq!(
            maps.url_for("Baker Street & 東京").as_deref(),
            Some("maps://?q=Baker%20Street%20%26%20%E6%9D%B1%E4%BA%AC")
        );
    }

    #[test]
    fn unsafe_templates_are_refused() {
        for template in [
            "file:///tmp/{query}".to_owned(),
            "maps://example.com/?q={query}".to_owned(),
            "maps://?q={query}&dirflg=d".to_owned(),
            "maps://?daddr={query}".to_owned(),
            "https:///search?q={query}".to_owned(),
            "https://:443/search?q={query}".to_owned(),
            "https://user:pass@example.com/?q={query}".to_owned(),
            "https://{query}.example/search".to_owned(),
            "https://example.com/search".to_owned(),
            "https://example.com/search?q={query}&q={query}".to_owned(),
            "https://example.com/search?q= {query}".to_owned(),
            "https://example.com/{query}".to_owned() + &"x".repeat(MAX_TEMPLATE_BYTES),
        ] {
            assert!(check_template(&template).is_err(), "{template}");
        }
        for template in [
            "https://example.com/{query}",
            "HTTP://example.com:8080/search?q={query}",
            "https://example.com/#{query}",
            MAPS_TEMPLATE,
        ] {
            assert_eq!(check_template(template), Ok(()), "{template}");
        }
    }

    #[test]
    fn aliases_titles_and_fallbacks_are_bounded() {
        assert!(check_alias("my-search_2").is_ok());
        for alias in ["", "has space", "é", &"x".repeat(MAX_ALIAS_BYTES + 1)] {
            assert!(check_alias(alias).is_err(), "{alias}");
        }
        let mut google = search("https://www.google.com/search?q={query}");
        assert_eq!(google.validate(), Ok(()));
        google.title = "Search Google".into();
        assert!(google.validate().is_err());
        google.title = "Search Google for {query}".into();
        google.fallback = Some(MAX_FALLBACK + 1);
        assert!(google.validate().is_err());
    }
}
