//! Keywords: every command and the keywords and aliases that reach it, with the aliases and the
//! per-feature keyword lists editable and what conflicts shown.

use std::collections::BTreeMap;

use dupin_core::config::{AliasTarget, check_alias};
use dupin_core::{Config, KeywordEntry, KeywordTarget};
use rune::{El, TextEdit, col, row, text, text_input};
use rune_kit::{Button, Kit};

use super::Prefs;
use super::controls::{DESCRIPTION, ROW_PX, card, note, setting, setting_with_lock};
use crate::icons;

/// What the page shows and what is being typed on it.
pub(super) struct Keywords {
    /// The registry config.toml composes, without extensions.
    entries: Vec<KeywordEntry>,
    /// Why each alias that reaches nothing was left out, by alias.
    problems: BTreeMap<String, String>,
    alias: TextEdit,
    target: TextEdit,
    dictionary: TextEdit,
    snippets: TextEdit,
    /// Why the last edit on the page wasn't saved.
    pub(super) problem: Option<String>,
}

impl Keywords {
    pub(super) fn new(config: &Config) -> Self {
        let mut keywords = Self {
            entries: Vec::new(),
            problems: BTreeMap::new(),
            alias: TextEdit::new(""),
            target: TextEdit::new(""),
            dictionary: TextEdit::new(""),
            snippets: TextEdit::new(""),
            problem: None,
        };
        keywords.reflect(config);
        keywords
    }

    /// Lists `config`'s keywords again, and puts its keyword lists in their fields.
    pub(super) fn reflect(&mut self, config: &Config) {
        if let Ok(configured) = crate::commands::configured(config) {
            self.entries = configured.registry.entries();
            self.problems = configured.alias_problems;
        }
        self.dictionary = TextEdit::new(config.dictionary_keywords.join(" "));
        self.snippets = TextEdit::new(config.snippet_keywords.join(" "));
    }
}

/// The keywords in `text`, separated by spaces or commas, each one a keyword can be.
fn keyword_list(text: &str) -> Result<Vec<String>, String> {
    let mut keywords: Vec<String> = Vec::new();
    for keyword in text.split([' ', ',']).filter(|keyword| !keyword.is_empty()) {
        if keyword.chars().any(char::is_control) {
            return Err(format!("`{keyword}` can't be a keyword"));
        }
        if !keywords.iter().any(|seen| seen == keyword) {
            keywords.push(keyword.to_owned());
        }
    }
    Ok(keywords)
}

impl Prefs {
    pub(super) fn add_alias(&mut self) {
        let alias = self.keywords.alias.text.trim().to_owned();
        let target = self.keywords.target.text.trim().to_owned();
        let parsed = check_alias(&alias)
            .map_err(|problem| format!("`{alias}`: {problem}"))
            .and_then(|()| AliasTarget::parse(&target));
        let target = match parsed {
            Ok(target) => target,
            Err(problem) => {
                self.keywords.problem = Some(problem);
                return;
            }
        };
        if self.config.aliases.0.contains_key(&alias) {
            self.keywords.problem = Some(format!("`{alias}` is already an alias; remove it first"));
            return;
        }
        if self.config.aliases.0.len() >= dupin_core::config::MAX_ALIASES {
            self.keywords.problem = Some(format!(
                "config.toml takes at most {} aliases",
                dupin_core::config::MAX_ALIASES
            ));
            return;
        }
        self.keywords.problem = None;
        self.keywords.alias = TextEdit::new("");
        self.keywords.target = TextEdit::new("");
        self.set(|config| {
            config.aliases.0.insert(alias, target);
        });
    }

    pub(super) fn remove_alias(&mut self, alias: &str) {
        self.keywords.problem = None;
        self.set(|config| {
            config.aliases.0.remove(alias);
        });
    }

    fn save_keyword_lists(&mut self) {
        let lists = keyword_list(&self.keywords.dictionary.text)
            .and_then(|dictionary| Ok((dictionary, keyword_list(&self.keywords.snippets.text)?)));
        match lists {
            Ok((dictionary, snippets)) => {
                self.keywords.problem = None;
                self.set(|config| {
                    config.dictionary_keywords = dictionary;
                    config.snippet_keywords = snippets;
                });
            }
            Err(problem) => self.keywords.problem = Some(problem),
        }
    }

    pub(super) fn refresh_keywords(&mut self) {
        self.keywords.reflect(&self.config);
    }

    #[cfg(test)]
    pub(super) fn keywords_problem(&self, alias: &str) -> Option<&str> {
        self.keywords.problems.get(alias).map(String::as_str)
    }

    #[cfg(test)]
    pub(super) fn set_alias_draft(&mut self, alias: &str, target: &str) {
        self.keywords.alias = TextEdit::new(alias);
        self.keywords.target = TextEdit::new(target);
    }
}

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let mut body = Vec::new();
    if let Some(problem) = &s.keywords.problem {
        body.push(
            text(problem)
                .key("keywords-problem")
                .w_full()
                .mx(ROW_PX)
                .font(DESCRIPTION)
                .color(k.c.text_base)
                .wrap(),
        );
    }
    body.push(card(k, hair, command_rows(s, k)));
    body.push(note(
        k,
        "Type a keyword, then Space, in the launcher to enter its command; type ? to list them all.",
    ));
    body.push(card(k, hair, alias_rows(s, k)));
    body.push(note(
        k,
        "An alias reaches a command (calendar, dictionary, files, contents, clipboard, snippets, snapshots or an extension's package name), app:<bundle id>, web:<keyword> or file:<path>. Built-in keywords, Dictionary and snippet keywords and web searches keep theirs; an alias goes before extensions' keywords.",
    ));
    body.push(card(k, hair, keyword_list_rows(s, k)));
    body
}

/// Each command, web search, app or file and what reaches it.
fn command_rows(s: &Prefs, k: &Kit) -> Vec<El> {
    let aliases: Vec<&str> = s.config.aliases.0.keys().map(String::as_str).collect();
    let mut rows: Vec<El> = s
        .keywords
        .entries
        .iter()
        .map(|entry| {
            let tags = entry
                .keywords
                .iter()
                .map(|keyword| (keyword.as_str(), aliases.contains(&keyword.as_str())));
            let space = matches!(entry.target, KeywordTarget::Command(_)) && entry.keywords.is_empty();
            command_row(k, &entry.title, &entry.subtitle, tags, space)
        })
        .collect();
    for listed in s.extensions.0.iter().filter(|listed| !listed.built_in) {
        let package = listed
            .package
            .as_str()
            .trim_start_matches(crate::extensions::BUNDLED_PREFIX);
        let mut tags = vec![(listed.keyword.as_str(), false)];
        tags.extend(
            s.config
                .aliases
                .0
                .iter()
                .filter(|(_, target)| **target == AliasTarget::Command(package.to_owned()))
                .map(|(alias, _)| (alias.as_str(), true)),
        );
        let detail = if listed.registered {
            "Extension".to_owned()
        } else {
            format!("Extension · `{}` reaches another command", listed.keyword)
        };
        rows.push(command_row(k, &listed.title, &detail, tags.into_iter(), false));
    }
    rows
}

fn command_row<'a>(
    k: &Kit,
    title: &str,
    detail: &str,
    keywords: impl Iterator<Item = (&'a str, bool)>,
    space: bool,
) -> El {
    let mut tags = row().gap(6.0).center_y();
    if space {
        tags = tags.child(tag(k, "Space on an empty query", false));
    }
    for (keyword, alias) in keywords {
        tags = tags.child(tag(k, keyword, alias));
    }
    setting(k, &icons::KEYBOARD, title, Some(detail), tags)
}

/// A keyword as a label; an alias is drawn quieter, as the user's own.
fn tag(k: &Kit, keyword: &str, alias: bool) -> El {
    text(keyword.to_owned())
        .font(k.ty.ui_medium.lh(16.0))
        .color(if alias { k.c.text_muted } else { k.c.text_base })
        .px(6.0)
        .py(2.0)
        .radius(k.radius.sm)
        .fill(k.c.layer_02)
}

/// Each alias, why it reaches nothing if it doesn't, and a row to add one.
fn alias_rows(s: &Prefs, k: &Kit) -> Vec<El> {
    let locked = s.locked("aliases");
    let installed: Vec<String> = s
        .extensions
        .0
        .iter()
        .filter(|listed| !listed.built_in && listed.registered)
        .map(|listed| {
            listed
                .package
                .as_str()
                .trim_start_matches(crate::extensions::BUNDLED_PREFIX)
                .to_owned()
        })
        .collect();
    let mut rows: Vec<El> = s
        .config
        .aliases
        .0
        .iter()
        .map(|(alias, target)| {
            let problem = s.keywords.problems.get(alias).cloned().or_else(|| match target {
                AliasTarget::Command(name)
                    if crate::features::aliases::built_in(name).is_none() && !installed.contains(name) =>
                {
                    Some(format!(
                        "names `{name}`, which is neither a command nor an extension that is on"
                    ))
                }
                _ => None,
            });
            let detail = match problem {
                Some(problem) => format!("{target} · Left out: {problem}"),
                None => target.to_string(),
            };
            let key = format!("remove-alias-{alias}");
            let removed = alias.clone();
            let remove = Button::new("Remove")
                .disabled(locked)
                .on_press(move |s: &mut Prefs| s.remove_alias(&removed))
                .build(k)
                .key(key);
            setting_with_lock(
                k,
                &icons::KEYBOARD,
                alias,
                Some(&detail),
                remove,
                s.lock_note(locked),
            )
        })
        .collect();
    if !locked {
        let inputs = row()
            .w_full()
            .px(ROW_PX)
            .py(8.0)
            .gap(8.0)
            .center_y()
            .child(
                field(k, &s.keywords.alias, "new-alias", "Alias", |s| {
                    &mut s.keywords.alias
                })
                .w(120.0),
            )
            .child(
                field(
                    k,
                    &s.keywords.target,
                    "new-alias-target",
                    "calendar, app:com.apple.Safari, web:g or file:~/Downloads",
                    |s| &mut s.keywords.target,
                )
                .grow(1.0),
            )
            .child(
                Button::new("Add Alias")
                    .on_press(Prefs::add_alias)
                    .build(k)
                    .key("add-alias"),
            );
        rows.push(inputs);
    }
    rows
}

/// Dictionary's and snippets' keywords, edited as one line each.
fn keyword_list_rows(s: &Prefs, k: &Kit) -> Vec<El> {
    let locked = s.locked("dictionary_keywords") || s.locked("snippet_keywords");
    let mut rows = vec![
        list_row(
            s,
            k,
            "Dictionary keywords",
            "dictionary-keywords",
            &s.keywords.dictionary,
            s.locked("dictionary_keywords"),
            |s| &mut s.keywords.dictionary,
        ),
        list_row(
            s,
            k,
            "Snippet keywords",
            "snippet-keywords",
            &s.keywords.snippets,
            s.locked("snippet_keywords"),
            |s| &mut s.keywords.snippets,
        ),
    ];
    if !locked {
        rows.push(
            row().w_full().px(ROW_PX).py(8.0).child(
                Button::new("Save Keywords")
                    .on_press(Prefs::save_keyword_lists)
                    .build(k)
                    .key("save-keywords"),
            ),
        );
    }
    rows
}

fn list_row(
    s: &Prefs,
    k: &Kit,
    label: &str,
    key: &str,
    value: &TextEdit,
    locked: bool,
    edit: fn(&mut Prefs) -> &mut TextEdit,
) -> El {
    let control = if locked {
        text(value.text.clone()).font(k.ty.ui).color(k.c.text_muted)
    } else {
        field(k, value, key, "Separated by spaces", edit).w(200.0)
    };
    setting_with_lock(
        k,
        &icons::KEYBOARD,
        label,
        Some("Each one, then Space, enters the search"),
        control,
        s.lock_note(locked),
    )
}

fn field(
    k: &Kit,
    value: &TextEdit,
    key: &str,
    placeholder: &str,
    edit: fn(&mut Prefs) -> &mut TextEdit,
) -> El {
    col().key(format!("{key}-box")).child(
        text_input(value, k.ty.ui)
            .key(key)
            .label(placeholder)
            .single_line()
            .placeholder(placeholder, k.c.text_muted)
            .color(k.c.text_base)
            .px(8.0)
            .py(4.0)
            .radius(k.radius.sm)
            .fill(k.c.layer_02)
            .on_edit(edit),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_lists_split_on_spaces_and_commas_once_each() {
        assert_eq!(keyword_list("define, d  d").unwrap(), ["define", "d"]);
        assert!(keyword_list("").unwrap().is_empty());
        assert!(keyword_list("bad\u{7}").is_err());
    }

    #[test]
    fn the_page_lists_aliases_beside_built_in_keywords_with_conflicts() {
        let config = Config::parse(
            "dictionary_keywords = [\"word\"]\n[aliases]\nlookup = \"dictionary\"\nword = \"calendar\"",
        )
        .unwrap();
        let keywords = Keywords::new(&config);
        let dictionary = keywords
            .entries
            .iter()
            .find(|entry| entry.title == "Dictionary")
            .unwrap();
        assert_eq!(dictionary.keywords, ["word", "lookup"]);
        assert_eq!(keywords.problems["word"], "is already a keyword of Dictionary");
        assert_eq!(keywords.dictionary.text, "word");
    }
}
