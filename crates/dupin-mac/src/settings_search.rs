use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use dupin_core::{Action, Item, ItemAction, Provider, ProviderError, Request, View, fuzzy};
use objc2::rc::autoreleasepool;
use objc2_foundation::{NSBundle, NSDictionary, NSNumber, NSString};

#[derive(Default)]
pub struct SettingsProvider {
    panes: OnceLock<Vec<Pane>>,
}

struct Pane {
    id: String,
    title: String,
    bundle: PathBuf,
}

impl Provider for SettingsProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.query.trim().is_empty() || request.is_cancelled() {
            return Ok(());
        }
        let panes = self.panes.get_or_init(|| {
            autoreleasepool(|_| discover(Path::new("/System/Library/ExtensionKit/Extensions")))
        });
        let items = rank(panes, &request.query, request.limit);
        if !request.is_cancelled() && !items.is_empty() {
            emit(items);
        }
        Ok(())
    }
}

fn discover(root: &Path) -> Vec<Pane> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    paths
        .into_iter()
        .take(512)
        .filter(|path| path.extension().is_some_and(|extension| extension == "appex"))
        .filter_map(|path| {
            let bundle = NSBundle::bundleWithPath(&NSString::from_str(path.to_str()?))?;
            let attributes = bundle
                .objectForInfoDictionaryKey(&NSString::from_str("EXAppExtensionAttributes"))?
                .downcast::<NSDictionary>()
                .ok()?;
            let point = attributes.objectForKey(&NSString::from_str("EXExtensionPointIdentifier"))?;
            if point.downcast_ref::<NSString>()?.to_string() != "com.apple.Settings.extension.ui" {
                return None;
            }
            let settings = attributes
                .objectForKey(&NSString::from_str("SettingsExtensionAttributes"))?
                .downcast::<NSDictionary>()
                .ok()?;
            let allowed =
                settings.objectForKey(&NSString::from_str("allowsXAppleSystemPreferencesURLScheme"))?;
            if !allowed.downcast_ref::<NSNumber>()?.boolValue() {
                return None;
            }
            let id = bundle.bundleIdentifier()?.to_string();
            if !valid_identifier(&id) {
                return None;
            }
            let title = ["CFBundleDisplayName", "CFBundleName"]
                .into_iter()
                .find_map(|key| {
                    bundle
                        .objectForInfoDictionaryKey(&NSString::from_str(key))?
                        .downcast::<NSString>()
                        .ok()
                        .map(|name| name.to_string())
                })?;
            Some(Pane {
                id,
                title,
                bundle: path,
            })
        })
        .collect()
}

fn valid_identifier(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn rank(panes: &[Pane], query: &str, limit: usize) -> Vec<Item> {
    let mut items = Vec::new();
    for pane in panes {
        if let Some(found) = fuzzy::score(query, &pane.title) {
            items.push(item(
                format!("settings:{}", pane.id),
                &pane.title,
                "System Settings",
                Some(pane.bundle.clone()),
                Action::OpenUrl(format!("x-apple.systempreferences:{}", pane.id)),
                found,
            ));
        }
    }
    if let Some(found) = fuzzy::score(query, "Dupin Settings") {
        items.push(item(
            "settings:dupin".into(),
            "Dupin Settings",
            "Preferences",
            None,
            Action::OpenPreferences,
            found,
        ));
    }
    items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    items.truncate(limit);
    items
}

fn item(
    id: String,
    title: &str,
    subtitle: &str,
    icon: Option<PathBuf>,
    action: Action,
    found: fuzzy::Match,
) -> Item {
    Item {
        id,
        view: View::Text {
            title: title.into(),
            subtitle: subtitle.into(),
        },
        record_usage: true,
        icon,
        matched: found.positions,
        completion: None,
        score: Some(found.score),
        primary: Some("open".into()),
        alternate: None,
        actions: vec![ItemAction::new("open", "Open", action)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes() -> Vec<Pane> {
        [
            ("Bluetooth", "com.apple.BluetoothSettings"),
            ("Displays", "com.apple.Displays-Settings.extension"),
            ("Keyboard", "com.apple.Keyboard-Settings.extension"),
        ]
        .into_iter()
        .map(|(title, id)| Pane {
            id: id.into(),
            title: title.into(),
            bundle: PathBuf::from("/fixture/pane.appex"),
        })
        .collect()
    }

    #[test]
    fn searches_panes_and_uses_their_declared_identifiers() {
        for (query, expected) in [
            (
                "bluetooth",
                "x-apple.systempreferences:com.apple.BluetoothSettings",
            ),
            (
                "displays",
                "x-apple.systempreferences:com.apple.Displays-Settings.extension",
            ),
            (
                "keyboard",
                "x-apple.systempreferences:com.apple.Keyboard-Settings.extension",
            ),
        ] {
            let items = rank(&panes(), query, 10);
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].action("open"), Some(&Action::OpenUrl(expected.into())));
            assert_eq!(items[0].matched.len(), query.len());
        }
        assert!(rank(&panes(), "unrelated-query", 10).is_empty());
        assert!(rank(&panes(), "", 10).is_empty());
        assert!(rank(&panes(), "bluetooth", 0).is_empty());
    }

    #[test]
    fn own_preferences_are_a_host_action() {
        let items = rank(&[], "dupin settings", 10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].action("open"), Some(&Action::OpenPreferences));
    }

    #[test]
    fn identifiers_cannot_add_url_syntax() {
        assert!(valid_identifier("com.apple.Keyboard-Settings.extension"));
        for id in ["", "id?query", "id#fragment", "id/path", "id:other"] {
            assert!(!valid_identifier(id));
        }
    }
}
