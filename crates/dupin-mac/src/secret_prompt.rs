//! A modal prompt with a secure text field, for a passphrase: what is typed is never drawn,
//! never offered to the accessibility tree as text, and the field is cleared before it goes.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSApplication, NSSecureTextField};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use zeroize::Zeroizing;

/// Asks for a secret with `message` and `detail`. Returns `None` when the user cancels or
/// leaves it empty, or off the main thread, where AppKit can't show the alert.
pub fn ask(message: &str, detail: &str, confirm: &str) -> Option<Zeroizing<String>> {
    let mtm = MainThreadMarker::new()?;
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(message));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.addButtonWithTitle(&NSString::from_str(confirm));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let field: Retained<NSSecureTextField> = NSSecureTextField::initWithFrame(
        mtm.alloc(),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 24.0)),
    );
    alert.setAccessoryView(Some(&field));
    alert.window().setInitialFirstResponder(Some(&field));
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    let answer = alert.runModal();
    let typed = Zeroizing::new(field.stringValue().to_string());
    field.setStringValue(&NSString::from_str(""));
    (answer == NSAlertFirstButtonReturn && !typed.is_empty()).then_some(typed)
}
