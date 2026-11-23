use dupin_core::window_management::Layout;
use dupin_core::{Action, Item, ItemAction, Provider, ProviderError, Request, View, fuzzy};

pub const COMMANDS: [(Layout, &str, &str); 10] = [
    (Layout::LeftHalf, "left-half", "Left Half"),
    (Layout::RightHalf, "right-half", "Right Half"),
    (Layout::TopHalf, "top-half", "Top Half"),
    (Layout::BottomHalf, "bottom-half", "Bottom Half"),
    (Layout::LeftThird, "left-third", "Left Third"),
    (Layout::CentreThird, "centre-third", "Centre Third"),
    (Layout::RightThird, "right-third", "Right Third"),
    (Layout::Maximise, "maximise", "Maximise"),
    (Layout::Centre, "centre", "Centre"),
    (Layout::NextDisplay, "next-display", "Next Display"),
];

pub struct WindowProvider;

impl Provider for WindowProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let query = request.query.trim().to_ascii_lowercase();
        let browse = matches!(query.as_str(), "window" | "window management");
        let query = query
            .strip_prefix("window ")
            .unwrap_or(&query)
            .replace("center", "centre")
            .replace("maximize", "maximise");
        let mut items: Vec<_> = COMMANDS
            .into_iter()
            .filter_map(|(layout, id, title)| {
                let (score, matched) = if browse {
                    (100, Vec::new())
                } else {
                    let found = fuzzy::score(&query, title)?;
                    (found.score, found.positions)
                };
                Some(Item {
                    payload: Default::default(),
                    id: format!("window:{id}"),
                    view: View::Text {
                        title: title.into(),
                        subtitle: "Window Management".into(),
                    },
                    record_usage: false,
                    icon: None,
                    matched,
                    completion: None,
                    score: Some(score),
                    primary: Some("arrange".into()),
                    alternate: None,
                    actions: vec![ItemAction::new("arrange", title, Action::ArrangeWindow(layout))],
                })
            })
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.score));
        items.truncate(request.limit);
        if !request.is_cancelled() && !items.is_empty() {
            emit(items);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_query_lists_every_command_and_names_match_individually() {
        let search = |query| {
            let mut items = Vec::new();
            WindowProvider
                .search(&Request::detached(query, COMMANDS.len()), &mut |batch| {
                    items.extend(batch)
                })
                .unwrap();
            items
        };
        assert!(search("").is_empty());
        assert_eq!(search("window").len(), COMMANDS.len());
        for (layout, _, title) in COMMANDS {
            let items = search(title);
            assert_eq!(items[0].action("arrange"), Some(&Action::ArrangeWindow(layout)));
        }
        assert_eq!(search("center third")[0].display_title(), "Centre Third");
    }
}
