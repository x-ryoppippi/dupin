//! The names macOS knows an app bundle by, for matching it.

use std::path::Path;

use objc2::AnyThread;
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSFileManager, NSMetadataItem, NSString, NSURL};

/// Finder's (localized) display name for `path`, and Spotlight's alternate names and keywords
/// for it. An app Spotlight hasn't indexed has none of the latter.
pub fn lookup(path: &Path) -> (String, Vec<String>) {
    let Some(path) = path.to_str() else {
        return (String::new(), Vec::new());
    };
    autoreleasepool(|_| {
        let ns_path = NSString::from_str(path);
        let display = NSFileManager::defaultManager()
            .displayNameAtPath(&ns_path)
            .to_string();
        let mut extra = Vec::new();
        let url = NSURL::fileURLWithPath(&ns_path);
        if let Some(item) = NSMetadataItem::initWithURL(NSMetadataItem::alloc(), &url) {
            for key in ["kMDItemAlternateNames", "kMDItemKeywords"] {
                if let Some(value) = item.valueForAttribute(&NSString::from_str(key)) {
                    strings(&value, &mut extra);
                }
            }
        }
        (display, extra)
    })
}

/// A string attribute, or each string in an array attribute.
fn strings(value: &AnyObject, out: &mut Vec<String>) {
    if let Some(s) = value.downcast_ref::<NSString>() {
        out.extend(split(&s.to_string()));
    } else if let Some(array) = value.downcast_ref::<NSArray>() {
        for v in array.iter() {
            if let Some(s) = v.downcast_ref::<NSString>() {
                out.extend(split(&s.to_string()));
            }
        }
    }
}

/// Some apps keep all their keywords in one comma-separated string.
fn split(names: &str) -> Vec<String> {
    names
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_comma_separated_keywords() {
        assert_eq!(
            super::split("browser, picker,,chooser"),
            ["browser", "picker", "chooser"]
        );
    }
}
