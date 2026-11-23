use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dupin_core::{Item, Provider, ProviderError, Request, session::RecentHistory};

use crate::AppsProvider;

pub struct RecentProvider {
    apps: Arc<AppsProvider>,
    history: RecentHistory,
}

impl RecentProvider {
    pub fn new(apps: Arc<AppsProvider>, history: RecentHistory) -> Self {
        Self { apps, history }
    }
}

impl Provider for RecentProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if !request.query.trim().is_empty() || request.limit == 0 || request.is_cancelled() {
            return Ok(());
        }
        let ids = self.history.ranked_ids();
        if ids.is_empty() {
            return Ok(());
        }
        let apps = self.apps.catalog_items();
        let items = collect_items(&ids, &apps, request.limit, || request.is_cancelled());
        if !items.is_empty() && !request.is_cancelled() {
            emit(items);
        }
        Ok(())
    }
}

fn collect_items(
    ids: &[String],
    apps: &HashMap<PathBuf, Item>,
    limit: usize,
    cancelled: impl Fn() -> bool,
) -> Vec<Item> {
    ids.iter()
        .take_while(|_| !cancelled())
        .filter_map(|id| {
            let path = Path::new(id);
            if !path.is_absolute() || !path.exists() {
                return None;
            }
            if let Some(item) = apps.get(path) {
                return Some(item.clone());
            }
            let title = path.file_name()?.to_string_lossy().into_owned();
            Some(crate::file_actions::item(path.to_owned(), title, Vec::new()))
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_paths_do_not_consume_the_limit_and_app_metadata_is_reused() {
        let root = std::env::temp_dir().join(format!("dupin-recent-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let app = root.join("Example.app");
        let file = root.join("report.txt");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(&file, "fixture").unwrap();
        let expected = Item {
            icon: Some(root.join("resolved-icon.app")),
            ..Item::path(app.clone(), "Localized name".into(), Vec::new())
        };
        let apps = HashMap::from([(app.clone(), expected.clone())]);
        let ids = [root.join("missing"), app, file.clone()].map(|p| p.to_string_lossy().into_owned());
        let items = collect_items(&ids, &apps, 2, || false);
        assert_eq!(items[0], expected);
        assert_eq!(items[1].id, file.to_string_lossy());
        assert_eq!(
            items[1],
            crate::file_actions::item(file, "report.txt".into(), Vec::new())
        );
        assert_eq!(collect_items(&ids, &apps, 1, || false).len(), 1);
        assert!(collect_items(&ids, &apps, 2, || true).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
