//! `[advanced]`: recomposing the built-in sources, filters and kinds. Absent, the feature
//! manifest supplies the whole composition, so an empty config.toml behaves as it always has.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The most actions one kind takes from config.toml.
pub const MAX_KIND_ACTIONS: usize = 16;
/// The most action IDs a kind's `hide` or `order` lists.
const MAX_ARRANGED: usize = 32;
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
    pub(crate) fn is_empty(&self) -> bool {
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
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Each kind's name, configuration and the placeholders its URL actions fill.
    fn each(&self) -> [(&'static str, &KindConfig, &'static [&'static str]); 4] {
        [
            ("text", &self.text, &["{text}"]),
            ("url", &self.url, &["{url}"]),
            ("file", &self.file, &["{path}", "{name}"]),
            ("app", &self.app, &["{path}", "{name}"]),
        ]
    }
}

/// The IDs of the kind `kind`'s own actions that `hide` and `order` may name; none for a kind
/// that takes neither.
fn native_actions(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "file" => Some(&crate::kind::FILE_ACTIONS),
        "app" => Some(&crate::kind::APP_ACTIONS),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct KindConfig {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<KindAction>,
    /// Actions left out of the action screen, by native action ID or `<package>/<action>`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
    /// Actions listed first in the action screen, in this order, named as `hide` names them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
}

impl KindConfig {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn check_arranged(&self, kind: &str) -> Result<(), String> {
        let native = native_actions(kind);
        for (field, names) in [("hide", &self.hide), ("order", &self.order)] {
            if names.is_empty() {
                continue;
            }
            let table = format!("advanced.kinds.{kind}.{field}");
            let Some(native) = native else {
                return Err(format!(
                    "{table}: only `file` and `app` take `hide` and `order`; a {kind} row lists the actions of the extension that made it"
                ));
            };
            if names.len() > MAX_ARRANGED {
                return Err(format!(
                    "{table} names {} actions; it takes at most {MAX_ARRANGED}",
                    names.len()
                ));
            }
            for (index, name) in names.iter().enumerate() {
                if !native.contains(&name.as_str()) && contribution(name).is_none() {
                    return Err(format!(
                        "{table} names `{name}`, which isn't one of the {kind} kind's actions ({}) or a package's kind action, written `<package>/<action>`",
                        native.join(", ")
                    ));
                }
                if names[..index].contains(name) {
                    return Err(format!("{table} names `{name}` more than once"));
                }
            }
        }
        if let Some(name) = self.hide.iter().find(|name| self.order.contains(name)) {
            return Err(format!("advanced.kinds.{kind} both hides and orders `{name}`"));
        }
        Ok(())
    }
}

/// An action the user adds to a kind: a script command that receives the row's value as its
/// one argument, an HTTP(S) URL template the value is percent-encoded into, or a kind action
/// an extension provides.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KindAction {
    pub title: String,
    /// The file name of a script in the scripts folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    /// `<package>/<action>`: an action the package's manifest provides for this kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) extension: Option<String>,
}

/// The package and name of a contribution an extension provides, written `<package>/<name>`:
/// its package.json `name`, and the name its manifest provides the filter or action under.
pub fn contribution(reference: &str) -> Option<(&str, &str)> {
    let (package, name) = reference.split_once('/')?;
    let package_ok = (1..=64).contains(&package.len())
        && package
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    let name_ok = (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    (package_ok && name_ok).then_some((package, name))
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
        for (kind, config, placeholders) in values.kinds.each() {
            config.check_arranged(kind)?;
            let actions = &config.actions;
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
        match (&self.script, &self.url, &self.extension) {
            (Some(script), None, None) => check_script(script),
            (None, Some(url), None) => check_template(url, placeholders),
            (None, None, Some(extension)) => contribution(extension).map(drop).ok_or_else(|| {
                format!(
                    "`extension` must be `<package>/<action>`: the package's name and an action it provides, not {extension:?}"
                )
            }),
            _ => Err("set exactly one of `script`, `url` and `extension`".into()),
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
    let host = authority
        .rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(authority, |(host, _)| host);
    if host.is_empty() || authority.contains('@') || authority.contains('{') {
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
                "advanced.kinds.file.actions[0]: set exactly one of `script`, `url` and `extension`",
            ),
            (
                "[[advanced.kinds.file.actions]]\ntitle = \"Neither\"",
                "set exactly one of `script`, `url` and `extension`",
            ),
            (
                "[[advanced.kinds.file.actions]]\ntitle = \"x\"\nextension = \"Uploader/upload\"",
                "`extension` must be `<package>/<action>`",
            ),
            (
                "[[advanced.kinds.file.actions]]\ntitle = \"x\"\nextension = \"uploader\"",
                "`extension` must be `<package>/<action>`",
            ),
            (
                "[[advanced.kinds.file.actions]]\ntitle = \"x\"\nextension = \"uploader/up-load\"",
                "`extension` must be `<package>/<action>`",
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
                "[[advanced.kinds.text.actions]]\ntitle = \"x\"\nurl = \"https://:443/{text}\"",
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
    fn hide_and_order_name_native_actions_and_packages_kind_actions() {
        let config = Config::parse(
            r#"
[advanced.kinds.file]
hide = ["quick-look", "uploader/upload"]
order = ["reveal", "open"]

[advanced.kinds.app]
order = ["quick-look"]
"#,
        )
        .unwrap();
        let kinds = &config.advanced.kinds;
        assert_eq!(kinds.file.hide, ["quick-look", "uploader/upload"]);
        assert_eq!(kinds.file.order, ["reveal", "open"]);
        assert_eq!(kinds.app.order, ["quick-look"]);
        assert_eq!(Config::parse(&toml::to_string(&config).unwrap()).unwrap(), config);
        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&encoded).unwrap(), config);
        for id in crate::kind::FILE_ACTIONS {
            Config::parse(&format!("[advanced.kinds.file]\nhide = [{id:?}]")).unwrap();
        }
        for id in crate::kind::APP_ACTIONS {
            Config::parse(&format!("[advanced.kinds.app]\norder = [{id:?}]")).unwrap();
        }
    }

    #[test]
    fn hide_and_order_reject_what_no_kind_action_is() {
        for (source, expected) in [
            (
                "[advanced.kinds.file]\nhide = [\"quicklook\"]",
                "advanced.kinds.file.hide names `quicklook`, which isn't one of the file kind's actions (open, browse, open-with, reveal, quick-look, copy-name, rename, move-to, copy-to, duplicate, compress, trash) or a package's kind action, written `<package>/<action>`",
            ),
            (
                "[advanced.kinds.app]\norder = [\"trash\"]",
                "advanced.kinds.app.order names `trash`, which isn't one of the app kind's actions (open, reveal, quick-look)",
            ),
            (
                "[advanced.kinds.file]\norder = [\"configured-file-0\"]",
                "names `configured-file-0`, which isn't one of the file kind's actions",
            ),
            (
                "[advanced.kinds.file]\norder = [\"reveal\", \"reveal\"]",
                "advanced.kinds.file.order names `reveal` more than once",
            ),
            (
                "[advanced.kinds.file]\nhide = [\"reveal\"]\norder = [\"reveal\"]",
                "advanced.kinds.file both hides and orders `reveal`",
            ),
            (
                "[advanced.kinds.text]\nhide = [\"copy\"]",
                "advanced.kinds.text.hide: only `file` and `app` take `hide` and `order`",
            ),
            (
                "[advanced.kinds.url]\norder = [\"open\"]",
                "advanced.kinds.url.order: only `file` and `app` take `hide` and `order`",
            ),
            ("[advanced.kinds.file]\nshow = []", "unknown field `show`"),
        ] {
            let message = problem(source);
            assert!(message.contains(expected), "{source:?}: {message}");
        }
        let many: Vec<_> = (0..=MAX_ARRANGED)
            .map(|index| format!("\"p/a{index}\""))
            .collect();
        assert!(
            problem(&format!("[advanced.kinds.file]\nhide = [{}]", many.join(", ")))
                .contains("it takes at most 32")
        );
    }

    #[test]
    fn a_problem_carries_its_location() {
        let source = "show_recent = true\n\n[[advanced.kinds.file.actions]]\ntitle = \"Neither\"\n";
        let error = Config::parse(source).unwrap_err();
        let span = error.span().unwrap();
        assert!(span.start >= source.find("[[advanced").unwrap(), "{span:?}");
    }
}
