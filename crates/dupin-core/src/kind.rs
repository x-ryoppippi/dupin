//! Kinds: what a row stands for, and the actions Dupin offers for it. A source says what each
//! of its rows is; the kind registered for that, not the source, decides which actions the row
//! has, so every source of files offers the same file actions and a configured kind changes
//! them everywhere at once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use crate::config::{KindAction, KindActions, KindConfig};
use crate::item::Transfer;
use crate::{Action, Condition, Item, ItemAction, Payload, ShareTarget};

/// The ID of the action that shows a file in Quick Look, which ⌘Y runs.
pub const QUICK_LOOK: &str = "quick-look";

/// The IDs of the file kind's own actions, in the order its rows list them: `browse` only on
/// a folder, and the actions from `rename` to `compress` only on a name that is UTF-8.
/// `[advanced.kinds.file]` `hide` and `order` name them.
pub(crate) const FILE_ACTIONS: [&str; 12] = [
    "open",
    "browse",
    "open-with",
    "reveal",
    QUICK_LOOK,
    "copy-name",
    "rename",
    "move-to",
    "copy-to",
    "duplicate",
    "compress",
    "trash",
];

/// The IDs of the app kind's own actions, in the order its rows list them.
pub(crate) const APP_ACTIONS: [&str; 3] = ["open", "reveal", QUICK_LOOK];

/// The kinds Dupin registers: `text`, `url`, `file` and `app`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindName {
    Text,
    Url,
    File,
    App,
}

impl KindName {
    const ALL: [Self; 4] = [Self::Text, Self::Url, Self::File, Self::App];

    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Url => "url",
            Self::File => "file",
            Self::App => "app",
        }
    }
}

/// What a row stands for, typed by its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    Text(String),
    /// A URL its source has already validated as one it may open.
    Url(String),
    /// A file or folder a native source found; `folder` is a directory Finder opens as a
    /// folder, not a package such as an app bundle.
    File {
        path: PathBuf,
        folder: bool,
    },
    /// An application bundle.
    App(PathBuf),
}

/// Who produced a row: one of Dupin's own sources, or an extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Producer {
    Dupin,
    /// An extension's rows never reach another extension's kind actions.
    Extension,
}

/// What must hold of a row before a package's kind action is offered on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// The app the row stands for is running.
    Running,
}

/// A kind action a package that runs now provides, as its manifest declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provided {
    pub kind: KindName,
    pub package: String,
    pub name: String,
    /// What the action screen calls it when Dupin composes it by default.
    pub title: Option<String>,
    /// Ask before running it, wherever it is composed.
    pub confirm: bool,
    pub availability: Option<Availability>,
    /// Composed onto every row of its kind, unless config.toml adds it itself.
    pub default: bool,
}

impl Provided {
    fn is(&self, kind: KindName, package: &str, name: &str) -> bool {
        self.kind == kind && self.package == package && self.name == name
    }

    /// `action` as this declaration asks for it on the row `subject`, titled `title`.
    fn shape(&self, action: ItemAction, subject: &Subject, title: &str) -> ItemAction {
        let mut action = action;
        if self.confirm {
            let verb = action.title.trim_end_matches('…').to_owned();
            action = action.confirm(&format!("{verb} “{title}”?"), &verb);
        }
        match (self.availability, subject) {
            (Some(Availability::Running), Subject::App(path)) => {
                action.when(Condition::Running(path.clone()))
            }
            _ => action,
        }
    }
}

impl Subject {
    pub(crate) fn value_bytes(&self) -> usize {
        match self {
            Self::Text(value) | Self::Url(value) => value.len(),
            Self::File { path, .. } | Self::App(path) => path.as_os_str().len(),
        }
    }

    fn kind(&self) -> KindName {
        match self {
            Self::Text(_) => KindName::Text,
            Self::Url(_) => KindName::Url,
            Self::File { .. } => KindName::File,
            Self::App(_) => KindName::App,
        }
    }
}

/// A row's actions as its kind resolved them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    /// In the order the action menu lists them.
    pub(crate) actions: Vec<ItemAction>,
    /// The action ID Enter runs.
    pub(crate) primary: Option<String>,
    /// The action ID ⌘Enter runs.
    pub(crate) alternate: Option<String>,
}

/// The actions Dupin offers for one kind of row.
pub trait Kind: Send + Sync + 'static {
    /// The actions for `subject`, a row of this kind titled `title`; nothing for a subject of
    /// another kind.
    fn actions(&self, subject: &Subject, title: &str) -> Resolved;

    /// What Send to… offers for the part of `payload` this kind carries.
    fn send_to(&self, payload: &Payload) -> Vec<ItemAction>;
}

/// The kind registered for each kind of row.
#[derive(Clone)]
pub struct Kinds {
    text: Arc<dyn Kind>,
    url: Arc<dyn Kind>,
    file: Arc<dyn Kind>,
    app: Arc<dyn Kind>,
    /// What `[advanced.kinds]` adds, shared by every clone so a reload reaches each source.
    configured: Arc<RwLock<KindActions>>,
    /// The kind actions the packages that run now provide, shared as `configured` is.
    provided: Arc<RwLock<Vec<Provided>>>,
}

impl Kinds {
    /// Dupin's own kinds, with `share` as the services Send to… offers for files.
    pub fn native(share: Vec<ShareChoice>) -> Self {
        Self {
            text: Arc::new(TextKind),
            url: Arc::new(UrlKind),
            file: Arc::new(FileKind { share }),
            app: Arc::new(AppKind),
            configured: Arc::default(),
            provided: Arc::default(),
        }
    }

    /// Give each kind the actions `actions` lists for it after its own, arranged by its `hide`
    /// and `order`, for rows resolved from now on. Whether they differ from the ones it had.
    pub fn configure(&self, actions: &KindActions) -> bool {
        let mut configured = self.configured.write().unwrap_or_else(PoisonError::into_inner);
        let changed = *configured != *actions;
        configured.clone_from(actions);
        changed
    }

    /// Hand over the kind actions the packages that run now provide, for rows resolved from now
    /// on. Whether they differ from the ones before.
    pub fn provide(&self, provided: Vec<Provided>) -> bool {
        let mut current = self.provided.write().unwrap_or_else(PoisonError::into_inner);
        let changed = *current != provided;
        *current = provided;
        changed
    }

    fn get(&self, name: KindName) -> &Arc<dyn Kind> {
        match name {
            KindName::Text => &self.text,
            KindName::Url => &self.url,
            KindName::File => &self.file,
            KindName::App => &self.app,
        }
    }

    /// The actions of a row Dupin produced for `subject`, titled `title`: its kind's own, then
    /// the ones config.toml adds, then the ones packages compose by default, arranged by the
    /// kind's `hide` and `order`.
    fn resolve(&self, subject: &Subject, title: &str) -> Resolved {
        let mut resolved = self.get(subject.kind()).actions(subject, title);
        let configured = self.configured.read().unwrap_or_else(PoisonError::into_inner);
        let provided = self.provided.read().unwrap_or_else(PoisonError::into_inner);
        let config = kind_config(&configured, subject.kind());
        let mut named: Vec<_> = std::mem::take(&mut resolved.actions)
            .into_iter()
            .map(|action| (Some(action.id.clone()), action))
            .collect();
        named.extend(composed(config, &provided, subject, title, Producer::Dupin));
        resolved.actions = arrange(named, config);
        resolved
    }

    /// The kind's own action Enter runs on a row of `subject`, as an extension's row that
    /// copies or opens borrows it.
    pub fn primary(&self, subject: &Subject, title: &str) -> Option<ItemAction> {
        let resolved = self.get(subject.kind()).actions(subject, title);
        let primary = resolved.primary?;
        resolved.actions.into_iter().find(|action| action.id == primary)
    }

    /// The actions config.toml adds for `subject`, titled `title`, which follow its kind's own,
    /// then the ones packages compose by default. A row `producer` made takes a package's kind
    /// action only when Dupin produced it.
    pub fn configured(&self, subject: &Subject, title: &str, producer: Producer) -> Vec<ItemAction> {
        let configured = self.configured.read().unwrap_or_else(PoisonError::into_inner);
        let provided = self.provided.read().unwrap_or_else(PoisonError::into_inner);
        composed(
            kind_config(&configured, subject.kind()),
            &provided,
            subject,
            title,
            producer,
        )
        .into_iter()
        .map(|(_, action)| action)
        .collect()
    }

    /// A file or folder row, as every native file source shows one.
    pub fn file(&self, path: PathBuf, title: String, matched: Vec<usize>, folder: bool) -> Item {
        let resolved = self.resolve(
            &Subject::File {
                path: path.clone(),
                folder,
            },
            &title,
        );
        Item::path_with(path, title, matched, resolved)
    }

    /// An application row.
    pub fn app(&self, path: PathBuf, title: String, matched: Vec<usize>) -> Item {
        let resolved = self.resolve(&Subject::App(path.clone()), &title);
        Item::path_with(path, title, matched, resolved)
    }

    /// What Send to… offers for `payload`: each kind's choices for the part it carries, text
    /// first.
    pub fn send_to(&self, payload: &Payload) -> Vec<ItemAction> {
        KindName::ALL
            .into_iter()
            .flat_map(|name| self.get(name).send_to(payload))
            .collect()
    }
}

fn kind_config(configured: &KindActions, kind: KindName) -> &KindConfig {
    match kind {
        KindName::Text => &configured.text,
        KindName::Url => &configured.url,
        KindName::File => &configured.file,
        KindName::App => &configured.app,
    }
}

/// The actions `config` adds for `subject`, titled `title`, then the ones `provided` composes
/// by default that `config` doesn't add itself, each with the name `hide` and `order` know it
/// by: a package's kind action is `<package>/<action>`, and a script or URL action has none,
/// since its place in `actions` already says where it goes.
fn composed(
    config: &KindConfig,
    provided: &[Provided],
    subject: &Subject,
    title: &str,
    producer: Producer,
) -> Vec<(Option<String>, ItemAction)> {
    let kind = subject.kind();
    let declared = |reference: Option<&str>| {
        let (package, name) = crate::config::contribution(reference?)?;
        provided.iter().find(|found| found.is(kind, package, name))
    };
    let mut actions: Vec<_> = config
        .actions
        .iter()
        .enumerate()
        .filter(|(_, action)| producer == Producer::Dupin || action.extension.is_none())
        .filter_map(|(index, action)| {
            let item = configured_action(action, &configured_id(kind.as_str(), index), subject)?;
            let item = match declared(action.extension.as_deref()) {
                Some(declaration) => declaration.shape(item, subject, title),
                None => item,
            };
            Some((action.extension.clone(), item))
        })
        .collect();
    if producer == Producer::Dupin {
        actions.extend(
            provided
                .iter()
                .filter(|declaration| declaration.kind == kind && declaration.default)
                .filter(|declaration| !added(config, &declaration.package, &declaration.name))
                .filter_map(|declaration| {
                    let item = package_action(
                        &provided_id(kind.as_str(), &declaration.package, &declaration.name),
                        declaration.title.as_deref()?,
                        &declaration.package,
                        &declaration.name,
                        subject,
                    );
                    Some((
                        Some(format!("{}/{}", declaration.package, declaration.name)),
                        declaration.shape(item, subject, title),
                    ))
                }),
        );
    }
    actions
}

/// `actions` with the ones `config.order` names first, in its order, then the rest in theirs,
/// and the ones `config.hide` names left out of the action screen. A hidden action keeps its
/// route and shortcut: Enter, ⌘Enter and ⌘Y still find it.
fn arrange(actions: Vec<(Option<String>, ItemAction)>, config: &KindConfig) -> Vec<ItemAction> {
    if config.hide.is_empty() && config.order.is_empty() {
        return actions.into_iter().map(|(_, action)| action).collect();
    }
    let mut placed: Vec<_> = actions
        .into_iter()
        .map(|(name, mut action)| {
            let name = name.as_deref();
            let place = name
                .and_then(|name| config.order.iter().position(|ordered| ordered == name))
                .unwrap_or(config.order.len());
            if name.is_some_and(|name| config.hide.iter().any(|hidden| hidden == name)) {
                action.hidden = true;
            }
            (place, action)
        })
        .collect();
    placed.sort_by_key(|(place, _)| *place);
    placed.into_iter().map(|(_, action)| action).collect()
}

/// The ID of the `index`th action config.toml adds to `kind`, which no source's own action
/// takes, so a row with both text and a URL tells their configured actions apart.
pub fn configured_id(kind: &str, index: usize) -> String {
    format!("configured-{kind}-{index}")
}

/// The ID of the kind action `package` provides as `name` and Dupin composes by default.
fn provided_id(kind: &str, package: &str, name: &str) -> String {
    format!("provided-{kind}-{package}-{name}")
}

/// Whether config.toml's `actions` already add `package`'s `name` to this kind, under a title
/// of its own, so its default isn't added twice.
fn added(config: &KindConfig, package: &str, name: &str) -> bool {
    config
        .actions
        .iter()
        .filter_map(|action| action.extension.as_deref())
        .any(|reference| crate::config::contribution(reference) == Some((package, name)))
}

/// The kind action `package` provides as `name`, run on `subject`.
fn package_action(id: &str, title: &str, package: &str, name: &str, subject: &Subject) -> ItemAction {
    ItemAction::new(
        id,
        title,
        Action::Extension {
            extension: package.to_owned(),
            request: crate::ExtensionRequest::Kind {
                action: name.to_owned(),
                subject: subject.clone(),
            },
        },
    )
    .keep_open()
}

/// `action` for `subject`, or none when the subject has no value the action can take: a path
/// that isn't UTF-8 would reach a script or a URL as some other path.
fn configured_action(action: &KindAction, id: &str, subject: &Subject) -> Option<ItemAction> {
    let (value, name) = match subject {
        Subject::Text(text) => (text.as_str(), None),
        Subject::Url(url) => (url.as_str(), None),
        Subject::File { path, .. } | Subject::App(path) => (
            path.to_str()?,
            Some(path.file_name().and_then(|name| name.to_str())?),
        ),
    };
    let run = match (&action.script, &action.url) {
        (Some(script), _) => Action::ConfiguredScript {
            script: script.clone(),
            argument: value.to_owned(),
        },
        (None, None) => {
            let (package, name) = crate::config::contribution(action.extension.as_deref()?)?;
            return Some(package_action(id, &action.title, package, name, subject));
        }
        (None, Some(template)) => {
            let encode = dupin_plugin_protocol::web_search::percent_encode;
            let placeholder = match subject.kind() {
                KindName::Text => "{text}",
                KindName::Url => "{url}",
                KindName::File | KindName::App => "{path}",
            };
            let mut url = template.replace(placeholder, &encode(value));
            if let Some(name) = name {
                url = url.replace("{name}", &encode(name));
            }
            Action::OpenUrl(url)
        }
    };
    let configured = ItemAction::new(id, &action.title, run);
    // The UI decides whether a script's run closes the panel, by the script's output mode.
    Some(if action.script.is_some() {
        configured.keep_open()
    } else {
        configured
    })
}

/// A service Send to… offers for files: its action ID, title and target.
pub type ShareChoice = (&'static str, &'static str, ShareTarget);

/// Enter opens, ⌘Enter reveals: what every path row starts with.
pub(crate) fn opening(path: &Path) -> Resolved {
    Resolved {
        actions: vec![
            ItemAction::new("open", "Open", Action::Open(path.to_owned())),
            ItemAction::new("reveal", "Reveal in Finder", Action::Reveal(path.to_owned())),
        ],
        primary: Some("open".into()),
        alternate: Some("reveal".into()),
    }
}

fn quick_look(path: &Path) -> ItemAction {
    ItemAction::new(QUICK_LOOK, "Quick Look", Action::Preview(path.to_owned())).keep_open()
}

struct TextKind;

impl Kind for TextKind {
    fn actions(&self, subject: &Subject, _: &str) -> Resolved {
        let Subject::Text(text) = subject else {
            return Resolved::default();
        };
        Resolved {
            actions: vec![ItemAction::new("copy", "Copy", Action::Copy(text.clone()))],
            primary: Some("copy".into()),
            alternate: None,
        }
    }

    fn send_to(&self, payload: &Payload) -> Vec<ItemAction> {
        payload
            .nonblank_text()
            .map(|text| ItemAction::new("copy-text", "Copy", Action::Copy(text.into())))
            .into_iter()
            .collect()
    }
}

struct UrlKind;

impl Kind for UrlKind {
    fn actions(&self, subject: &Subject, _: &str) -> Resolved {
        let Subject::Url(url) = subject else {
            return Resolved::default();
        };
        Resolved {
            actions: vec![ItemAction::new("open", "Open URL", Action::OpenUrl(url.clone()))],
            primary: Some("open".into()),
            alternate: None,
        }
    }

    fn send_to(&self, _: &Payload) -> Vec<ItemAction> {
        Vec::new()
    }
}

struct AppKind;

impl Kind for AppKind {
    fn actions(&self, subject: &Subject, _: &str) -> Resolved {
        let Subject::App(path) = subject else {
            return Resolved::default();
        };
        let mut resolved = opening(path);
        resolved.actions.push(quick_look(path));
        resolved
    }

    fn send_to(&self, _: &Payload) -> Vec<ItemAction> {
        Vec::new()
    }
}

struct FileKind {
    share: Vec<ShareChoice>,
}

impl Kind for FileKind {
    fn actions(&self, subject: &Subject, title: &str) -> Resolved {
        let Subject::File { path, folder } = subject else {
            return Resolved::default();
        };
        let mut resolved = opening(path);
        let mut actions = Vec::with_capacity(11);
        let mut opened = std::mem::take(&mut resolved.actions).into_iter();
        actions.extend(opened.next());
        if *folder {
            actions.push(ItemAction::new("browse", "Browse", Action::Browse(path.clone())));
        }
        actions.push(ItemAction::new(
            "open-with",
            "Open with…",
            Action::ChooseApplication(path.clone()),
        ));
        actions.extend(opened);
        actions.push(quick_look(path));
        let name = path.file_name();
        if let Some(name) = name {
            actions.push(ItemAction::new(
                "copy-name",
                "Copy name",
                Action::Copy(name.to_string_lossy().into_owned()),
            ));
        }
        // Renaming, transferring and the copies made beside the original all need a name
        // they can write back.
        if name.and_then(|name| name.to_str()).is_some() {
            actions.extend([
                ItemAction::new("rename", "Rename…", Action::Rename(path.clone())),
                ItemAction::new(
                    "move-to",
                    "Move to…",
                    Action::ChooseDestination {
                        path: path.clone(),
                        transfer: Transfer::Move,
                    },
                ),
                ItemAction::new(
                    "copy-to",
                    "Copy to…",
                    Action::ChooseDestination {
                        path: path.clone(),
                        transfer: Transfer::Copy,
                    },
                ),
                ItemAction::new("duplicate", "Duplicate", Action::Duplicate(path.clone())).keep_open(),
                ItemAction::new("compress", "Compress", Action::Compress(path.clone())).keep_open(),
            ]);
        }
        actions.push(
            ItemAction::new("trash", "Move to Trash", Action::Trash(path.clone()))
                .confirm(&format!("Move “{title}” to the Trash?"), "Move to Trash")
                .consumes(),
        );
        resolved.actions = actions;
        resolved
    }

    fn send_to(&self, payload: &Payload) -> Vec<ItemAction> {
        let paths = &payload.paths;
        if paths.is_empty() {
            return Vec::new();
        }
        let mut choices: Vec<ItemAction> = self
            .share
            .iter()
            .map(|(id, title, target)| {
                let action = ItemAction::new(
                    id,
                    title,
                    Action::Share {
                        target: target.clone(),
                        paths: paths.clone(),
                    },
                );
                if *target == ShareTarget::Picker {
                    action.keep_open()
                } else {
                    action
                }
            })
            .collect();
        let text = paths
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        let title = if paths.len() == 1 {
            "Copy path"
        } else {
            "Copy paths"
        };
        choices.push(ItemAction::new("copy-path", title, Action::Copy(text)));
        choices
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    fn kinds() -> Kinds {
        Kinds::native(vec![
            ("share", "Share…", ShareTarget::Picker),
            (
                "mail",
                "New Mail with Attachment",
                ShareTarget::Service {
                    name: "mail".into(),
                    label: "Mail".into(),
                },
            ),
        ])
    }

    fn ids(actions: &[ItemAction]) -> Vec<&str> {
        actions.iter().map(|action| action.id.as_str()).collect()
    }

    #[test]
    fn files_offer_ordered_choices_and_confirmed_trash() {
        for path in [
            r"/fixture/file\name.txt",
            "/fixture/folder",
            "/fixture/Example.app",
        ] {
            let item = kinds().file(path.into(), "Example".into(), vec![], false);
            assert_eq!(
                ids(&item.actions),
                [
                    "open",
                    "open-with",
                    "reveal",
                    QUICK_LOOK,
                    "copy-name",
                    "rename",
                    "move-to",
                    "copy-to",
                    "duplicate",
                    "compress",
                    "trash"
                ]
            );
            assert_eq!(
                (item.primary.as_deref(), item.alternate.as_deref()),
                (Some("open"), Some("reveal"))
            );
            let trash = item.item_action("trash").unwrap();
            assert_eq!(
                trash.confirmation().unwrap().prompt,
                "Move “Example” to the Trash?"
            );
            assert_eq!(trash.disposition, crate::ActionDisposition::Consume);
            assert_eq!(
                item.item_action(QUICK_LOOK).unwrap().disposition,
                crate::ActionDisposition::KeepOpen
            );
            assert_eq!(item.payload.paths, [PathBuf::from(path)]);
        }
    }

    #[test]
    fn a_folder_row_is_exactly_the_row_file_search_showed() {
        let path = PathBuf::from("/Users/test/Projects");
        let item = kinds().file(path.clone(), "Projects".into(), vec![0, 1], true);
        let p = || path.clone();
        assert_eq!(
            item,
            Item {
                id: "/Users/test/Projects".into(),
                view: crate::View::Text {
                    title: "Projects".into(),
                    subtitle: crate::item::tilde("/Users/test"),
                },
                payload: Payload {
                    text: None,
                    paths: vec![p()],
                },
                record_usage: true,
                icon: Some(crate::Icon::File(p())),
                matched: vec![0, 1],
                completion: None,
                score: None,
                primary: Some("open".into()),
                alternate: Some("reveal".into()),
                actions: vec![
                    ItemAction::new("open", "Open", Action::Open(p())),
                    ItemAction::new("browse", "Browse", Action::Browse(p())),
                    ItemAction::new("open-with", "Open with…", Action::ChooseApplication(p())),
                    ItemAction::new("reveal", "Reveal in Finder", Action::Reveal(p())),
                    ItemAction::new("quick-look", "Quick Look", Action::Preview(p())).keep_open(),
                    ItemAction::new("copy-name", "Copy name", Action::Copy("Projects".into())),
                    ItemAction::new("rename", "Rename…", Action::Rename(p())),
                    ItemAction::new(
                        "move-to",
                        "Move to…",
                        Action::ChooseDestination {
                            path: p(),
                            transfer: Transfer::Move,
                        },
                    ),
                    ItemAction::new(
                        "copy-to",
                        "Copy to…",
                        Action::ChooseDestination {
                            path: p(),
                            transfer: Transfer::Copy,
                        },
                    ),
                    ItemAction::new("duplicate", "Duplicate", Action::Duplicate(p())).keep_open(),
                    ItemAction::new("compress", "Compress", Action::Compress(p())).keep_open(),
                    ItemAction::new("trash", "Move to Trash", Action::Trash(p()))
                        .confirm("Move “Projects” to the Trash?", "Move to Trash")
                        .consumes(),
                ],
            }
        );
    }

    #[test]
    fn folders_can_be_browsed() {
        let path = PathBuf::from("/fixture/folder");
        let item = kinds().file(path.clone(), "folder".into(), vec![], true);
        assert_eq!(ids(&item.actions)[..3], ["open", "browse", "open-with"]);
        assert_eq!(item.action("browse"), Some(&Action::Browse(path)));
    }

    #[test]
    fn a_non_unicode_basename_hides_actions_that_require_a_usable_name() {
        let path = PathBuf::from(OsStr::from_bytes(b"/fixture/file-\xff.txt"));
        let item = kinds().file(path.clone(), "Example".into(), vec![], false);
        assert_eq!(item.payload.paths, [path]);
        assert_eq!(
            ids(&item.actions),
            ["open", "open-with", "reveal", QUICK_LOOK, "copy-name", "trash"]
        );
    }

    #[test]
    fn apps_open_reveal_and_preview() {
        let path = PathBuf::from("/Applications/Safari.app");
        let item = kinds().app(path.clone(), "Safari".into(), vec![0]);
        assert_eq!(ids(&item.actions), ["open", "reveal", QUICK_LOOK]);
        assert_eq!(item.action(QUICK_LOOK), Some(&Action::Preview(path.clone())));
        assert_eq!(
            (item.primary.as_deref(), item.alternate.as_deref()),
            (Some("open"), Some("reveal"))
        );
        assert_eq!(item.matched, [0]);
        assert_eq!(item.id, path.to_string_lossy());
    }

    #[test]
    fn a_kind_resolves_nothing_for_another_kinds_subject() {
        let kinds = kinds();
        let subjects = [
            Subject::Text("text".into()),
            Subject::Url("https://example.com".into()),
            Subject::File {
                path: "/fixture".into(),
                folder: false,
            },
            Subject::App("/fixture.app".into()),
        ];
        for name in KindName::ALL {
            for subject in &subjects {
                let resolved = kinds.get(name).actions(subject, "Title");
                assert_eq!(
                    resolved.actions.is_empty(),
                    subject.kind() != name,
                    "{name:?} {subject:?}"
                );
            }
        }
    }

    #[test]
    fn text_and_url_kinds_copy_and_open() {
        let kinds = kinds();
        let text = kinds.resolve(&Subject::Text("2".into()), "2");
        assert_eq!(
            text.actions,
            [ItemAction::new("copy", "Copy", Action::Copy("2".into()))]
        );
        assert_eq!(text.primary.as_deref(), Some("copy"));
        let url = kinds.resolve(&Subject::Url("https://example.com".into()), "Example");
        assert_eq!(
            url.actions,
            [ItemAction::new(
                "open",
                "Open URL",
                Action::OpenUrl("https://example.com".into())
            )]
        );
        assert_eq!(url.primary.as_deref(), Some("open"));
    }

    fn running_apps(name: &str, title: &str, confirm: bool, default: bool) -> Provided {
        Provided {
            kind: KindName::App,
            package: "running-apps".into(),
            name: name.into(),
            title: Some(title.into()),
            confirm,
            availability: Some(Availability::Running),
            default,
        }
    }

    #[test]
    fn a_packages_default_kind_actions_follow_configured_ones_and_hide_and_order_arrange_them() {
        let kinds = kinds();
        let path = PathBuf::from("/Applications/Safari.app");
        let app = |kinds: &Kinds| kinds.app(path.clone(), "Safari".into(), vec![]);
        let native = ids(&app(&kinds).actions).len();
        assert!(kinds.provide(vec![
            running_apps("hide", "Hide", false, true),
            running_apps("quit", "Quit", false, true),
            running_apps("quit_forcibly", "Force Quit…", true, true),
            running_apps("details", "Details", false, false),
        ]));
        let same = kinds.provided.read().unwrap().clone();
        assert!(!kinds.provide(same));
        let item = app(&kinds);
        assert_eq!(
            ids(&item.actions)[native..],
            [
                "provided-app-running-apps-hide",
                "provided-app-running-apps-quit",
                "provided-app-running-apps-quit_forcibly"
            ]
        );
        let force = item
            .item_action("provided-app-running-apps-quit_forcibly")
            .unwrap();
        assert_eq!(force.title, "Force Quit…");
        let confirmation = force.confirmation().unwrap();
        assert_eq!(
            (confirmation.prompt.as_str(), confirmation.button.as_str()),
            ("Force Quit “Safari”?", "Force Quit")
        );
        assert_eq!(force.condition(), Some(&Condition::Running(path.clone())));
        assert_eq!(
            force.action,
            Action::Extension {
                extension: "running-apps".into(),
                request: crate::ExtensionRequest::Kind {
                    action: "quit_forcibly".into(),
                    subject: Subject::App(path.clone()),
                },
            }
        );
        assert!(
            item.item_action("provided-app-running-apps-quit")
                .unwrap()
                .confirmation()
                .is_none()
        );
        assert!(
            kinds
                .configured(&Subject::App(path.clone()), "Safari", Producer::Extension)
                .is_empty(),
            "an extension's rows never take a package's action"
        );
        assert!(
            ids(&kinds.file("/a.txt".into(), "a.txt".into(), vec![], false).actions)
                .iter()
                .all(|id| !id.starts_with("provided-")),
            "an app action stays on app rows"
        );

        kinds.configure(&configured(
            r#"
[advanced.kinds.app]
hide = ["running-apps/hide"]
order = ["open", "running-apps/quit"]
"#,
        ));
        assert_eq!(
            listed(&app(&kinds).actions),
            [
                "open",
                "provided-app-running-apps-quit",
                "reveal",
                QUICK_LOOK,
                "provided-app-running-apps-quit_forcibly"
            ],
            "`hide` and `order` arrange a default action as they do a native one"
        );

        kinds.configure(&configured(
            r#"
[[advanced.kinds.app.actions]]
title = "Force Quit Now"
extension = "running-apps/quit_forcibly"
"#,
        ));
        let item = app(&kinds);
        assert_eq!(
            ids(&item.actions)[native..],
            [
                configured_id("app", 0).as_str(),
                "provided-app-running-apps-hide",
                "provided-app-running-apps-quit"
            ],
            "an action config.toml adds under its own title isn't added again by default"
        );
        let own = item.item_action(&configured_id("app", 0)).unwrap();
        assert_eq!(
            own.confirmation().unwrap().prompt,
            "Force Quit Now “Safari”?",
            "what the package declares holds wherever it is composed"
        );
        assert_eq!(own.condition(), Some(&Condition::Running(path.clone())));

        kinds.configure(&KindActions::default());
        assert!(kinds.provide(Vec::new()));
        assert_eq!(
            ids(&app(&kinds).actions).len(),
            native,
            "a package that stops takes its defaults with it"
        );
    }

    fn configured(source: &str) -> KindActions {
        crate::Config::parse(source).unwrap().advanced.kinds
    }

    #[test]
    fn configured_actions_follow_their_kinds_own_and_reach_every_clone() {
        let kinds = kinds();
        let source = kinds.clone();
        let path = PathBuf::from("/Users/test/a b.txt");
        let before = source.file(path.clone(), "a b.txt".into(), vec![], false);
        assert!(!kinds.configure(&KindActions::default()));
        assert!(kinds.configure(&configured(
            r#"
[[advanced.kinds.file.actions]]
title = "Open in Zed"
script = "open-in-zed"

[[advanced.kinds.file.actions]]
title = "Search the name"
url = "https://search.example/?q={name}&in={path}"

[[advanced.kinds.url.actions]]
title = "Archive"
url = "https://web.archive.org/save/{url}"
"#
        )));
        let after = source.file(path.clone(), "a b.txt".into(), vec![], false);
        assert_eq!(after.actions[..before.actions.len()], before.actions[..]);
        assert_eq!(
            (&after.primary, &after.alternate),
            (&before.primary, &before.alternate)
        );
        let zed = after.item_action(&configured_id("file", 0)).unwrap();
        assert_eq!(zed.title, "Open in Zed");
        assert_eq!(
            zed.action,
            Action::ConfiguredScript {
                script: "open-in-zed".into(),
                argument: "/Users/test/a b.txt".into()
            }
        );
        assert_eq!(zed.disposition, crate::ActionDisposition::KeepOpen);
        assert_eq!(
            after.action(&configured_id("file", 1)),
            Some(&Action::OpenUrl(
                "https://search.example/?q=a%20b.txt&in=%2FUsers%2Ftest%2Fa%20b.txt".into()
            ))
        );
        assert_eq!(after.actions.len(), before.actions.len() + 2);

        let url = source.resolve(&Subject::Url("https://example.com/?a=1".into()), "Example");
        assert_eq!(
            url.actions[1].action,
            Action::OpenUrl("https://web.archive.org/save/https%3A%2F%2Fexample.com%2F%3Fa%3D1".into())
        );
        let app = source.app("/Applications/Safari.app".into(), "Safari".into(), vec![]);
        assert_eq!(ids(&app.actions), ["open", "reveal", QUICK_LOOK]);
        let text = source.resolve(&Subject::Text("x".into()), "x");
        assert_eq!(ids(&text.actions), ["copy"]);

        assert!(kinds.configure(&KindActions::default()));
        assert_eq!(source.file(path, "a b.txt".into(), vec![], false), before);
    }

    #[test]
    fn a_configured_action_skips_a_path_it_would_misname() {
        let kinds = kinds();
        kinds.configure(&configured(
            "[[advanced.kinds.file.actions]]\ntitle = \"Run\"\nscript = \"run\"",
        ));
        let path = PathBuf::from(OsStr::from_bytes(b"/fixture/file-\xff.txt"));
        let item = kinds.file(path, "Example".into(), vec![], false);
        assert!(item.item_action(&configured_id("file", 0)).is_none());
    }

    #[test]
    fn send_to_follows_the_payload_types() {
        let kinds = kinds();
        assert!(kinds.send_to(&Payload::default()).is_empty());
        assert!(kinds.send_to(&Payload::text(" \n ")).is_empty());
        assert_eq!(ids(&kinds.send_to(&Payload::text("Definition"))), ["copy-text"]);
        let mut payload = Payload {
            text: Some("  Definition  ".into()),
            paths: vec!["/tmp/file.txt".into()],
        };
        assert_eq!(
            ids(&kinds.send_to(&payload)),
            ["copy-text", "share", "mail", "copy-path"]
        );
        payload.paths.push("/tmp/another file.txt".into());
        let actions = kinds.send_to(&payload);
        let action = |id: &str| actions.iter().find(|action| action.id == id).unwrap();
        assert_eq!(action("copy-text").action, Action::Copy("  Definition  ".into()));
        assert_eq!(action("copy-path").title, "Copy paths");
        assert_eq!(
            action("copy-path").action,
            Action::Copy("/tmp/file.txt\n/tmp/another file.txt".into())
        );
        assert_eq!(action("share").disposition, crate::ActionDisposition::KeepOpen);
        assert_eq!(action("mail").disposition, crate::ActionDisposition::Dismiss);
        assert_eq!(
            action("mail").action,
            Action::Share {
                target: ShareTarget::Service {
                    name: "mail".into(),
                    label: "Mail".into()
                },
                paths: payload.paths.clone()
            }
        );
    }

    #[test]
    fn every_native_action_has_its_documented_id_in_the_documented_order() {
        let folder = kinds().file("/fixture/folder".into(), "folder".into(), vec![], true);
        assert_eq!(ids(&folder.actions), FILE_ACTIONS);
        let file = kinds().file("/fixture/a.txt".into(), "a.txt".into(), vec![], false);
        let without_browse: Vec<_> = FILE_ACTIONS.into_iter().filter(|id| *id != "browse").collect();
        assert_eq!(ids(&file.actions), without_browse);
        let app = kinds().app("/Applications/Safari.app".into(), "Safari".into(), vec![]);
        assert_eq!(ids(&app.actions), APP_ACTIONS);
    }

    fn listed(actions: &[ItemAction]) -> Vec<&str> {
        actions
            .iter()
            .filter(|action| action.is_listed())
            .map(|action| action.id.as_str())
            .collect()
    }

    #[test]
    fn hide_and_order_arrange_the_list_and_keep_routes_and_shortcuts() {
        let kinds = kinds();
        let path = PathBuf::from("/fixture/a.txt");
        let before = kinds.file(path.clone(), "a.txt".into(), vec![], false);
        kinds.configure(&configured(
            r#"
[advanced.kinds.file]
hide = ["quick-look", "open", "duplicate"]
order = ["trash", "reveal"]

[advanced.kinds.app]
order = ["quick-look"]
"#,
        ));
        let item = kinds.file(path.clone(), "a.txt".into(), vec![], false);
        assert_eq!(
            listed(&item.actions),
            [
                "trash",
                "reveal",
                "open-with",
                "copy-name",
                "rename",
                "move-to",
                "copy-to",
                "compress"
            ]
        );
        assert_eq!(item.actions.len(), before.actions.len(), "hidden, not removed");
        assert_eq!(
            (item.primary.as_deref(), item.alternate.as_deref()),
            (Some("open"), Some("reveal"))
        );
        assert_eq!(item.action("open"), Some(&Action::Open(path.clone())));
        assert_eq!(item.action(QUICK_LOOK), Some(&Action::Preview(path)));
        assert!(item.has_actions());

        let folder = kinds.file("/fixture/folder".into(), "folder".into(), vec![], true);
        assert_eq!(
            listed(&folder.actions)[..4],
            ["trash", "reveal", "browse", "open-with"]
        );
        let app = kinds.app("/Applications/Safari.app".into(), "Safari".into(), vec![]);
        assert_eq!(listed(&app.actions), [QUICK_LOOK, "open", "reveal"]);
        assert_eq!(
            listed(&kinds.resolve(&Subject::Text("x".into()), "x").actions),
            ["copy"]
        );

        kinds.configure(&KindActions::default());
        assert_eq!(
            kinds.file("/fixture/a.txt".into(), "a.txt".into(), vec![], false),
            before
        );
    }

    #[test]
    fn order_places_a_packages_kind_action_and_leaves_script_and_url_actions_in_theirs() {
        let kinds = kinds();
        kinds.configure(&configured(
            r#"
[advanced.kinds.app]
order = ["uploader/upload", "reveal"]
hide = ["uploader/send"]

[[advanced.kinds.app.actions]]
title = "Run"
script = "run"

[[advanced.kinds.app.actions]]
title = "Upload"
extension = "uploader/upload"

[[advanced.kinds.app.actions]]
title = "Send"
extension = "uploader/send"

[[advanced.kinds.app.actions]]
title = "Search"
url = "https://search.example/?q={name}"
"#,
        ));
        let app = kinds.app("/Applications/Safari.app".into(), "Safari".into(), vec![]);
        assert_eq!(
            listed(&app.actions),
            [
                &configured_id("app", 1),
                "reveal",
                "open",
                QUICK_LOOK,
                &configured_id("app", 0),
                &configured_id("app", 3),
            ]
        );
        assert!(
            app.item_action(&configured_id("app", 2))
                .is_some_and(|action| !action.is_listed())
        );
    }

    #[test]
    fn an_extensions_copy_borrows_the_kinds_own_action_whatever_the_order() {
        let kinds = kinds();
        kinds.configure(&configured(
            "[[advanced.kinds.text.actions]]\ntitle = \"Translate\"\nurl = \"https://translate.example/?q={text}\"",
        ));
        assert_eq!(
            kinds.primary(&Subject::Text("x".into()), "x"),
            Some(ItemAction::new("copy", "Copy", Action::Copy("x".into())))
        );
        assert_eq!(
            kinds
                .primary(&Subject::Url("https://example.com".into()), "Example")
                .map(|action| action.id),
            Some("open".into())
        );
    }
}
