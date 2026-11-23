use std::path::PathBuf;

use dupin_core::item::Transfer;
use dupin_core::{Action, Item, ItemAction};

/// A file or folder result with every file action, as Spotlight results and renamed entries
/// show it.
pub fn item(path: PathBuf, title: String, matched: Vec<usize>) -> Item {
    let mut item = Item::path(path, title, matched);
    crate::quick_look::add_action(&mut item);
    add(&mut item);
    item
}

pub(crate) fn add(item: &mut Item) {
    let Some(Action::Open(path)) = item.action("open") else {
        return;
    };
    let path = path.clone();
    let prompt = format!("Move “{}” to the Trash?", item.display_title());
    let index = item
        .actions
        .iter()
        .position(|action| action.id == "open")
        .unwrap()
        + 1;
    item.actions.insert(
        index,
        ItemAction::new("open-with", "Open with…", Action::ChooseApplication(path.clone())),
    );
    if let Some(name) = path.file_name() {
        item.actions.push(ItemAction::new(
            "copy-name",
            "Copy name",
            Action::Copy(name.to_string_lossy().into_owned()),
        ));
    }
    // The input edits text, so a name that isn't valid Unicode can't be shown for editing.
    if path.file_name().is_some_and(|name| name.to_str().is_some()) {
        item.actions.push(ItemAction::new(
            "rename",
            "Rename…",
            Action::Rename {
                path: path.clone(),
                extension: crate::file_ops::has_extension(&path),
            },
        ));
    }
    item.actions.extend([
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
        ItemAction::new("trash", "Move to Trash", Action::Trash(path))
            .confirm(&prompt, "Move to Trash")
            .consumes(),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_offer_ordered_choices_and_confirmed_trash() {
        for path in ["/fixture/file.txt", "/fixture/folder", "/fixture/Example.app"] {
            let item = item(path.into(), "Example".into(), vec![]);
            assert_eq!(
                item.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
                [
                    "open",
                    "open-with",
                    "reveal",
                    "quick-look",
                    "copy-path",
                    "copy-name",
                    "rename",
                    "move-to",
                    "copy-to",
                    "duplicate",
                    "compress",
                    "trash"
                ]
            );
            assert!(item.item_action("trash").unwrap().confirmation().is_some());
        }
    }

    #[test]
    fn non_path_results_have_no_file_actions() {
        let mut item = Item::path("/fixture".into(), "Fixture".into(), vec![]);
        item.actions.clear();
        add(&mut item);
        assert!(item.actions.is_empty());
    }
}
