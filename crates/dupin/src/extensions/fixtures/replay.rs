//! A fixture's steps replayed as the launcher would take them: queries under the keyword or on
//! a page a view opened, rows chosen, actions run and the effects they return, pages opened and
//! closed, and forms filled in and submitted, with what a person would press for each step, so
//! `render` drives the headless launcher the same way.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use dupin_core::form::{self, FieldKind, Form};
use dupin_ext_sdk::fixture::{self, Action, Fixture};
use dupin_extension_bridge::{Call, ClipboardAccess, Request, Trigger};
use dupin_extension_host::{ActionEffect, Host};
use dupin_js_runtime::row::{Row, RowAction, RowEffect};
use dupin_js_runtime::{Diagnostic, PageContext, Preferences, RunError, ViewAnswer};
use serde_json::{Map, Value, json};

use super::super::provider::{DETAIL_ROW, form_of, row_item};
use super::super::{ACTION_DEADLINE, Extension, QUERY_DEADLINE, Role};
use super::{Held, Press, Replay, StepResult, fixture_host, guest_environment, query_rows, record, services};

/// The most pages the launcher keeps open above the keyword's own results.
const MAX_DEPTH: usize = 8;

/// A page a view opened, as the launcher holds it.
struct Page {
    view: String,
    title: String,
    state: Value,
    query: String,
    /// The row selected on the page below when this one opened, by ID, which closing it selects
    /// again wherever it has moved to.
    below: Option<String>,
    answer: Option<ViewAnswer>,
    form: Option<Drafts>,
}

/// A form's drafts, as the launcher keeps them.
struct Drafts {
    form: Form,
    values: Vec<form::Value>,
    errors: BTreeMap<String, String>,
    /// The field focused, by index.
    focus: usize,
}

impl Drafts {
    /// Shows `errors`, focusing the first field that has one, as the launcher does.
    fn show_errors(&mut self, errors: BTreeMap<String, String>) {
        if let Some(first) = self
            .form
            .fields
            .iter()
            .position(|field| errors.contains_key(&field.id))
        {
            self.focus = first;
        }
        self.errors = errors;
    }
}

impl Page {
    fn new(view: String, title: String, state: Option<Value>) -> Self {
        Self {
            view,
            title,
            state: state.unwrap_or_default(),
            query: String::new(),
            below: None,
            answer: None,
            form: None,
        }
    }

    fn context(&self) -> PageContext {
        PageContext {
            query: self.query.clone(),
            state: self.state.clone(),
        }
    }

    /// The page as fixtures state it.
    fn outcome(&self, depth: usize) -> Value {
        let mut out = Map::new();
        out.insert("name".into(), self.view.clone().into());
        out.insert("title".into(), self.title.clone().into());
        out.insert("depth".into(), depth.into());
        if !self.state.is_null() {
            out.insert("state".into(), self.state.clone());
        }
        let kind = match &self.answer {
            Some(ViewAnswer::List { empty_text, .. }) => {
                if let Some(text) = empty_text {
                    out.insert("emptyText".into(), text.clone().into());
                }
                "list"
            }
            Some(ViewAnswer::Detail { .. }) => "detail",
            Some(ViewAnswer::Form(_)) => "form",
            None => "none",
        };
        out.insert("type".into(), kind.into());
        if let Some(drafts) = &self.form {
            out.insert("submit".into(), drafts.form.submit_title.clone().into());
            let fields: Vec<Value> = drafts
                .form
                .fields
                .iter()
                .zip(&drafts.values)
                .map(|(field, value)| {
                    let (kind, value) = match (&field.kind, value) {
                        (FieldKind::Text { multiline, .. }, form::Value::Text(text)) => (
                            if *multiline { "textarea" } else { "text" },
                            Value::from(text.clone()),
                        ),
                        (FieldKind::Checkbox { .. }, form::Value::Checkbox(on)) => {
                            ("checkbox", Value::from(*on))
                        }
                        (FieldKind::Select { options, .. }, form::Value::Select(index)) => (
                            "select",
                            options
                                .get(*index)
                                .map_or(Value::Null, |(value, _)| value.clone().into()),
                        ),
                        _ => ("unknown", Value::Null),
                    };
                    let mut entry = Map::new();
                    entry.insert("id".into(), field.id.clone().into());
                    entry.insert("type".into(), kind.into());
                    entry.insert("title".into(), field.title.clone().into());
                    entry.insert("value".into(), value);
                    if field.required() {
                        entry.insert("required".into(), true.into());
                    }
                    if let Some(error) = drafts.errors.get(&field.id) {
                        entry.insert("error".into(), error.clone().into());
                    }
                    Value::Object(entry)
                })
                .collect();
            out.insert("fields".into(), fields.into());
        }
        Value::Object(out)
    }
}

/// The launcher's state as a fixture replays it.
struct Replayer<'a> {
    extension: &'a Extension,
    host: Host,
    world: services::Fixture,
    preferences: Arc<Preferences>,
    held: Held<'a>,
    cancelled: &'a dyn Fn() -> bool,
    /// What is typed under the keyword.
    query: Option<String>,
    pages: Vec<Page>,
    rows: Vec<Row>,
    selected: Option<usize>,
    /// Whether the step asked again for what shows.
    changed: bool,
}

/// What a step did, beyond its outcome.
#[derive(Default)]
struct Done {
    problem: Option<String>,
    presses: Vec<Press>,
    /// What shows changed, so the outcome states it.
    shown: bool,
}

impl Done {
    fn problem(problem: String) -> Self {
        Self {
            problem: Some(problem),
            ..Self::default()
        }
    }

    fn pressing(presses: Vec<Press>) -> Self {
        Self {
            presses,
            shown: true,
            ..Self::default()
        }
    }
}

/// Runs the steps in order through the fixture's host and broker bridge, stopping at the first
/// that can't run.
pub(super) fn replay(
    extension: &Extension,
    name: &str,
    fixture: &Fixture,
    cancelled: &dyn Fn() -> bool,
) -> Result<Replay, Diagnostic> {
    let mut replayer = Replayer {
        extension,
        host: fixture_host(name, fixture)?,
        world: services::Fixture::open(
            extension,
            name,
            &fixture.replies,
            &fixture.preferences,
            guest_environment(fixture).now,
        )?,
        preferences: Arc::new(fixture.preferences.clone()),
        held: super::Held::new(extension, guest_environment(fixture).now),
        cancelled,
        query: None,
        pages: Vec::new(),
        rows: Vec::new(),
        selected: None,
        changed: false,
    };
    let mut steps = Vec::new();
    for step in &fixture.steps {
        let action = step.action();
        let mut outcome = Map::new();
        let done = replayer.step(&action, &mut outcome);
        let effects = replayer.world.take_effects();
        if !effects.clipboard.is_empty() {
            outcome.insert("clipboard".into(), effects.clipboard.into());
        }
        if !effects.windows.is_empty() {
            outcome.insert(
                "windows".into(),
                serde_json::to_value(effects.windows).unwrap_or_default(),
            );
        }
        if let Some(storage) = effects.storage {
            outcome.insert("storage".into(), storage);
        }
        let differences = match (&done.problem, &step.expect) {
            (None, Some(expected)) => fixture::compare(expected, &outcome),
            _ => Vec::new(),
        };
        let stop = done.problem.is_some();
        steps.push(StepResult {
            action: action.describe(),
            outcome,
            differences,
            problem: done.problem,
            presses: done.presses,
        });
        if stop {
            break;
        }
    }
    Ok(Replay { steps })
}

impl Replayer<'_> {
    fn call(&self, trigger: Trigger) -> Call {
        self.world.bridge.call(&self.extension.package, trigger)
    }

    fn step(&mut self, action: &Action<'_>, outcome: &mut Map<String, Value>) -> Done {
        self.changed = false;
        let done = match action {
            Action::Query(query) => {
                let presses = self.retype(query);
                match self.pages.last_mut() {
                    Some(page) => page.query = (*query).to_owned(),
                    None => self.query = Some((*query).to_owned()),
                }
                self.answer(outcome);
                self.selected = (!self.rows.is_empty()).then_some(0);
                Done::pressing(presses)
            }
            Action::Select(index) if *index < self.rows.len() => {
                let from = self.selected.unwrap_or(0);
                self.selected = Some(*index);
                let (key, count) = if *index >= from {
                    ("Down", index - from)
                } else {
                    ("Up", from - index)
                };
                outcome.insert("selected".into(), (*index).into());
                Done {
                    presses: vec![Press::key(key); count],
                    ..Done::default()
                }
            }
            Action::Select(index) => Done::problem(format!(
                "there is no row {index} to select; the answer has {} {}",
                self.rows.len(),
                if self.rows.len() == 1 { "row" } else { "rows" }
            )),
            Action::Activate(name) => self.activate(name, outcome),
            Action::Event(event, data) => {
                let handled = self.host.event(
                    &self.extension.compiled,
                    event,
                    data,
                    &self.extension.preferences_with(&self.preferences),
                    Instant::now() + ACTION_DEADLINE,
                    self.cancelled,
                    &self.call(Trigger::Background).services,
                );
                let mut done = Done::default();
                match handled {
                    Ok(handled) if handled.invalidated => {
                        self.held.cached.invalidate();
                        outcome.insert("invalidated".into(), true.into());
                    }
                    Ok(_) => {}
                    Err(error) => done.problem = record(error, outcome),
                }
                // A watch runs in the background: what shows changes only on the next search.
                return done;
            }
            Action::Back => {
                if self.pages.is_empty() {
                    return Done::problem("there is no page to close".into());
                }
                self.close(outcome);
                Done::pressing(vec![Press::key("Escape")])
            }
            Action::Fill(fields) => self.fill(fields),
            Action::Submit => self.submit(outcome),
        };
        if done.problem.is_none() && (done.shown || self.changed) {
            self.describe(outcome);
        }
        done
    }

    /// The rows and page on screen, as a step's outcome states them.
    fn describe(&self, outcome: &mut Map<String, Value>) {
        if let Some(page) = self.pages.last() {
            outcome.insert("view".into(), page.outcome(self.pages.len()));
        }
        outcome.entry("rows").or_insert_with(|| fixture::rows(&self.rows));
        if let Some(index) = self.selected {
            outcome.insert("selected".into(), index.into());
        }
    }

    /// What a person presses to replace what the query field holds with `query`: under the
    /// keyword, or in the main search for a package a built-in answering there ships as, where a
    /// leading space can't be typed, since it opens file search.
    fn retype(&self, query: &str) -> Vec<Press> {
        let main_search = crate::features::ported_role(&self.extension.id) == Role::MainSearch;
        let query = if main_search { query.trim_start() } else { query };
        let mut presses = Vec::new();
        let typed = match self.pages.last() {
            Some(page) => page.query.as_str(),
            None => match &self.query {
                None => match &self.extension.keyword {
                    Some(keyword) if !main_search => {
                        return vec![Press::Type(format!("{keyword} {query}"))];
                    }
                    _ => "",
                },
                Some(typed) => typed.as_str(),
            },
        };
        // The keyword is a chip once typed, so only the text after it is replaced; an empty
        // field is left alone, since ⌫ there would leave the keyword or the page.
        if !typed.is_empty() {
            presses.push(Press::command("a"));
            presses.push(Press::key("Backspace"));
        }
        if !query.is_empty() {
            presses.push(Press::Type(query.to_owned()));
        }
        presses
    }

    /// Asks again for what shows now: the page on top's view, or the keyword's search.
    fn answer(&mut self, outcome: &mut Map<String, Value>) {
        self.changed = true;
        let answered = match self.pages.last() {
            Some(_) => self.render(),
            None => query_rows(
                &self.host,
                self.extension,
                &self.held,
                self.query.as_deref().unwrap_or_default(),
                &self.preferences,
                &self.call(Trigger::Query),
                self.cancelled,
            ),
        };
        self.rows = match answered {
            Ok(rows) => rows,
            Err(error) => {
                // A page that fails keeps its last answer out of the outcome, as the launcher
                // shows the failure instead.
                if record(error, outcome).is_none()
                    && let Some(page) = self.pages.last_mut()
                {
                    page.answer = None;
                    page.form = None;
                }
                Vec::new()
            }
        };
        if self.selected.is_some_and(|index| index >= self.rows.len()) {
            self.selected = (!self.rows.is_empty()).then_some(0);
        }
    }

    /// Asks the page on top's view, keeping a form's drafts while its fields stay the same.
    fn render(&mut self) -> Result<Vec<Row>, RunError> {
        let page = self.pages.last().expect("a page is open");
        let answer = self.host.view(
            &self.extension.compiled,
            &page.view,
            &page.context(),
            &self.extension.preferences_with(&self.preferences),
            Instant::now() + QUERY_DEADLINE,
            self.cancelled,
            &self.call(Trigger::Query).services,
        )?;
        let page = self.pages.last_mut().expect("a page is open");
        let rows = match &answer {
            ViewAnswer::List { rows, .. } => {
                page.form = None;
                rows.clone()
            }
            ViewAnswer::Detail {
                title,
                detail,
                actions,
            } => {
                page.form = None;
                vec![Row {
                    detail: Some(detail.clone()),
                    actions: actions.clone(),
                    ..Row::new(DETAIL_ROW, title.clone())
                }]
            }
            ViewAnswer::Form(spec) => {
                let form = form_of(self.extension, spec.clone());
                let kept = page.form.take().filter(|drafts| {
                    drafts.form.fields.len() == form.fields.len()
                        && drafts
                            .form
                            .fields
                            .iter()
                            .zip(&form.fields)
                            .all(|(old, new)| old.id == new.id)
                        && form.fits(&drafts.values)
                });
                page.form = Some(match kept {
                    Some(drafts) => Drafts { form, ..drafts },
                    None => Drafts {
                        values: form.fields.iter().map(form::Field::default_value).collect(),
                        form,
                        errors: BTreeMap::new(),
                        focus: 0,
                    },
                });
                Vec::new()
            }
        };
        page.answer = Some(answer);
        Ok(rows)
    }

    /// Opens `page` above what shows, as the launcher does.
    fn open(&mut self, mut page: Page, outcome: &mut Map<String, Value>) {
        if self.pages.len() >= MAX_DEPTH {
            outcome.insert(
                "error".into(),
                fixture::error(&Diagnostic::new(
                    "too-deep",
                    format!("Dupin keeps at most {MAX_DEPTH} views open"),
                )),
            );
            return;
        }
        page.below = self
            .selected
            .and_then(|index| self.rows.get(index))
            .map(|row| row.id.clone());
        self.pages.push(page);
        self.answer(outcome);
        self.selected = (!self.rows.is_empty()).then_some(0);
    }

    /// Closes the page on top, selecting again what was selected below it.
    fn close(&mut self, outcome: &mut Map<String, Value>) {
        let closed = self.pages.pop().expect("a page is open");
        self.answer(outcome);
        self.selected = closed
            .below
            .and_then(|id| self.rows.iter().position(|row| row.id == id))
            .or((!self.rows.is_empty()).then_some(0));
    }

    fn activate(&mut self, name: &str, outcome: &mut Map<String, Value>) -> Done {
        let row = self.selected.and_then(|index| self.rows.get(index)).cloned();
        // The IDs the launcher gives the row's actions, which a step may name instead of a
        // title, and where each one sits in the row's action panel.
        let chosen = row.as_ref().and_then(|row| {
            let shown = row_item(self.extension, row.clone(), false);
            row.actions
                .iter()
                .zip(&shown.item.actions)
                .enumerate()
                .find(|(_, (action, item))| action.title == name || item.id == name)
                .map(|(index, (action, _))| (index, action.clone()))
        });
        let Some((index, action)) = chosen else {
            let offered: Vec<&str> = row
                .iter()
                .flat_map(|row| row.actions.iter().map(|action| action.title.as_str()))
                .collect();
            return Done::problem(if offered.is_empty() {
                format!("the selected row has no action {}", Value::from(name))
            } else {
                format!(
                    "the selected row has no action {}; it offers {}",
                    Value::from(name),
                    offered.join(", ")
                )
            });
        };
        outcome.insert("effect".into(), Value::Object(fixture::chosen(&action)));
        let mut done = Done::default();
        match &action.effect {
            RowEffect::Paste { text } => {
                let paste = Request::Clipboard(ClipboardAccess::Paste);
                // The launcher copies instead, so the fixture sees what the user would.
                if let Err(refused) =
                    self.world
                        .bridge
                        .authorize(&self.extension.package, &paste, Trigger::Activation)
                {
                    let copied = RowAction {
                        effect: RowEffect::Copy { text: text.clone() },
                        ..action.clone()
                    };
                    outcome.insert("effect".into(), Value::Object(fixture::chosen(&copied)));
                    outcome.insert(
                        "error".into(),
                        fixture::error(&Diagnostic::new(refused.code().to_owned(), refused.to_string())),
                    );
                }
            }
            RowEffect::Push { view, state } => {
                done.presses = choose(index, &action);
                let page = Page::new(view.clone(), action.title.clone(), state.clone());
                self.open(page, outcome);
            }
            RowEffect::Run { handler, data } => {
                done.presses = choose(index, &action);
                done.problem = self.run(handler, data.as_ref().unwrap_or(&Value::Null), outcome, false);
            }
            RowEffect::Copy { .. } | RowEffect::OpenUrl { .. } | RowEffect::Reveal { .. } => {}
        }
        done
    }

    /// Runs `actions[handler](data)` on the page on top and carries out the effect it returns.
    fn run(
        &mut self,
        handler: &str,
        data: &Value,
        outcome: &mut Map<String, Value>,
        submitting: bool,
    ) -> Option<String> {
        let page = self.pages.last().map(Page::context).unwrap_or_default();
        let acted = self.host.action(
            &self.extension.compiled,
            handler,
            data,
            &page,
            &self.extension.preferences_with(&self.preferences),
            Instant::now() + ACTION_DEADLINE,
            self.cancelled,
            &self.call(Trigger::Activation).services,
        );
        // Dupin hands the bundled snippets package its changed snippets before the action
        // that changed them answers, so what it shows next lists them.
        if let Some(snippets) = self.world.take_snippets() {
            let mut preferences = (*self.preferences).clone();
            preferences.extend(snippets);
            self.preferences = Arc::new(preferences);
        }
        if let Ok(acted) = &acted
            && acted.invalidated
        {
            self.held.cached.invalidate();
            outcome.insert("invalidated".into(), true.into());
        }
        let effect = match acted {
            Ok(acted) => acted.effect,
            Err(error) => return record(error, outcome),
        };
        if let Some(text) = effect.as_ref().and_then(ActionEffect::toast) {
            outcome.insert("toasts".into(), json!([text]));
        }
        let refuse = |outcome: &mut Map<String, Value>, message: &str| {
            outcome.insert(
                "error".into(),
                fixture::error(&Diagnostic::new("invalid-effect", message)),
            );
        };
        match effect {
            // The launcher searches again after an action; under the keyword that only matters
            // to a catalogue the action invalidated, which the next query step asks for.
            None | Some(ActionEffect::Toast { .. }) if !self.pages.is_empty() => self.answer(outcome),
            None | Some(ActionEffect::Toast { .. }) => {}
            Some(ActionEffect::Hide) => {
                outcome.insert("panel".into(), "hide".into());
            }
            Some(ActionEffect::Paste) => {
                outcome.insert("panel".into(), "paste".into());
            }
            Some(ActionEffect::Enter { command, .. }) if !self.extension.may_enter(&command) => {
                refuse(
                    outcome,
                    "only a package that ships with Dupin may enter another command",
                );
            }
            Some(ActionEffect::Enter { command, query }) => {
                outcome.insert(
                    "panel".into(),
                    json!({ "enter": { "command": command, "query": query } }),
                );
            }
            Some(ActionEffect::Push {
                view, title, state, ..
            }) => {
                let title = title.unwrap_or_else(|| self.extension.name.clone());
                self.open(Page::new(view, title, state), outcome);
            }
            Some(ActionEffect::Pop { .. }) if !self.pages.is_empty() => self.close(outcome),
            Some(ActionEffect::Pop { .. }) => {}
            Some(ActionEffect::Refresh { .. }) => self.answer(outcome),
            Some(ActionEffect::ReplaceState { state, .. }) => match self.pages.last_mut() {
                Some(page) => {
                    page.state = state;
                    self.answer(outcome);
                }
                None => refuse(outcome, "replaceState changes only a page a view opened"),
            },
            Some(ActionEffect::FieldErrors { errors }) => {
                match self.pages.last_mut().and_then(|page| page.form.as_mut()) {
                    Some(drafts) if submitting => {
                        let known = errors
                            .into_iter()
                            .filter(|(id, _)| drafts.form.fields.iter().any(|field| field.id == *id))
                            .collect();
                        drafts.show_errors(known);
                    }
                    _ => refuse(outcome, "fieldErrors answers only a form's submit"),
                }
            }
        }
        None
    }

    fn fill(&mut self, fields: &Map<String, Value>) -> Done {
        let Some(drafts) = self.pages.last_mut().and_then(|page| page.form.as_mut()) else {
            return Done::problem("there is no form on screen to fill in".into());
        };
        let mut presses = Vec::new();
        // Fills move through the form in order, from the field focused.
        let mut focus = drafts.focus;
        let count = drafts.form.fields.len();
        for (index, field) in drafts.form.fields.iter().enumerate() {
            let Some(wanted) = fields.get(&field.id) else {
                continue;
            };
            let value = match (&field.kind, wanted) {
                (FieldKind::Text { .. }, Value::String(text)) => form::Value::Text(text.clone()),
                (FieldKind::Checkbox { .. }, Value::Bool(on)) => form::Value::Checkbox(*on),
                (FieldKind::Select { options, .. }, Value::String(chosen)) => {
                    match options.iter().position(|(value, _)| value == chosen) {
                        Some(at) => form::Value::Select(at),
                        None => {
                            return Done::problem(format!(
                                "`{}` has no option {}",
                                field.id,
                                Value::from(chosen.as_str())
                            ));
                        }
                    }
                }
                _ => {
                    return Done::problem(format!(
                        "`{}` takes {}",
                        field.id,
                        match field.kind {
                            FieldKind::Text { .. } => "text",
                            FieldKind::Checkbox { .. } => "`true` or `false`",
                            FieldKind::Select { .. } => "one of its options' values",
                        }
                    ));
                }
            };
            presses.extend(std::iter::repeat_n(
                Press::key("Tab"),
                (index + count - focus) % count,
            ));
            focus = index;
            match (&drafts.values[index], &value) {
                (form::Value::Text(before), form::Value::Text(text)) => {
                    if !before.is_empty() {
                        presses.push(Press::command("a"));
                        presses.push(Press::key("Backspace"));
                    }
                    for (index, line) in text.split('\n').enumerate() {
                        if index > 0 {
                            presses.push(Press::key("Enter"));
                        }
                        if !line.is_empty() {
                            presses.push(Press::Type(line.to_owned()));
                        }
                    }
                }
                (form::Value::Checkbox(before), form::Value::Checkbox(on)) if before != on => {
                    presses.push(Press::Type(" ".into()));
                }
                (form::Value::Select(before), form::Value::Select(at)) => {
                    let options = match &field.kind {
                        FieldKind::Select { options, .. } => options.len(),
                        _ => 1,
                    };
                    presses.extend(std::iter::repeat_n(
                        Press::key("Right"),
                        (at + options - before) % options,
                    ));
                }
                _ => {}
            }
            drafts.values[index] = value;
            drafts.errors.remove(&field.id);
        }
        if let Some(unknown) = fields
            .keys()
            .find(|id| !drafts.form.fields.iter().any(|field| field.id == **id))
        {
            return Done::problem(format!("the form has no field `{unknown}`"));
        }
        // Back to the first field, where a form opens with its focus.
        presses.extend(std::iter::repeat_n(Press::key("Tab"), (count - focus) % count));
        drafts.focus = 0;
        Done::pressing(presses)
    }

    fn submit(&mut self, outcome: &mut Map<String, Value>) -> Done {
        let Some(drafts) = self.pages.last_mut().and_then(|page| page.form.as_mut()) else {
            return Done::problem("there is no form on screen to submit".into());
        };
        let presses = vec![Press::command_key("Enter")];
        match drafts.form.submission(&drafts.values) {
            Err(errors) => {
                drafts.show_errors(errors);
                Done::pressing(presses)
            }
            Ok(values) => {
                drafts.errors.clear();
                let handler = drafts.form.submit.clone();
                let problem = self.run(&handler, &Value::Object(values), outcome, true);
                Done {
                    problem,
                    presses,
                    shown: true,
                }
            }
        }
    }
}

/// What a person presses to choose the row's action at `index` from its action panel.
fn choose(index: usize, action: &RowAction) -> Vec<Press> {
    let mut presses = vec![Press::command("k")];
    presses.extend(std::iter::repeat_n(Press::key("Down"), index));
    presses.push(Press::key("Enter"));
    if action.confirm.is_some() {
        presses.push(Press::key("Down"));
        presses.push(Press::key("Enter"));
    }
    presses
}
