//! Snippets: the ones stored in Dupin, which are added, edited and deleted here, and
//! config.toml's, which are listed read-only and can be moved into storage once when the file
//! that holds them is writable.

use dupin_core::Config;
use dupin_core::snippets::{Problems, Snippet, Source};
use rune::{El, TextEdit, auto_grow, col, el, row, text, text_input};
use rune_kit::{Button, Kit};

use super::Prefs;
use super::controls::{DESCRIPTION, ROW_PX, card, note, setting, setting_with_lock};
use crate::icons;

const PREVIEW_CHARS: usize = 60;

/// What the page lists and what is being typed on it.
#[derive(Default)]
pub(super) struct Snippets {
    listed: Vec<(Snippet, Source)>,
    notices: Vec<String>,
    editor: Option<Editor>,
    /// The stored snippet whose Delete was pressed once, which a second press deletes.
    confirming: Option<String>,
    /// A change is being written, so another waits.
    busy: bool,
    /// What the last change did, or why it wasn't made.
    pub(super) outcome: Option<String>,
}

struct Editor {
    /// The keyword of the stored snippet being edited; `None` while adding one.
    replaces: Option<String>,
    name: TextEdit,
    keyword: TextEdit,
    text: TextEdit,
    problems: Problems,
}

impl Snippets {
    pub(super) fn new(config: &Config) -> Self {
        let mut snippets = Self::default();
        snippets.reflect(config);
        snippets
    }

    /// Lists config.toml's snippets and the stored ones again, as the launcher lists them.
    pub(super) fn reflect(&mut self, config: &Config) {
        let listing = dupin_core::snippets::listing(&config.snippets, &crate::snippets::library().stored());
        self.listed = listing.snippets;
        self.notices = listing.notices;
        if let Some(problem) = crate::snippets::library().problem() {
            self.notices
                .push(format!("Stored snippets couldn't be read: {problem}"));
        }
    }
}

impl Prefs {
    pub fn reflect_snippets(&mut self) {
        self.snippets.reflect(&self.config);
    }

    pub(super) fn add_snippet(&mut self) {
        self.snippets.confirming = None;
        self.snippets.editor = Some(Editor {
            replaces: None,
            name: TextEdit::new(""),
            keyword: TextEdit::new(""),
            text: TextEdit::new(""),
            problems: Problems::default(),
        });
    }

    fn edit_snippet(&mut self, keyword: &str) {
        let Some((snippet, _)) = self
            .snippets
            .listed
            .iter()
            .find(|(snippet, source)| *source == Source::Stored && snippet.keyword == keyword)
        else {
            return;
        };
        self.snippets.confirming = None;
        self.snippets.editor = Some(Editor {
            replaces: Some(snippet.keyword.clone()),
            name: TextEdit::new(snippet.name.as_deref().unwrap_or_default()),
            keyword: TextEdit::new(&snippet.keyword),
            text: TextEdit::new(&snippet.text),
            problems: Problems::default(),
        });
    }

    fn save_snippet(&mut self) {
        let Some(editor) = &self.snippets.editor else {
            return;
        };
        if self.snippets.busy {
            return;
        }
        let draft = Snippet {
            keyword: editor.keyword.text.trim().to_owned(),
            name: Some(editor.name.text.clone()),
            text: editor.text.text.clone(),
        };
        let replaces = editor.replaces.clone();
        self.snippets.busy = true;
        let me = self.me.clone();
        std::thread::spawn(move || {
            let saved = crate::snippets::library().save(draft, replaces.as_deref());
            me.update(move |s, _| s.snippet_saved(saved));
        });
    }

    fn snippet_saved(&mut self, saved: Result<Problems, String>) {
        self.snippets.busy = false;
        match saved {
            Ok(problems) if problems.is_empty() => {
                self.snippets.editor = None;
                self.snippets.outcome = Some("Saved the snippet.".into());
            }
            Ok(problems) => {
                if let Some(editor) = &mut self.snippets.editor {
                    editor.problems = problems;
                }
            }
            Err(problem) => self.snippets.outcome = Some(problem),
        }
    }

    fn delete_snippet(&mut self, keyword: String) {
        if self.snippets.confirming.as_deref() != Some(keyword.as_str()) {
            self.snippets.confirming = Some(keyword);
            return;
        }
        if self.snippets.busy {
            return;
        }
        self.snippets.confirming = None;
        self.snippets.busy = true;
        let me = self.me.clone();
        std::thread::spawn(move || {
            let deleted = crate::snippets::library().delete(&keyword);
            me.update(move |s, _| {
                s.snippets.busy = false;
                s.snippets.outcome = Some(match deleted {
                    Ok(_) => "Deleted the snippet.".into(),
                    Err(problem) => problem,
                });
            });
        });
    }

    /// Stores config.toml's snippets, then removes the ones stored from config.toml, which
    /// only a writable file allows: a locked `snippets` key is never offered the move.
    pub(super) fn move_snippets(&mut self) {
        if self.locked("snippets") || self.snippets.busy || self.config.snippets.is_empty() {
            return;
        }
        let declared = self.config.snippets.clone();
        self.snippets.busy = true;
        let me = self.me.clone();
        std::thread::spawn(move || {
            let moved = crate::snippets::library().import(&declared);
            me.update(move |s, _| s.snippets_moved(moved));
        });
    }

    fn snippets_moved(&mut self, moved: Result<Vec<String>, String>) {
        self.snippets.busy = false;
        let moved = match moved {
            Ok(moved) => moved,
            Err(problem) => {
                self.snippets.outcome = Some(problem);
                return;
            }
        };
        let count = moved.len();
        self.set(|config| {
            for keyword in &moved {
                config.snippets.remove(keyword);
            }
        });
        let left = self.config.snippets.len();
        self.snippets.outcome = Some(match (count, left) {
            (1, 0) => "Moved the snippet to encrypted storage.".into(),
            (_, 0) => format!("Moved {count} snippets to encrypted storage."),
            _ => format!(
                "Moved {count} of {} snippets to encrypted storage; the others are invalid and stay in config.toml.",
                count + left
            ),
        });
        self.reflect_snippets();
    }

    #[cfg(test)]
    pub(super) fn snippets_busy(&self) -> bool {
        self.snippets.busy
    }

    #[cfg(test)]
    pub(super) fn snippet_rows(&self) -> Vec<(String, Source)> {
        self.snippets
            .listed
            .iter()
            .map(|(snippet, source)| (snippet.keyword.clone(), *source))
            .collect()
    }
}

/// The text on one line, cut to fit a row's description.
fn preview(text: &str) -> String {
    let line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > PREVIEW_CHARS {
        let cut: String = line.chars().take(PREVIEW_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        line
    }
}

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let mut body = Vec::new();
    for (index, line) in s.snippets.outcome.iter().chain(&s.snippets.notices).enumerate() {
        body.push(
            text(line)
                .key(format!("snippets-notice-{index}"))
                .w_full()
                .mx(ROW_PX)
                .font(DESCRIPTION)
                .color(k.c.text_base)
                .wrap(),
        );
    }
    match &s.snippets.editor {
        Some(editor) => body.push(card(k, hair, editor_rows(s, k, editor))),
        None => body.push(
            row().w_full().px(ROW_PX).child(
                Button::new("Add Snippet")
                    .disabled(s.snippets.busy)
                    .on_press(Prefs::add_snippet)
                    .build(k)
                    .key("add-snippet"),
            ),
        ),
    }
    if s.snippets.listed.is_empty() {
        body.push(note(
            k,
            "No snippets yet. Add one here, or type snip, then Space, in the launcher.",
        ));
    } else {
        body.push(card(k, hair, snippet_rows(s, k)));
    }
    body.push(note(
        k,
        "Snippets you add are stored in Dupin, encrypted. Type a snippet's keyword or name after snip, then Space, to find it; {date}, {time}, {datetime} and {clipboard} are filled in when it's used.",
    ));
    if !s.config.snippets.is_empty() {
        body.push(card(k, hair, vec![move_row(s, k)]));
    }
    body
}

fn snippet_rows(s: &Prefs, k: &Kit) -> Vec<El> {
    s.snippets
        .listed
        .iter()
        .map(|(snippet, source)| {
            let title = snippet.name.clone().unwrap_or_else(|| snippet.keyword.clone());
            let detail = format!("{}  ·  {}", snippet.keyword, preview(&snippet.text));
            let key = &snippet.keyword;
            match source {
                Source::Config => setting_with_lock(
                    k,
                    &icons::QUOTE,
                    &title,
                    Some(&detail),
                    el(),
                    Some("From config.toml"),
                ),
                Source::Stored => {
                    let confirming = s.snippets.confirming.as_deref() == Some(key.as_str());
                    let (edited, deleted) = (key.clone(), key.clone());
                    let controls = row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            Button::new("Edit")
                                .disabled(s.snippets.busy)
                                .on_press(move |s: &mut Prefs| s.edit_snippet(&edited))
                                .build(k)
                                .key(format!("edit-snippet-{key}")),
                        )
                        .child(
                            Button::new(if confirming { "Confirm Delete" } else { "Delete" })
                                .disabled(s.snippets.busy)
                                .on_press(move |s: &mut Prefs| s.delete_snippet(deleted.clone()))
                                .build(k)
                                .key(format!("delete-snippet-{key}")),
                        );
                    setting(k, &icons::QUOTE, &title, Some(&detail), controls)
                }
            }
        })
        .collect()
}

fn editor_rows(s: &Prefs, k: &Kit, editor: &Editor) -> Vec<El> {
    let field = |label: &str,
                 key: &str,
                 value: &TextEdit,
                 placeholder: &str,
                 multiline: bool,
                 problem: &Option<String>,
                 edit: fn(&mut Prefs) -> &mut TextEdit| {
        let input = text_input(value, k.ty.ui)
            .key(key.to_owned())
            .label(label.to_owned())
            .placeholder(placeholder, k.c.text_muted)
            .color(k.c.text_base)
            .px(8.0)
            .py(4.0)
            .radius(k.radius.sm)
            .fill(k.c.layer_02)
            .on_edit(edit);
        let input = if multiline {
            auto_grow(input, 4, 10)
        } else {
            input.single_line()
        };
        let mut column = col()
            .key(format!("{key}-box"))
            .w_full()
            .px(ROW_PX)
            .py(6.0)
            .gap(4.0)
            .child(text(label.to_owned()).font(k.ty.ui).color(k.c.text_base))
            .child(input);
        if let Some(problem) = problem {
            column = column.child(
                text(problem.clone())
                    .key(format!("{key}-problem"))
                    .font(DESCRIPTION)
                    .color(k.c.text_base)
                    .wrap(),
            );
        }
        column
    };
    vec![
        field(
            "Name",
            "snippet-name",
            &editor.name,
            "Shown in place of the keyword",
            false,
            &editor.problems.name,
            editor_name,
        ),
        field(
            "Keyword",
            "snippet-keyword",
            &editor.keyword,
            "Finds it, without spaces",
            false,
            &editor.problems.keyword,
            editor_keyword,
        ),
        field(
            "Text",
            "snippet-text",
            &editor.text,
            "Fills in {date}, {time}, {datetime} and {clipboard}",
            true,
            &editor.problems.text,
            editor_text,
        ),
        row()
            .w_full()
            .px(ROW_PX)
            .py(8.0)
            .gap(8.0)
            .child(
                Button::new("Save Snippet")
                    .disabled(s.snippets.busy)
                    .on_press(Prefs::save_snippet)
                    .build(k)
                    .key("save-snippet"),
            )
            .child(
                Button::new("Cancel")
                    .on_press(|s: &mut Prefs| s.snippets.editor = None)
                    .build(k)
                    .key("cancel-snippet"),
            ),
    ]
}

fn editor_name(s: &mut Prefs) -> &mut TextEdit {
    &mut s.snippets.editor.as_mut().expect("the editor is open").name
}

fn editor_keyword(s: &mut Prefs) -> &mut TextEdit {
    &mut s.snippets.editor.as_mut().expect("the editor is open").keyword
}

fn editor_text(s: &mut Prefs) -> &mut TextEdit {
    &mut s.snippets.editor.as_mut().expect("the editor is open").text
}

/// Moving config.toml's snippets into storage, offered only while the file holding them can
/// be written; a read-only one, such as Nix's, keeps them and says so.
fn move_row(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("snippets");
    let control = if locked {
        el()
    } else {
        Button::new("Move to Encrypted Storage")
            .disabled(s.snippets.busy)
            .on_press(Prefs::move_snippets)
            .build(k)
            .key("move-snippets")
    };
    let detail = if locked {
        "config.toml is read-only, so its snippets stay there and take the place of a stored one with the same keyword."
    } else {
        "Stores config.toml's snippets in Dupin, encrypted, and removes them from config.toml, so they can be edited here and in the launcher."
    };
    setting_with_lock(
        k,
        &icons::QUOTE,
        "config.toml's snippets",
        Some(detail),
        control,
        s.lock_note(locked),
    )
}
