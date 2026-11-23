use dupin_core::{Action, Item, ItemAction};

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
    item.actions.push(
        ItemAction::new("trash", "Move to Trash", Action::Trash(path))
            .confirm(&prompt, "Move to Trash")
            .consumes(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_offer_ordered_choices_and_confirmed_trash() {
        for path in ["/fixture/file.txt", "/fixture/folder", "/fixture/Example.app"] {
            let mut item = Item::path(path.into(), "Example".into(), vec![]);
            crate::quick_look::add_action(&mut item);
            add(&mut item);
            assert_eq!(
                item.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
                ["open", "open-with", "reveal", "quick-look", "copy-path", "trash"]
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
