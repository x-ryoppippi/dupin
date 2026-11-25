//! `[advanced]`: recomposing the built-in sources, filters and kinds. Absent, the feature
//! manifest supplies the whole composition, so an empty config.toml behaves as it always has.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The most actions one kind takes from config.toml.
pub const MAX_KIND_ACTIONS: usize = 16;
const MAX_TITLE_BYTES: usize = 128;
const MAX_TEMPLATE_BYTES: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, try_from = "AdvancedValues")]
pub struct Advanced {
    /// Each native source's filter stages, by source name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<String, SourceStages>,
    #[serde(skip_serializing_if = "Routes::is_empty")]
    pub commands: Routes,
    /// Actions added to every row of a kind.
    #[serde(skip_serializing_if = "KindActions::is_empty")]
    pub kinds: KindActions,
}

impl Advanced {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The filter stages a source's matches pass through, by the names Dupin registers them under.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourceStages {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sorter: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Routes {
    /// The main search.
    #[serde(skip_serializing_if = "Route::is_empty")]
    pub default: Route,
}

impl Routes {
    fn is_empty(&self) -> bool {
        self.default.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Route {
    /// Native sources searched after the route's own, by source name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub append_sources: Vec<String>,
}

impl Route {
    fn is_empty(&self) -> bool {
        self.append_sources.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KindActions {
    #[serde(skip_serializing_if = "KindConfig::is_empty")]
    pub text: KindConfig,
    #[serde(skip_serializing_if = "KindConfig::is_empty")]
    pub url: KindConfig,
    #[serde(skip_serializing_if = "KindConfig::is_empty")]
    pub file: KindConfig,
    #[serde(skip_serializing_if = "KindConfig::is_empty")]
    pub app: KindConfig,
}

impl KindActions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn each(&self) -> [(&'static str, &[KindAction], &'static [&'static str]); 4] {
        [
            ("text", &self.text.actions, &["{text}"]),
            ("url", &self.url.actions, &["{url}"]),
            ("file", &self.file.actions, &["{path}", "{name}"]),
            ("app", &self.app.actions, &["{path}", "{name}"]),
        ]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KindConfig {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<KindAction>,
}

impl KindConfig {
    fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

/// An action the user adds to a kind: a script command that receives the row's value as its
/// one argument, or an HTTP(S) URL template the value is percent-encoded into.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KindAction {
    pub title: String,
    /// The file name of a script in the scripts folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AdvancedValues {
    sources: BTreeMap<String, SourceStages>,
    commands: Routes,
    kinds: KindActions,
}

impl TryFrom<AdvancedValues> for Advanced {
    type Error = String;

    fn try_from(values: AdvancedValues) -> Result<Self, String> {
        for (name, stages) in &values.sources {
            if stages.matcher.is_none() && stages.sorter.is_none() {
                return Err(format!(
                    "[advanced.sources.{name}] sets neither `matcher` nor `sorter`"
                ));
            }
        }
        let route = &values.commands.default.append_sources;
        for (index, name) in route.iter().enumerate() {
            if route[..index].contains(name) {
                return Err(format!(
                    "advanced.commands.default.append_sources names `{name}` more than once"
                ));
            }
        }
        for (kind, actions, placeholders) in values.kinds.each() {
            if actions.len() > MAX_KIND_ACTIONS {
                return Err(format!(
                    "advanced.kinds.{kind}.actions has {} actions; a kind takes at most {MAX_KIND_ACTIONS}",
                    actions.len()
                ));
            }
            for (index, action) in actions.iter().enumerate() {
                action
                    .check(placeholders)
                    .map_err(|problem| format!("advanced.kinds.{kind}.actions[{index}]: {problem}"))?;
            }
        }
        Ok(Self {
            sources: values.sources,
            commands: values.commands,
            kinds: values.kinds,
        })
    }
}

impl KindAction {
    fn check(&self, placeholders: &[&str]) -> Result<(), String> {
        let title = self.title.trim();
        if title.is_empty()
            || title != self.title
            || self.title.len() > MAX_TITLE_BYTES
            || self.title.chars().any(char::is_control)
        {
            return Err(format!(
                "`title` must be one line of 1 to {MAX_TITLE_BYTES} bytes without outer whitespace"
            ));
        }
        match (&self.script, &self.url) {
            (Some(script), None) => check_script(script),
            (None, Some(url)) => check_template(url, placeholders),
            _ => Err("set exactly one of `script` and `url`".into()),
        }
    }
}

/// A script is named by its file name in the scripts folder, as a script binding names one.
fn check_script(script: &str) -> Result<(), String> {
    if script.is_empty()
        || script.contains('/')
        || script.starts_with('.')
        || script.chars().any(char::is_control)
    {
        return Err(format!(
            "`script` must be the file name of a script in the scripts folder, not {script:?}"
        ));
    }
    Ok(())
}

/// An HTTP or HTTPS URL with a host and no credentials, whose placeholders, at least one, are
/// all ones the kind fills and sit after the host.
fn check_template(template: &str, placeholders: &[&str]) -> Result<(), String> {
    let allowed = placeholders.join(", ");
    let invalid = || {
        Err(format!(
            "`url` must be an http or https URL of up to {MAX_TEMPLATE_BYTES} bytes with a host, no credentials and at least one of {allowed} after the host"
        ))
    };
    if template.len() > MAX_TEMPLATE_BYTES || template.chars().any(|ch| ch.is_whitespace() || ch.is_control())
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
    if authority.is_empty() || authority.contains('@') || authority.contains('{') {
        return invalid();
    }
    let mut found = false;
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(length) = rest[start..].find('}') else {
            return Err("`url` has a `{` without a closing `}`".into());
        };
        let placeholder = &rest[start..=start + length];
        if !placeholders.contains(&placeholder) {
            return Err(format!(
                "`url` uses {placeholder}, which this kind doesn't fill; it fills {allowed}"
            ));
        }
        found = true;
        rest = &rest[start + length + 1..];
    }
    if !found {
        return invalid();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    use super::*;

    fn problem(source: &str) -> String {
        Config::parse(source).unwrap_err().message().to_owned()
    }

    #[test]
    fn absent_or_empty_advanced_is_the_default_composition_and_is_never_written() {
        for source in ["", "[advanced]", "[advanced.kinds.file]\nactions = []"] {
            let config = Config::parse(source).unwrap();
            assert_eq!(config, Config::default(), "{source:?}");
            assert!(config.advanced.is_empty());
            assert!(
                !toml::to_string(&config).unwrap().contains("advanced"),
                "{source:?}"
            );
        }
    }

    #[test]
    fn the_designs_example_parses_and_round_trips() {
        let config = Config::parse(
            r#"
[advanced.commands.default]
append_sources = ["recent"]

[advanced.sources.apps]
matcher = "builtin.prefix"

[[advanced.kinds.file.actions]]
title = "Open in Zed"
script = "open-in-zed"

[[advanced.kinds.url.actions]]
title = "Archive"
url = "https://web.archive.org/web/{url}"
"#,
        )
        .unwrap();
        let advanced = &config.advanced;
        assert_eq!(advanced.commands.default.append_sources, ["recent"]);
        assert_eq!(
            advanced.sources["apps"].matcher.as_deref(),
            Some("builtin.prefix")
        );
        assert_eq!(advanced.sources["apps"].sorter, None);
        assert_eq!(
            advanced.kinds.file.actions[0].script.as_deref(),
            Some("open-in-zed")
        );
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);
    }

    #[test]
    fn invalid_composition_is_reported_where_it_is() {
        for (source, expected) in [
            (
                "[advanced.source.apps]\nmatcher = \"x\"",
                "unknown field `source`",
            ),
            ("[advanced.sources.apps]\nmatch = \"x\"", "unknown field `match`"),
            ("[advanced.sources.apps]", "[advanced.sources.apps] sets neither"),
            (
                "[advanced.commands.files]\nappend_sources = []",
                "unknown field `files`",
            ),
            (
                "[advanced.commands.default]\nappend_sources = [\"files\", \"files\"]",
                "names `files` more than once",
            ),
            (
                "[[advanced.kinds.folder.actions]]\ntitle = \"x\"\nscript = \"x\"",
                "unknown field `folder`",
            ),
            (
                "[[advanced.kinds.file.actions]]\ntitle = \"Both\"\nscript = \"x\"\nurl = \"https://e.example/{path}\"",
                "advanced.kinds.file.actions[0]: set exactly one of `script` and `url`",
            ),
            (
                "[[advanced.kinds.file.actions]]\ntitle = \"Neither\"",
                "set exactly one of `script` and `url`",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \" Padded\"\nscript = \"x\"",
                "`title` must be one line",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nscript = \"../bin/sh\"",
                "`script` must be the file name",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nscript = \".hidden\"",
                "`script` must be the file name",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nurl = \"https://e.example/{path}\"",
                "`url` uses {path}, which this kind doesn't fill; it fills {text}",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nurl = \"file:///{text}\"",
                "must be an http or https URL",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nurl = \"https://{text}.example/\"",
                "must be an http or https URL",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nurl = \"https://u:p@e.example/{text}\"",
                "must be an http or https URL",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nurl = \"https://e.example/\"",
                "must be an http or https URL",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nurl = \"https://e.example/{text\"",
                "without a closing",
            ),
            (
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nscript = \"x\"\nextra = 1",
                "unknown field `extra`",
            ),
        ] {
            let message = problem(source);
            assert!(message.contains(expected), "{source:?}: {message}");
        }
        let many =
            "[[advanced.kinds.app.actions]]\ntitle = \"x\"\nscript = \"x\"\n".repeat(MAX_KIND_ACTIONS + 1);
        assert!(problem(&many).contains("a kind takes at most 16"));
    }

    #[test]
    fn a_problem_carries_its_location() {
        let source = "show_recent = true\n\n[[advanced.kinds.file.actions]]\ntitle = \"Neither\"\n";
        let error = Config::parse(source).unwrap_err();
        let span = error.span().unwrap();
        assert!(span.start >= source.find("[[advanced").unwrap(), "{span:?}");
    }
}
