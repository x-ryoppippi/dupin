//! What the user set for each package, which its `items()` reads as `ctx.preferences`. A
//! bundled package that replaces a built-in gets the config.toml keys of that built-in, so
//! settings made before the port keep working. A catalogue loaded with other preferences than
//! the current ones is asked for again from the start.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use dupin_extension_bridge::PackageId;
use dupin_js_runtime::Preferences;

fn held() -> &'static RwLock<HashMap<String, Arc<Preferences>>> {
    static HELD: OnceLock<RwLock<HashMap<String, Arc<Preferences>>>> = OnceLock::new();
    HELD.get_or_init(RwLock::default)
}

fn none() -> &'static Arc<Preferences> {
    static NONE: OnceLock<Arc<Preferences>> = OnceLock::new();
    NONE.get_or_init(Arc::default)
}

/// Sets the preferences of `package`, and says whether they changed.
pub fn set(package: &str, preferences: Preferences) -> bool {
    let mut held = held().write().unwrap_or_else(|poisoned| poisoned.into_inner());
    if held.get(package).is_some_and(|current| **current == preferences) {
        return false;
    }
    if preferences.is_empty() {
        return held.remove(package).is_some();
    }
    held.insert(package.to_owned(), Arc::new(preferences));
    true
}

/// The preferences of `package`: the same value until they are set again.
pub fn of(package: &PackageId) -> Arc<Preferences> {
    held()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(package.as_str())
        .unwrap_or_else(|| none())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_stay_the_same_value_until_they_change() {
        let package = PackageId::new("bundled:preferences-test").unwrap();
        assert!(of(&package).is_empty());
        let mut preferences = Preferences::new();
        preferences.insert("a".into(), 1.into());
        assert!(set(package.as_str(), preferences.clone()));
        let first = of(&package);
        assert!(!set(package.as_str(), preferences));
        assert!(Arc::ptr_eq(&first, &of(&package)));
        assert!(set(package.as_str(), Preferences::new()));
        assert!(of(&package).is_empty());
        assert!(!set(package.as_str(), Preferences::new()));
    }
}
