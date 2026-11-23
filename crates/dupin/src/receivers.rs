use dupin_core::{Action, ItemAction, Payload, ReceiverActions, ShareTarget};

use crate::web::WebCatalog;

pub fn for_payload(payload: &Payload, web: &WebCatalog) -> Vec<ItemAction> {
    let mut receivers = ReceiverActions::new(payload);
    receivers.for_text(|text| [ItemAction::new("copy-text", "Copy", Action::Copy(text.into()))]);
    receivers.for_paths(|paths| {
        dupin_mac::sharing::targets()
            .into_iter()
            .map(move |(id, title, target)| {
                let keep_open = target == ShareTarget::Picker;
                let action = ItemAction::new(
                    id,
                    title,
                    Action::Share {
                        target,
                        paths: paths.to_vec(),
                    },
                );
                if keep_open { action.keep_open() } else { action }
            })
    });
    receivers.for_paths(|paths| {
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
        [ItemAction::new("copy-path", title, Action::Copy(text))]
    });
    receivers.for_text(|query| {
        web.search_urls(query).map(|(keyword, name, url)| {
            ItemAction::new(
                &format!("web:{keyword}"),
                &format!("Search {name} ({keyword})"),
                Action::OpenUrl(url),
            )
        })
    });
    receivers.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_core::Item;

    #[test]
    fn receivers_follow_the_payload_types() {
        let web = WebCatalog::default();
        let ids = |actions: Vec<ItemAction>| actions.into_iter().map(|action| action.id).collect::<Vec<_>>();
        assert!(for_payload(&Payload::default(), &web).is_empty());
        assert!(for_payload(&Payload::text(" \n "), &web).is_empty());
        assert_eq!(
            ids(for_payload(&Payload::text("Definition"), &web)),
            ["copy-text"]
        );

        let mut item = Item::path("/tmp/file.txt".into(), "file.txt".into(), vec![]);
        assert_eq!(
            ids(for_payload(&item.payload, &web)),
            ["share", "airdrop", "mail", "copy-path"]
        );
        item.payload.text = Some("  Definition  ".into());
        item.payload.paths.push("/tmp/another file.txt".into());
        item.actions = for_payload(&item.payload, &web);
        assert_eq!(
            ids(item.actions.clone()),
            ["copy-text", "share", "airdrop", "mail", "copy-path"]
        );
        assert_eq!(
            item.action("copy-text"),
            Some(&Action::Copy("  Definition  ".into()))
        );
        assert_eq!(
            item.action("copy-path"),
            Some(&Action::Copy("/tmp/file.txt\n/tmp/another file.txt".into()))
        );
        for (id, _, target) in dupin_mac::sharing::targets() {
            assert_eq!(
                item.action(id),
                Some(&Action::Share {
                    target,
                    paths: item.payload.paths.clone()
                })
            );
        }
    }
}
