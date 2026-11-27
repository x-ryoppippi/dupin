use dupin_core::kind::Kinds;
use dupin_core::{Action, Item, ItemAction, Payload};
use dupin_js_runtime::web_search::WebSearch;

/// What Send to… offers for `payload`: its kinds' choices, then the web searches its text can
/// fill.
pub fn for_payload(kinds: &Kinds, payload: &Payload, web: &[(Item, WebSearch)]) -> Vec<ItemAction> {
    let mut receivers = kinds.send_to(payload);
    if let Some(query) = payload.nonblank_text() {
        receivers.extend(crate::web::search_urls(web, query).map(|(alias, title, url)| {
            ItemAction::new(
                &format!("web:{alias}"),
                &format!("{title} ({alias})"),
                Action::OpenUrl(url),
            )
        }));
    }
    receivers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receivers_follow_the_payload_types() {
        let kinds = dupin_mac::kinds::native();
        let web: &[(Item, WebSearch)] = &[];
        let ids = |actions: Vec<ItemAction>| actions.into_iter().map(|action| action.id).collect::<Vec<_>>();
        assert!(for_payload(&kinds, &Payload::default(), web).is_empty());
        assert!(for_payload(&kinds, &Payload::text(" \n "), web).is_empty());
        assert_eq!(
            ids(for_payload(&kinds, &Payload::text("Definition"), web)),
            ["copy-text"]
        );

        let mut item = Item::path("/tmp/file.txt".into(), "file.txt".into(), vec![]);
        assert_eq!(
            ids(for_payload(&kinds, &item.payload, web)),
            ["share", "airdrop", "mail", "copy-path"]
        );
        item.payload.text = Some("  Definition  ".into());
        item.payload.paths.push("/tmp/another file.txt".into());
        item.actions = for_payload(&kinds, &item.payload, web);
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
