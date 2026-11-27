use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};

use dupin_core::catalogue::Entry;
use dupin_core::form::{FieldKind, Form};
use dupin_core::kind::{Kinds, Producer, Subject};
use dupin_core::{
    Accessory, Action, Batch, Detail, ExtensionRequest, HostImage, HostImageSource, Icon, Item, ItemAction,
    Page, Payload, Provider, ProviderError, Request, RowState, View, snippets,
};
use dupin_extension_bridge::{Bridge, Call, ConsentReason, ConsentRequest, PackageId, Trigger};
use dupin_js_runtime::row::{self, Row, RowEffect, RowImage};
use dupin_js_runtime::view::{self, ViewAnswer};
use dupin_js_runtime::{PageContext, RunError};

use super::Extension;
use super::catalogue::Convert;

/// Shorter than the command's session deadline, so a slow guest ends with its own error row
/// rather than the generic expiry.
pub const QUERY_DEADLINE: Duration = Duration::from_secs(2);
/// An action the user chose may wait on the network or a program for longer than typing may.
pub const ACTION_DEADLINE: Duration = Duration::from_secs(30);

/// Ranks what `search` computed for the query above the catalogue's matches, in its own order,
/// while every row still carries a score, so usage ranking applies to the catalogue's rows.
const COMPUTED_SCORE: i32 = 1_000_000;

/// Which search an extension's provider answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Its own keyword: `search` and the catalogue.
    Keyword,
    /// The main search, from the catalogue alone, so no keystroke reaches the package.
    DefaultSearch,
    /// The main search, through `search` alone, for the built-in the package answers there.
    /// What is typed reaches it only while it holds the `query` scope, and the main search
    /// shows only its rows: never a question, a failure or a catalogue.
    MainSearch,
}

pub struct ExtensionProvider {
    extension: Arc<Extension>,
    role: Role,
    bridge: Option<Bridge>,
}

impl ExtensionProvider {
    /// A provider for the keyword whose guest reaches no host service.
    #[cfg(test)]
    pub fn new(extension: Arc<Extension>) -> Self {
        Self::with_role(extension, Role::Keyword)
    }

    /// A provider whose guest reaches no host service.
    #[cfg(test)]
    pub fn with_role(extension: Arc<Extension>, role: Role) -> Self {
        Self {
            extension,
            role,
            bridge: None,
        }
    }

    /// A provider whose guest's requests go through `bridge`, as typing in the launcher.
    pub fn with_bridge(extension: Arc<Extension>, role: Role, bridge: Bridge) -> Self {
        Self {
            extension,
            role,
            bridge: Some(bridge),
        }
    }

    fn problem(&self, message: String) -> Item {
        Item {
            id: PROBLEM_ROW.into(),
            payload: Payload::default(),
            view: View::Text {
                title: format!("{} failed", self.extension.name),
                subtitle: message,
            },
            record_usage: false,
            icon: None,
            matched: Vec::new(),
            completion: None,
            score: None,
            primary: None,
            alternate: None,
            actions: Vec::new(),
        }
    }
}

/// The rows `extension` answers `query` with under its keyword: what `search` computes for it,
/// then the catalogue's matches, in batches by section. Both answer their host requests through
/// `call`.
pub fn answer(
    extension: &Arc<Extension>,
    query: &str,
    page: Option<&Page>,
    limit: usize,
    deadline: Instant,
    is_cancelled: impl Fn() -> bool + 'static,
    call: &Call,
) -> Result<Vec<Batch>, RunError> {
    if let Some(page) = page {
        return page_answer(extension, page, query, deadline, is_cancelled, call);
    }
    let exports = extension.check(deadline)?;
    let mut batches = Vec::new();
    // Like any keyword command, `search` runs once there is something to search for, unless
    // the package has something to show before that.
    if exports.search && (extension.empty_query || !query.trim().is_empty()) {
        let scored = exports.items;
        let rows = extension.query(query, limit, deadline, is_cancelled, &call.services)?;
        let rows = rows.into_iter().enumerate().map(|(index, row)| {
            let mut shown = row_item(extension, row, false);
            if scored {
                shown.item.score = Some(COMPUTED_SCORE - index as i32);
            }
            shown
        });
        batches.extend(sectioned(rows));
    }
    if exports.items {
        let left = limit.saturating_sub(batches.iter().map(|batch| batch.items.len()).sum());
        batches.extend(extension.catalogue(query, left, deadline, call, Role::Keyword, Some(extension))?);
        let left = limit.saturating_sub(batches.iter().map(|batch| batch.items.len()).sum());
        let searches: Vec<Item> = extension
            .web_searches(deadline, call, Role::Keyword)?
            .iter()
            .filter_map(|(item, search)| crate::web::item(item, search, query.trim(), false))
            .take(left)
            .collect();
        if !searches.is_empty() {
            batches.push(Batch::from(searches));
        }
    }
    Ok(batches)
}

/// The row whose ID is `row` in `extension`'s catalogue, as its keyword lists it; none when
/// the package lists no such row.
pub fn catalogue_row(
    extension: &Arc<Extension>,
    row: &str,
    deadline: Instant,
    call: &Call,
) -> Result<Option<Item>, RunError> {
    if !extension.check(deadline)?.items {
        return Ok(None);
    }
    let id = extension.entry_id(row);
    let batches = extension.catalogue("", usize::MAX, deadline, call, Role::Keyword, Some(extension))?;
    Ok(batches
        .into_iter()
        .flat_map(|batch| batch.items)
        .find(|item| item.id == id))
}

/// What `search` computes for what is typed in the main search, for the built-in `extension`
/// answers there.
fn main_answer(
    extension: &Extension,
    query: &str,
    limit: usize,
    deadline: Instant,
    is_cancelled: impl Fn() -> bool + 'static,
    call: &Call,
) -> Result<Vec<Batch>, RunError> {
    if query.trim().is_empty() || !extension.check(deadline)?.search {
        return Ok(Vec::new());
    }
    let rows = extension.query(query, limit, deadline, is_cancelled, &call.services)?;
    Ok(sectioned(
        rows.into_iter().map(|row| row_item(extension, row, false)),
    ))
}

/// The catalogue's matches for the main search. The package never sees the query, but loading
/// its catalogue reaches host services through `call` as it does under the keyword.
pub fn default_answer(
    extension: &Arc<Extension>,
    query: &str,
    limit: usize,
    deadline: Instant,
    call: &Call,
) -> Result<Vec<Batch>, RunError> {
    if query.trim().is_empty() || !extension.check(deadline)?.items {
        return Ok(Vec::new());
    }
    extension.catalogue(query, limit, deadline, call, Role::DefaultSearch, Some(extension))
}

/// What a page of the package's shows with `query` typed on it: what its view answered,
/// with the state the page holds.
fn page_answer(
    extension: &Extension,
    page: &Page,
    query: &str,
    deadline: Instant,
    is_cancelled: impl Fn() -> bool + 'static,
    call: &Call,
) -> Result<Vec<Batch>, RunError> {
    let context = PageContext {
        query: query.to_owned(),
        state: page.context.clone().unwrap_or_default(),
    };
    let answer = extension.view(&page.view, &context, deadline, is_cancelled, &call.services)?;
    Ok(view_batches(extension, answer))
}

/// The row a detail view shows as, which carries the view's actions.
pub(super) const DETAIL_ROW: &str = "detail";
/// The row a form view shows as.
pub(super) const FORM_ROW: &str = "form";

/// A view's answer as the batches the session shows: a list's rows, a detail as one card that
/// stays open, or a form as one row the page shows in its place.
pub(super) fn view_batches(extension: &Extension, answer: ViewAnswer) -> Vec<Batch> {
    match answer {
        ViewAnswer::List { rows, empty_text } => {
            let mut batches = sectioned(rows.into_iter().map(|row| row_item(extension, row, false)));
            match batches.first_mut() {
                Some(batch) => batch.empty_text = empty_text,
                None => batches.push(Batch {
                    empty_text,
                    ..Batch::default()
                }),
            }
            batches
        }
        ViewAnswer::Detail {
            title,
            detail,
            actions,
        } => {
            let row = Row {
                detail: Some(detail),
                actions,
                ..Row::new(DETAIL_ROW, title)
            };
            let shown = row_item(extension, row, false);
            let mut batch = Batch::from(vec![shown.item]);
            batch
                .details
                .extend(shown.detail.map(|detail| (DETAIL_ROW.to_owned(), detail)));
            vec![batch]
        }
        ViewAnswer::Form(form) => {
            let item = Item {
                id: FORM_ROW.into(),
                view: View::Text {
                    title: form.title.clone(),
                    subtitle: String::new(),
                },
                payload: Payload::default(),
                record_usage: false,
                icon: None,
                matched: Vec::new(),
                completion: None,
                score: None,
                primary: None,
                alternate: None,
                actions: Vec::new(),
            };
            vec![Batch {
                form: Some(Arc::new(form_of(extension, form))),
                ..Batch::from(vec![item])
            }]
        }
    }
}

/// The form as the launcher draws it and holds its drafts.
pub(super) fn form_of(extension: &Extension, form: view::Form) -> Form {
    Form {
        submit_title: form.submit_title.unwrap_or_else(|| "Submit".into()),
        title: form.title,
        extension: extension.id.clone(),
        submit: form.submit,
        fields: form
            .fields
            .into_iter()
            .map(|field| {
                let max_bytes = field.kind.max_text_bytes();
                let kind = match field.kind {
                    view::FieldKind::Text {
                        required,
                        default,
                        placeholder,
                    }
                    | view::FieldKind::Textarea {
                        required,
                        default,
                        placeholder,
                    } => FieldKind::Text {
                        multiline: max_bytes == Some(view::MAX_TEXTAREA_BYTES),
                        required,
                        default,
                        placeholder,
                        max_bytes: max_bytes.expect("a text field"),
                    },
                    view::FieldKind::Checkbox { default } => FieldKind::Checkbox { default },
                    view::FieldKind::Select { options, default } => FieldKind::Select {
                        default: options
                            .iter()
                            .position(|option| option.value == default)
                            .unwrap_or_default(),
                        options: options
                            .into_iter()
                            .map(|option| (option.value, option.title))
                            .collect(),
                    },
                };
                dupin_core::form::Field {
                    id: field.id,
                    title: field.title,
                    kind,
                }
            })
            .collect(),
    }
}

/// Consecutive rows of one section in one batch, each with its detail shown while selected.
fn sectioned(rows: impl Iterator<Item = Shown>) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::new();
    for shown in rows {
        let Shown {
            item,
            detail,
            section,
            accessories,
        } = shown;
        let batch = match batches.last_mut() {
            Some(batch) if batch.section == section => batch,
            _ => {
                batches.push(Batch {
                    section,
                    details_when_selected: true,
                    ..Batch::default()
                });
                batches.last_mut().expect("just pushed")
            }
        };
        if let Some(detail) = detail {
            batch.details.insert(item.id.clone(), detail);
        }
        if !accessories.is_empty() {
            batch.accessories.insert(item.id.clone(), accessories);
        }
        batch.items.push(item);
    }
    batches
}

impl Convert for Extension {
    /// A catalogue row as the launcher holds it.
    fn entry(&self, mut row: Row) -> Entry {
        let aliases = std::mem::take(&mut row.aliases);
        let keywords = std::mem::take(&mut row.keywords);
        let web_search = row.web_search.take();
        if web_search.is_some() {
            self.note_keyword_only_web_searches();
        }
        let match_text = row.match_text.take();
        let time = row.time.map(|ms| ms / 1000);
        let file = row.file.clone().map(Into::into);
        let file_actions = file_actions(&row);
        let Shown {
            item,
            detail,
            section,
            accessories,
        } = row_item(self, row, true);
        Entry {
            item,
            aliases,
            keywords,
            section: section.map(Into::into),
            accessories,
            detail,
            web_search,
            match_text,
            time,
            file,
            file_actions,
        }
    }

    /// Carries the extension's ID, since usage history keys rows by ID across every command. A
    /// built-in's port keeps the IDs the built-in gave its rows, so what was used carries over.
    fn entry_id(&self, id: &str) -> String {
        if crate::features::ported_command(&self.id).is_some() {
            return id.to_owned();
        }
        format!("extension:{}/{id}", self.id)
    }
}

/// The IDs [`row_item`] gives the row's actions that act on its file.
fn file_actions(row: &Row) -> Vec<String> {
    row.actions
        .iter()
        .enumerate()
        .filter(|(_, action)| action.needs_file || matches!(action.effect, RowEffect::Reveal { .. }))
        .map(|(index, _)| format!("action-{index}"))
        .collect()
}

/// The app's kinds, which a guest's row resolves through once the app has handed them over.
/// A row's `text` and `url` are text and URLs as any source's are, so they take the actions
/// config.toml adds to those kinds as well, which only the user runs. Its `file` is a path the
/// guest named, which Dupin hasn't checked is one the guest may act on, so it is no file-kind
/// row: it shows the file's icon and nothing more.
static KINDS: OnceLock<Kinds> = OnceLock::new();
/// Outside the app, as `dupin ext run` and fixtures run a package, nothing is configured.
static UNCONFIGURED: LazyLock<Kinds> = LazyLock::new(|| Kinds::native(Vec::new()));

/// Resolve every guest row's text and URLs through `kinds` from now on.
pub(crate) fn hand_over_kinds(kinds: Kinds) {
    let _ = KINDS.set(kinds);
}

type Refreshed = Box<dyn Fn(&PackageId) + Send + Sync>;
static REFRESHED: OnceLock<Refreshed> = OnceLock::new();

/// What the launcher does once a package's catalogue changed or outlived its `ttl` without a
/// keystroke: search again if the panel still lists it. Outside the launcher nobody does.
pub(crate) fn on_catalogue_refreshed(refreshed: impl Fn(&PackageId) + Send + Sync + 'static) {
    let _ = REFRESHED.set(Box::new(refreshed));
}

pub(super) fn catalogue_refreshed(package: &PackageId) {
    if let Some(refreshed) = REFRESHED.get() {
        refreshed(package);
    }
}

fn kinds() -> &'static Kinds {
    KINDS.get().unwrap_or(&UNCONFIGURED)
}

/// The action `subject`'s kind runs first, under the ID and title the row gives it.
fn kind_action(kinds: &Kinds, subject: &Subject, id: String, title: &str) -> ItemAction {
    let mut action = kinds
        .primary(subject, title)
        .expect("the text and URL kinds offer an action for their own subjects");
    action.id = id;
    title.clone_into(&mut action.title);
    action
}

/// A row as the launcher shows it, with what it shows beside the item.
pub(super) struct Shown {
    pub item: Item,
    pub detail: Option<Arc<Detail>>,
    pub section: Option<String>,
    pub accessories: Vec<Accessory>,
}

pub(super) fn row_item(extension: &Extension, row: Row, catalogue: bool) -> Shown {
    row_item_with(kinds(), extension, row, catalogue)
}

fn row_item_with(kinds: &Kinds, extension: &Extension, row: Row, catalogue: bool) -> Shown {
    let Row {
        id,
        title,
        subtitle,
        aliases: _,
        keywords: _,
        match_text: _,
        time: _,
        file,
        icon,
        image,
        section,
        detail,
        actions: offered,
        web_search: _,
        accessories,
        answer,
    } = row;
    let accessories = accessories
        .into_iter()
        .map(|accessory| match accessory {
            row::Accessory::Text(text) => Accessory::Text(text),
            row::Accessory::Tag(text) => Accessory::Tag(text),
            row::Accessory::Freshness(time) => Accessory::Freshness(time),
            row::Accessory::State(row::RowState::Offline) => Accessory::State(RowState::Offline),
            row::Accessory::State(row::RowState::Stale) => Accessory::State(RowState::Stale),
        })
        .collect();
    let detail = detail.map(|detail| {
        let mut shown = Detail::new(title.clone(), &detail.markdown);
        if let Some(image) = detail.image {
            shown = shown.with_image(host_image(extension, image));
        }
        for entry in detail.metadata {
            shown = shown.with_metadata(entry.label, entry.text);
        }
        Arc::new(shown)
    });
    // Dupin reads the clipboard unfiltered to fill a template, as the built-in it ports did, so
    // only the package Dupin shipped for a built-in gets templates; any other copies as written.
    let templates = crate::features::fills_templates(&extension.package);
    // Text Dupin fills in only when it is chosen isn't the row's text until then.
    let payload = offered
        .iter()
        .find_map(|action| match &action.effect {
            RowEffect::Copy { text }
                if !(templates && action.template) || !snippets::has_placeholders(text) =>
            {
                Some(Payload::text(text.clone()))
            }
            RowEffect::Copy { .. } => Some(Payload::default()),
            _ => None,
        })
        .unwrap_or_default();
    let (mut copy, mut paste, mut open) = (false, false, false);
    let mut alternate = None;
    // The first text a row copies as written and the first URL it opens are what the kinds'
    // configured actions take, once each.
    let mut configured = Vec::new();
    let mut configure = |subject: Subject, done: &mut bool| {
        if !std::mem::replace(done, true) {
            configured.extend(kinds.configured(&subject, &title, Producer::Extension));
        }
    };
    let (mut texts, mut urls) = (false, false);
    let mut actions: Vec<ItemAction> = offered
        .into_iter()
        .enumerate()
        .map(|(index, action)| {
            let name = |taken: &mut bool, first: &str| {
                if std::mem::replace(taken, true) {
                    format!("action-{index}")
                } else {
                    first.to_owned()
                }
            };
            let template = templates && action.template;
            match action.effect {
                RowEffect::Copy { text } if template => ItemAction::new(
                    &name(&mut copy, "copy"),
                    &action.title,
                    Action::Snippet { text, paste: false },
                ),
                RowEffect::Copy { text } => {
                    configure(Subject::Text(text.clone()), &mut texts);
                    kind_action(
                        kinds,
                        &Subject::Text(text),
                        name(&mut copy, "copy"),
                        &action.title,
                    )
                }
                RowEffect::Paste { text } => ItemAction::new(
                    &name(&mut paste, "paste"),
                    &action.title,
                    Action::Extension {
                        extension: extension.id.clone(),
                        request: ExtensionRequest::Paste { text, template },
                    },
                ),
                RowEffect::OpenUrl { url } => {
                    configure(Subject::Url(url.clone()), &mut urls);
                    kind_action(kinds, &Subject::Url(url), name(&mut open, "open"), &action.title)
                }
                RowEffect::Reveal { path } => ItemAction::new(
                    &format!("action-{index}"),
                    &action.title,
                    Action::Reveal(path.into()),
                ),
                RowEffect::Run { handler, data } => {
                    let id = format!("action-{index}");
                    if action.alternate {
                        alternate = Some(id.clone());
                    }
                    let run = ItemAction::new(
                        &id,
                        &action.title,
                        Action::Extension {
                            extension: extension.id.clone(),
                            request: ExtensionRequest::Run {
                                action: handler,
                                data,
                            },
                        },
                    )
                    .keep_open();
                    match &action.confirm {
                        Some(prompt) => run.confirm(prompt, &action.title),
                        None => run,
                    }
                }
                RowEffect::Push { view, state } => ItemAction::new(
                    &format!("action-{index}"),
                    &action.title,
                    Action::Push(Page {
                        view,
                        title: action.title.clone(),
                        context: state,
                    }),
                ),
            }
        })
        .collect();
    actions.extend(configured);
    let view = match answer {
        Some(result) => View::Pair {
            source: title,
            result,
        },
        None => View::Text { title, subtitle },
    };
    let item = Item {
        id: if catalogue { extension.entry_id(&id) } else { id },
        view,
        payload,
        // A catalogue matched by words keeps its own order, as a history does, so picks never
        // rank it.
        record_usage: catalogue && extension.matching == dupin_core::catalogue::Matching::Fuzzy,
        icon: image
            .map(|image| Icon::Host(host_image(extension, image)))
            .or_else(|| icon.map(|icon| row_icon(extension, icon)))
            .or_else(|| file.map(|file| Icon::File(file.into()))),
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: actions.first().map(|action| action.id.clone()),
        // Without an `alternate` of its own, ⌘Enter copies what Enter would paste, as it does
        // for snippets.
        alternate: alternate.or_else(|| {
            actions
                .first()
                .filter(|action| action.id == "paste")
                .and(copy.then(|| "copy".to_owned()))
        }),
        actions,
    };
    Shown {
        item,
        detail,
        section,
        accessories,
    }
}

/// The icon a row names, which the row's check found to be a symbol's name or a path inside
/// the package.
fn row_icon(extension: &Extension, icon: String) -> Icon {
    match icon.strip_prefix(dupin_core::item::SYMBOL_PREFIX) {
        Some(name) => Icon::Symbol(name.to_owned()),
        None => Icon::Image(extension.root.join(icon)),
    }
}

/// The image a row of the extension names, which Dupin resolves in that package's storage.
fn host_image(extension: &Extension, image: RowImage) -> HostImage {
    HostImage {
        extension: extension.id.clone(),
        source: match image {
            RowImage::Blob(handle) => HostImageSource::Blob(handle),
            RowImage::File(file) => HostImageSource::File {
                path: file.path.into(),
                handle: file.handle,
            },
        },
    }
}

impl ExtensionProvider {
    /// Among the main search's rows from every source, each of the package's rows without an
    /// icon shows the package's. Under its keyword its chip shows it once, so its rows keep
    /// only their own.
    fn with_package_icon(&self, role: Role, mut batch: Batch) -> Batch {
        if let (Role::DefaultSearch | Role::MainSearch, Some(icon)) = (role, &self.extension.icon) {
            for item in &mut batch.items {
                item.icon.get_or_insert_with(|| icon.clone());
            }
        }
        batch
    }
}

impl Provider for ExtensionProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        if request.limit == 0 {
            return Ok(());
        }
        // A page belongs to its package alone, wherever its row was chosen, so it answers as the
        // keyword does, questions and failures included.
        let role = if request.page.is_some() {
            Role::Keyword
        } else {
            self.role
        };
        let deadline = crate::extensions::deadline_in(QUERY_DEADLINE);
        let id = &self.extension.package;
        // A main search the package may not see runs nothing, and asks nothing while typing.
        let seen = if role == Role::MainSearch {
            let Some(bridge) = &self.bridge else {
                return Ok(());
            };
            match bridge.authorize(id, &dupin_extension_bridge::Request::Query, Trigger::Query) {
                Ok(permit) => Some(permit),
                Err(_) => return Ok(()),
            }
        } else {
            None
        };
        // Loading a catalogue is a query too, from either search, with no activation.
        let call = match &self.bridge {
            Some(bridge) => bridge.call(id, Trigger::Query),
            None => Call::none(),
        };
        let answer = match role {
            Role::Keyword => {
                let cancellation = request.cancellation();
                answer(
                    &self.extension,
                    &request.query,
                    request.page.as_ref(),
                    request.limit,
                    deadline,
                    move || cancellation.is_cancelled(),
                    &call,
                )
            }
            Role::DefaultSearch => {
                default_answer(&self.extension, &request.query, request.limit, deadline, &call)
            }
            Role::MainSearch => {
                let cancellation = request.cancellation();
                main_answer(
                    &self.extension,
                    &request.query,
                    request.limit,
                    deadline,
                    move || cancellation.is_cancelled(),
                    &call,
                )
            }
        };
        if let (Some(bridge), Some(permit)) = (&self.bridge, &seen)
            && !bridge.validate(permit)
        {
            return Ok(());
        }
        if let Some(bridge) = &self.bridge
            && !bridge.is_current(id, call.generation)
        {
            // The main search shows everyone's results; a package whose access changed only
            // drops its own.
            if role == Role::Keyword {
                emit(Batch::from(vec![self.problem(
                    "Its access changed while it ran, so its answer was dropped. Type again to retry.".into(),
                )]));
            }
            return Ok(());
        }
        if request.is_cancelled() || matches!(answer, Err(RunError::Cancelled)) {
            return Ok(());
        }
        // Only the keyword asks for consent; the main search never shows another package's
        // questions among its results.
        let access = match role {
            Role::Keyword => self.bridge.as_ref().map(|bridge| (bridge, id)),
            Role::DefaultSearch | Role::MainSearch => None,
        };
        if let Some((bridge, id)) = access {
            let mut asking: Vec<_> = bridge
                .pending(Some(id))
                .iter()
                .map(|request| consent_row(&self.extension, request))
                .collect();
            if bridge.asked_while_off(id) {
                asking.insert(0, turn_on_row(&self.extension));
            }
            if !asking.is_empty() {
                emit(Batch::from(asking));
            }
        }
        match answer {
            Ok(batches) => batches
                .into_iter()
                .map(|batch| self.with_package_icon(role, batch))
                .for_each(emit),
            Err(RunError::Cancelled) => {}
            Err(RunError::Failed(diagnostic)) => {
                crate::log::warn(&format!(
                    "extension {}: {}",
                    self.extension.id,
                    diagnostic.for_log()
                ));
                // The main search shows everyone's results; a failing extension only logs.
                if role == Role::Keyword {
                    // A located problem starts with its file name, which keeps its case.
                    let subtitle = if diagnostic.file.is_some() {
                        diagnostic.to_string()
                    } else {
                        capitalized(&diagnostic.message)
                    };
                    emit(Batch::from(vec![self.problem(subtitle)]));
                }
            }
        }
        Ok(())
    }
}

fn extension_action(extension: &Extension, id: &str, title: &str, request: ExtensionRequest) -> ItemAction {
    ItemAction::new(
        id,
        title,
        Action::Extension {
            extension: extension.id.clone(),
            request,
        },
    )
    .consumes()
}

/// A row that asks the user to decide. It sits above the extension's results, where Enter
/// lands while typing, so Enter opens its choices instead of granting anything.
fn notice_row(id: String, title: String, subtitle: String, actions: Vec<ItemAction>) -> Item {
    Item {
        id,
        payload: Payload::default(),
        view: View::Text { title, subtitle },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: None,
        alternate: None,
        actions,
    }
}

/// What the panel shows when the extension asked for a scope the user hasn't decided on: the
/// scope as the broker describes it, and Allow and Deny. Nothing runs until the user chooses,
/// and choosing runs nothing the extension asked for before.
pub(crate) fn consent_row(extension: &Extension, request: &ConsentRequest) -> Item {
    let key = request.key().to_string();
    let title = format!("Allow {} to {}?", extension.name, request.description);
    let (subtitle, actions) = match request.reason {
        ConsentReason::OsAccess(permission) => (
            format!(
                "macOS hasn't allowed Dupin {} yet. Requesting it asks macOS once.",
                permission.name()
            ),
            vec![extension_action(
                extension,
                "request-access",
                "Request Access",
                ExtensionRequest::RequestAccess { scope: key.clone() },
            )],
        ),
        _ if request.needs_pick() => (
            format!("{key} · Choose what it may use in Settings → Extensions."),
            vec![extension_action(
                extension,
                "review",
                "Open Settings",
                ExtensionRequest::Review,
            )],
        ),
        reason => (
            if reason == ConsentReason::Changed {
                format!(
                    "{key} · An update changed what it asks for. Allowing runs nothing until you use it again."
                )
            } else {
                format!("{key} · Allowing runs nothing until you use it again.")
            },
            vec![
                extension_action(
                    extension,
                    "allow",
                    "Allow",
                    ExtensionRequest::Allow {
                        scope: key.clone(),
                        revision: request.revision.to_string(),
                    },
                ),
                extension_action(
                    extension,
                    "deny",
                    "Deny",
                    ExtensionRequest::Deny { scope: key.clone() },
                ),
            ],
        ),
    };
    let mut actions = actions;
    if !actions.iter().any(|action| action.id == "deny") {
        actions.push(extension_action(
            extension,
            "deny",
            "Deny",
            ExtensionRequest::Deny { scope: key.clone() },
        ));
    }
    notice_row(format!("{CONSENT_ROW}{key}"), title, subtitle, actions)
}

const CONSENT_ROW: &str = "consent:";
const TURN_ON_ROW: &str = "consent:turn-on";
const PROBLEM_ROW: &str = "extension-error";

/// Whether the row `id` is one Dupin shows in place of an extension's answer: the extension
/// failed, or waits to be turned on or for consent.
pub(crate) fn stands_in(id: &str) -> bool {
    id == PROBLEM_ROW || id.starts_with(CONSENT_ROW)
}

/// Whether `row` asks the user something about `id` that the bridge no longer waits on, so the
/// panel can drop it after an answer without running the guest again.
pub(crate) fn is_answered_consent_row(row: &Item, bridge: &Bridge, id: &PackageId) -> bool {
    let Some(asked) = row.id.strip_prefix(CONSENT_ROW) else {
        return false;
    };
    if row.id == TURN_ON_ROW {
        return !bridge.asked_while_off(id);
    }
    !bridge
        .pending(Some(id))
        .iter()
        .any(|request| request.key().to_string() == asked)
}

fn turn_on_row(extension: &Extension) -> Item {
    notice_row(
        TURN_ON_ROW.into(),
        format!("Turn on {}?", extension.name),
        "It asked to use something while it's off. Turning it on lets it ask for each thing it declares."
            .into(),
        vec![
            extension_action(extension, "turn-on", "Turn On", ExtensionRequest::TurnOn),
            extension_action(extension, "review", "Open Settings", ExtensionRequest::Review),
        ],
    )
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(provider: &ExtensionProvider, query: &str) -> Vec<Item> {
        let mut rows = Vec::new();
        provider
            .search(&Request::detached(query, 10), &mut |batch| rows.extend(batch))
            .unwrap();
        rows
    }

    fn page(provider: &ExtensionProvider, view: &str, query: &str, state: serde_json::Value) -> Vec<Batch> {
        let page = Page {
            view: view.into(),
            title: "Page".into(),
            context: Some(state),
        };
        let mut batches = Vec::new();
        provider
            .search_batches(&Request::detached_page(query, page, 10), &mut |batch| {
                batches.push(batch)
            })
            .unwrap();
        batches
    }

    const VIEWS: &str = r#"
        export const search = () => [{ title: 'Note', accessories: [{ tag: 'New' }, { freshness: 1700000000000 }, { state: 'offline' }], answer: '4',
            actions: [{ title: 'Read', push: { view: 'note', state: { id: 7 } } }] }];
        export const views = {
            note: (ctx) => ({ type: 'detail', title: `Note ${ctx.state.id}`, markdown: ctx.query,
                actions: [{ title: 'Edit', push: { view: 'edit', state: ctx.state } }, { title: 'Delete', run: 'remove', confirm: 'Delete it?' }] }),
            empty: (ctx) => ({ type: 'list', sections: [], emptyText: `No ${ctx.query}` }),
            edit: () => ({ type: 'form', title: 'Edit', submit: 'save', fields: [
                { type: 'text', id: 'title', title: 'Title', required: true },
                { type: 'select', id: 'label', title: 'Label', default: 'blue',
                  options: [{ value: 'red', title: 'Red' }, { value: 'blue', title: 'Blue' }] }] }),
            broken: () => ({ type: 'grid' }),
        };
    "#;

    #[test]
    fn rows_open_their_package_views_and_show_accessories_and_answers() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider) = javascript(VIEWS);
        let mut batches = Vec::new();
        provider
            .search_batches(&Request::detached("x", 10), &mut |batch| batches.push(batch))
            .unwrap();
        let row = &batches[0].items[0];
        assert_eq!(
            row.view,
            View::Pair {
                source: "Note".into(),
                result: "4".into()
            }
        );
        assert_eq!(
            batches[0].accessories[&row.id],
            [
                Accessory::Tag("New".into()),
                Accessory::Freshness(1_700_000_000_000),
                Accessory::State(RowState::Offline)
            ]
        );
        assert_eq!(
            row.action("action-0"),
            Some(&Action::Push(Page {
                view: "note".into(),
                title: "Read".into(),
                context: Some(serde_json::json!({"id": 7})),
            }))
        );
    }

    #[test]
    fn a_page_shows_its_view_with_the_state_it_holds() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider) = javascript(VIEWS);
        let detail = page(&provider, "note", "typed", serde_json::json!({"id": 7}));
        let row = &detail[0].items[0];
        assert_eq!(row.id, DETAIL_ROW);
        assert!(!detail[0].details_when_selected, "a detail page stays open");
        assert_eq!(detail[0].details[DETAIL_ROW].title, "Note 7");
        assert_eq!(row.primary.as_deref(), Some("action-0"));
        assert!(row.item_action("action-1").unwrap().confirmation().is_some());

        let empty = page(&provider, "empty", "tea", serde_json::Value::Null);
        assert!(empty[0].items.is_empty());
        assert_eq!(empty[0].empty_text.as_deref(), Some("No tea"));

        let form = page(&provider, "edit", "", serde_json::Value::Null);
        let form = form[0].form.as_ref().expect("a form");
        assert_eq!(
            (
                form.extension.as_str(),
                form.submit.as_str(),
                form.submit_title.as_str()
            ),
            ("word-count", "save", "Submit")
        );
        assert!(form.fields[0].required());
        assert!(matches!(
            form.fields[1].kind,
            FieldKind::Select { default: 1, .. }
        ));

        let broken = page(&provider, "broken", "", serde_json::Value::Null);
        assert_eq!(broken[0].items[0].id, PROBLEM_ROW);
        let missing = page(&provider, "absent", "", serde_json::Value::Null);
        let View::Text { subtitle, .. } = &missing[0].items[0].view else {
            panic!()
        };
        assert!(subtitle.contains("views.absent"), "{subtitle}");
    }

    #[test]
    fn a_page_opened_from_the_main_search_answers_as_the_keyword_does() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, keyword) = javascript(VIEWS);
        let main = ExtensionProvider::with_role(keyword.extension.clone(), Role::DefaultSearch);
        let shown = page(&main, "broken", "", serde_json::Value::Null);
        assert_eq!(
            shown[0].items[0].id, PROBLEM_ROW,
            "its failure shows on its own page"
        );
    }

    fn javascript(source: &str) -> (tempfile::TempDir, ExtensionProvider) {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join(super::super::manifest::PACKAGE_JSON),
            r#"{"name": "word-count", "version": "0.1.0", "dupin": {"title": "Word Count", "keyword": "wc"}}"#,
        )
        .unwrap();
        std::fs::write(folder.path().join("main.js"), source).unwrap();
        let extension = super::super::load_folder(folder.path()).unwrap();
        (folder, ExtensionProvider::new(extension))
    }

    #[test]
    fn the_word_counter_counts_words_and_copies_the_count() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/word-count-js");
        let extension = super::super::load_folder(&root).unwrap();
        assert_eq!(extension.keyword.as_deref(), Some("wc"));
        let rows = search(&ExtensionProvider::new(extension), "two words");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].view,
            View::Text {
                title: "2 words".into(),
                subtitle: String::new()
            }
        );
        assert_eq!(rows[0].primary.as_deref(), Some("copy"));
        assert_eq!(rows[0].action("copy"), Some(&Action::Copy("2".into())));
        assert_eq!(rows[0].payload, Payload::text("2"));
        assert!(!rows[0].record_usage);
    }

    #[test]
    fn configured_kind_actions_take_a_guest_rows_text_and_url_but_never_its_file() {
        use dupin_js_runtime::row::RowAction;

        let kinds = dupin_mac::kinds::native();
        kinds.configure(
            &dupin_core::Config::parse(
                r#"
[[advanced.kinds.text.actions]]
title = "Translate"
url = "https://translate.example/?q={text}"

[[advanced.kinds.url.actions]]
title = "Archive"
script = "archive"

[[advanced.kinds.file.actions]]
title = "Open in Zed"
script = "open-in-zed"

[[advanced.kinds.app.actions]]
title = "App"
script = "app"
"#,
            )
            .unwrap()
            .advanced
            .kinds,
        );
        let extension = super::super::tests::extension("Guest", "g");
        let mut row: Row = serde_json::from_value(serde_json::json!({
            "id": "configured-file-0",
            "title": "configured-text-0",
            "file": "/Applications/Safari.app"
        }))
        .unwrap();
        row.actions = vec![
            RowAction::new("Copy", RowEffect::Copy { text: "one".into() }),
            RowAction::new("Copy again", RowEffect::Copy { text: "two".into() }),
            RowAction::new(
                "Open",
                RowEffect::OpenUrl {
                    url: "https://example.com/a b".into(),
                },
            ),
            RowAction::new(
                "Reveal",
                RowEffect::Reveal {
                    path: "/Applications/Safari.app".into(),
                },
            ),
        ];
        let item = row_item_with(&kinds, &extension, row, false).item;
        let ids: Vec<_> = item.actions.iter().map(|action| action.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "copy",
                "action-1",
                "open",
                "action-3",
                "configured-text-0",
                "configured-url-0"
            ]
        );
        assert_eq!(item.primary.as_deref(), Some("copy"));
        assert_eq!(
            item.action("configured-text-0"),
            Some(&Action::OpenUrl("https://translate.example/?q=one".into()))
        );
        assert_eq!(
            item.action("configured-url-0"),
            Some(&Action::ConfiguredScript {
                script: "archive".into(),
                argument: "https://example.com/a b".into()
            })
        );
        for id in ["configured-file-0", "configured-app-0", "configured-text-1"] {
            assert_eq!(
                item.action(id),
                None,
                "a guest's file or app path, or second text, reaches no configured action"
            );
        }

        let unconfigured = row_item_with(
            &dupin_mac::kinds::native(),
            &extension,
            serde_json::from_value(serde_json::json!({"id": "plain", "title": "Plain"})).unwrap(),
            false,
        );
        assert!(unconfigured.item.actions.is_empty());
    }

    #[test]
    fn javascript_urls_open_first_and_text_copies() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider) = javascript(
            "export function search(query) { return [{ id: 'docs', title: 'Docs', subtitle: query, url: 'https://example.com/?q=2', text: '2' }]; }",
        );
        let rows = search(&provider, "two words");
        assert_eq!(rows[0].id, "docs");
        assert_eq!(rows[0].primary.as_deref(), Some("open"));
        assert_eq!(
            rows[0].action("open"),
            Some(&Action::OpenUrl("https://example.com/?q=2".into()))
        );
        assert_eq!(rows[0].action("copy"), Some(&Action::Copy("2".into())));
        assert_eq!(rows[0].payload, Payload::text("2"));
    }

    #[test]
    fn a_paste_action_asks_for_its_extension_and_a_copy_is_its_payload() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider) = javascript(
            "export function search() { return [{ title: 'Tea', actions: [{ title: 'Paste', paste: '🍵' }, { title: 'Copy', copy: '🍵' }] }]; }",
        );
        let rows = search(&provider, "tea");
        assert_eq!(rows[0].primary.as_deref(), Some("paste"));
        assert_eq!(
            rows[0].action("paste"),
            Some(&Action::Extension {
                extension: provider.extension.id.clone(),
                request: ExtensionRequest::Paste {
                    text: "🍵".into(),
                    template: false,
                },
            })
        );
        assert_eq!(rows[0].payload, Payload::text("🍵"));
    }

    #[test]
    fn only_the_package_dupin_shipped_for_a_built_in_fills_templates() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join(super::super::manifest::PACKAGE_JSON),
            r#"{"name": "snippets", "version": "0.1.0", "dupin": {"title": "Snippets", "keyword": "snip"}}"#,
        )
        .unwrap();
        std::fs::write(
            folder.path().join("main.js"),
            "export function search() { return [{ title: 'Quote', actions: [{ title: 'Paste', paste: '> {clipboard}', template: true }, { title: 'Copy', copy: '> {clipboard}', template: true }] }]; }",
        )
        .unwrap();
        let paste = |template| Action::Extension {
            extension: "snippets".into(),
            request: ExtensionRequest::Paste {
                text: "> {clipboard}".into(),
                template,
            },
        };

        let copy = super::super::load_folder(folder.path()).unwrap();
        assert_eq!(crate::features::paste_notice(&copy.package), None);
        let rows = search(&ExtensionProvider::new(copy), "quote");
        assert_eq!(rows[0].action("paste"), Some(&paste(false)));
        assert_eq!(
            rows[0].action("copy"),
            Some(&Action::Copy("> {clipboard}".into()))
        );
        assert_eq!(rows[0].payload, Payload::text("> {clipboard}"));

        let shipped = super::super::load_bundled(folder.path()).unwrap();
        assert!(crate::features::paste_notice(&shipped.package).is_some());
        let rows = search(&ExtensionProvider::new(shipped), "quote");
        assert_eq!(rows[0].action("paste"), Some(&paste(true)));
        assert_eq!(
            rows[0].action("copy"),
            Some(&Action::Snippet {
                text: "> {clipboard}".into(),
                paste: false,
            })
        );
        assert_eq!(rows[0].payload, Payload::default());
    }

    #[test]
    fn javascript_failures_become_a_visible_error_row_with_the_location() {
        dupin_extension_host::skip_inside_a_sandbox!();
        for (source, subtitle) in [
            (
                "export function search() {\n  throw new Error('boom');\n}",
                "main.js:2:13: search threw Error: boom",
            ),
            (
                "export function search() { return [{ title: 'x', url: 'file:///' }]; }",
                "main.js: `items[0].url` must be an http or https URL with a host, a dict:// URL, or a calendar event's `link`",
            ),
        ] {
            let (_folder, provider) = javascript(source);
            let rows = search(&provider, "query");
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0].view,
                View::Text {
                    title: "Word Count failed".into(),
                    subtitle: subtitle.into(),
                }
            );
            assert!(rows[0].actions.is_empty());
        }
    }

    /// A JavaScript extension that declares `capabilities`, behind an in-memory bridge.
    fn declaring(capabilities: &str, source: &str) -> (tempfile::TempDir, ExtensionProvider, Bridge) {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join(super::super::manifest::PACKAGE_JSON),
            format!(
                r#"{{"name": "word-count", "version": "0.1.0", "dupin": {{"title": "Word Count", "keyword": "wc", "capabilities": {capabilities}}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(folder.path().join("main.js"), source).unwrap();
        let extension = super::super::load_folder(folder.path()).unwrap();
        let bridge = Bridge::new(
            dupin_extension_bridge::Broker::new(Default::default(), Default::default()),
            None,
            dupin_extension_bridge::Services::none(),
        );
        let (id, revision) = (extension.package.clone(), extension.revision.clone());
        bridge.install(&id, revision, extension.capabilities.clone());
        let (package_on_disk, revision_on_disk, _) = super::super::declaration_of(folder.path()).unwrap();
        assert_eq!(package_on_disk, extension.package);
        assert_eq!(revision_on_disk, extension.revision);
        (
            folder,
            ExtensionProvider::with_bridge(extension, Role::Keyword, bridge.clone()),
            bridge,
        )
    }

    #[test]
    fn a_request_while_off_offers_to_turn_the_extension_on() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider, bridge) = declaring(
            r#"{"storage": {"quotaBytes": 1024}}"#,
            "import { storage } from 'dupin'; export async function search() { try { await storage.usage(); } catch (error) { return [{ title: error.code }]; } }",
        );
        let rows = search(&provider, "x");
        assert_eq!(rows[0].id, "consent:turn-on");
        assert_eq!(rows[0].display_title(), "Turn on Word Count?");
        assert_eq!(
            rows[0].action("turn-on"),
            Some(&Action::Extension {
                extension: "word-count".into(),
                request: ExtensionRequest::TurnOn
            })
        );
        assert_eq!(rows[1].display_title(), "disabled");

        let id = provider.extension.package.clone();
        assert!(!is_answered_consent_row(&rows[0], &bridge, &id));
        bridge.set_enabled(&id, true).unwrap();
        assert!(is_answered_consent_row(&rows[0], &bridge, &id));
        assert!(
            !is_answered_consent_row(&rows[1], &bridge, &id),
            "only questions go"
        );
        let rows = search(&provider, "x");
        assert_eq!(rows[0].id, "consent:storage");
        assert_eq!(rows[0].primary, None, "Enter shows the choices, never grants");
        assert_eq!(
            rows[0].display_title(),
            "Allow Word Count to keep up to 1024 bytes of encrypted data?"
        );
        let Some(Action::Extension {
            request: ExtensionRequest::Allow { scope, revision },
            ..
        }) = rows[0].action("allow")
        else {
            panic!("{:?}", rows[0].actions);
        };
        assert_eq!(scope, "storage");
        assert_eq!(revision, provider.extension.revision.as_str());
        assert!(rows[0].action("deny").is_some());
        assert_eq!(rows[1].display_title(), "needs-consent");

        let request = bridge.pending(Some(&id)).remove(0);
        assert!(!is_answered_consent_row(&rows[0], &bridge, &id));
        bridge
            .allow(&id, &request.revision, &request.scope, None)
            .unwrap();
        assert!(
            is_answered_consent_row(&rows[0], &bridge, &id),
            "the panel drops the answered row without searching again"
        );
        let rows = search(&provider, "x");
        assert_eq!(rows.len(), 1, "the request is answered");
        // The in-memory bridge reaches no service, so the granted request now fails there.
        assert_eq!(rows[0].display_title(), "unavailable");
    }

    #[test]
    fn rows_offering_actions_run_them_in_their_own_extension() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider, _bridge) = declaring(
            "{}",
            "export const search = () => [{ title: 'Note', actions: [{ title: 'Save', run: 'save', data: { id: 7 } }] }];",
        );
        let rows = search(&provider, "x");
        assert_eq!(rows[0].primary.as_deref(), Some("action-0"));
        assert_eq!(
            rows[0].action("action-0"),
            Some(&Action::Extension {
                extension: "word-count".into(),
                request: ExtensionRequest::Run {
                    action: "save".into(),
                    data: Some(serde_json::json!({"id": 7}))
                }
            })
        );
    }
}
