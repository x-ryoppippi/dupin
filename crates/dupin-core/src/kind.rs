//! Kinds: what a row stands for, and the actions Dupin offers for it. A source says what each
//! of its rows is; the kind registered for that, not the source, decides which actions the row
//! has, so every source of files offers the same file actions and a configured kind changes
//! them everywhere at once.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::item::Transfer;
use crate::{Action, Item, ItemAction, Payload, ShareTarget};

/// The ID of the action that shows a file in Quick Look, which Space runs.
pub const QUICK_LOOK: &str = "quick-look";

/// The kinds Dupin registers: `text`, `url`, `file` and `app`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KindName {
    Text,
    Url,
    File,
    App,
}

impl KindName {
    const ALL: [Self; 4] = [Self::Text, Self::Url, Self::File, Self::App];
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

impl Subject {
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
    pub actions: Vec<ItemAction>,
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
}

impl Kinds {
    /// Dupin's own kinds, with `share` as the services Send to… offers for files.
    pub fn native(share: Vec<ShareChoice>) -> Self {
        Self {
            text: Arc::new(TextKind),
            url: Arc::new(UrlKind),
            file: Arc::new(FileKind { share }),
            app: Arc::new(AppKind),
        }
    }

    fn get(&self, name: KindName) -> &Arc<dyn Kind> {
        match name {
            KindName::Text => &self.text,
            KindName::Url => &self.url,
            KindName::File => &self.file,
            KindName::App => &self.app,
        }
    }

    pub fn resolve(&self, subject: &Subject, title: &str) -> Resolved {
        self.get(subject.kind()).actions(subject, title)
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
}
