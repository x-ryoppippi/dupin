//! The application's controller for macOS's shared Quick Look preview panel.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;

use dupin_core::{Action, Item, ItemAction};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSEvent, NSEventType, NSResponder, NSView, NSWindow, NSWindowDelegate};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSString, NSURL};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource, QLPreviewPanelDelegate};

pub const ACTION_ID: &str = "quick-look";

/// Add Quick Look to a result created by `Item::path`.
pub(crate) fn add_action(item: &mut Item) {
    let Some(Action::Open(path)) = item.action("open") else {
        return;
    };
    let action = ItemAction::new(ACTION_ID, "Quick Look", Action::Preview(path.clone())).keep_open();
    let at = item
        .actions
        .iter()
        .position(|candidate| candidate.id == "copy-path")
        .unwrap_or(item.actions.len());
    item.actions.insert(at, action);
}

struct ControllerState {
    item: RefCell<Option<Retained<NSURL>>>,
    owner: Retained<NSView>,
    panel_resigned: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ControllerState]
    struct PreviewController;

    unsafe impl NSObjectProtocol for PreviewController {}

    unsafe impl QLPreviewPanelDataSource for PreviewController {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        unsafe fn number_of_preview_items(&self, _panel: Option<&QLPreviewPanel>) -> isize {
            if self.ivars().item.borrow().is_some() { 1 } else { 0 }
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        unsafe fn preview_item(
            &self,
            _panel: Option<&QLPreviewPanel>,
            index: isize,
        ) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
            (index == 0)
                .then(|| self.ivars().item.borrow().clone())
                .flatten()
                .map(ProtocolObject::from_retained)
        }
    }

    unsafe impl NSWindowDelegate for PreviewController {
        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            self.clear_item();
            let callback = &self.ivars().panel_resigned;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback));
        }
    }

    unsafe impl QLPreviewPanelDelegate for PreviewController {
        #[unsafe(method(previewPanel:handleEvent:))]
        unsafe fn handle_panel_event(
            &self,
            panel: Option<&QLPreviewPanel>,
            event: Option<&NSEvent>,
        ) -> bool {
            let (Some(panel), Some(event)) = (panel, event) else { return Bool::NO };
            if event.r#type() != NSEventType::KeyDown || event.keyCode() != 53 {
                return Bool::NO;
            }
            if !current_controller_is(panel, self) { return Bool::NO; }
            self.clear_item();
            panel.orderOut(None);
            if let Some(window) = self.ivars().owner.window() {
                window.makeKeyWindow();
            }
            true
        }
    }

    impl PreviewController {
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_preview_panel_control(&self, _panel: Option<&QLPreviewPanel>) -> bool {
            self.ivars().item.borrow().is_some()
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_preview_panel_control(&self, panel: Option<&QLPreviewPanel>) {
            let Some(panel) = panel else { return };
            let source: &ProtocolObject<dyn QLPreviewPanelDataSource> = ProtocolObject::from_ref(self);
            let delegate: &AnyObject = self;
            // SAFETY: QL retains neither pointer; this controller is retained by `Previewer`
            // for at least as long as it owns the shared panel.
            unsafe {
                panel.setDataSource(Some(source));
                panel.setDelegate(Some(delegate));
                panel.reloadData();
            }
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end_preview_panel_control(&self, panel: Option<&QLPreviewPanel>) {
            let Some(panel) = panel else { return };
            // SAFETY: Clear only the unretained callbacks that still point at this controller.
            unsafe {
                if panel
                    .dataSource()
                    .is_some_and(|source| same_object(&*source, self))
                {
                    panel.setDataSource(None);
                }
                if panel
                    .delegate()
                    .is_some_and(|delegate| same_object(&*delegate, self))
                {
                    panel.setDelegate(None);
                }
            }
        }
    }
);

impl PreviewController {
    fn new(
        mtm: MainThreadMarker,
        owner: Retained<NSView>,
        panel_resigned: impl Fn() + 'static,
    ) -> Retained<Self> {
        let state = ControllerState {
            item: RefCell::new(None),
            owner,
            panel_resigned: Box::new(panel_resigned),
        };
        let this = Self::alloc(mtm).set_ivars(state);
        // SAFETY: `PreviewController` uses NSResponder's normal initializer and ivars were set.
        unsafe { msg_send![super(this), init] }
    }

    fn clear_item(&self) {
        self.ivars().item.borrow_mut().take();
    }

    fn set_item(&self, item: Retained<NSURL>) {
        *self.ivars().item.borrow_mut() = Some(item);
    }
}

/// Keeps the responder proxy alive while it is linked into Rune's borrowed AppKit view.
pub struct Previewer {
    view: Retained<NSView>,
    controller: Retained<PreviewController>,
    previous: Option<Retained<NSResponder>>,
}

impl Previewer {
    /// The raw Rune view is borrowed only for this call; this retains it before installing the
    /// proxy and restores its original next responder when the previewer is dropped.
    ///
    /// # Safety
    ///
    /// `ns_view` must point to a live AppKit `NSView`, and this function must run on AppKit's main
    /// thread.
    pub unsafe fn attach(ns_view: NonNull<c_void>, panel_resigned: impl Fn() + 'static) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        // SAFETY: The caller guarantees a live NSView while its raw Rune handle is borrowed.
        let view: &NSView = unsafe { ns_view.cast().as_ref() };
        view.window()?;
        // SAFETY: The borrowed Rune view is live here and Retained keeps it alive for restoration.
        let view = unsafe { Retained::retain(view as *const NSView as *mut NSView) }?;
        // SAFETY: AppKit exposes the current responder as an unretained link; retaining it keeps
        // the original chain alive while our proxy temporarily replaces that link.
        let previous = unsafe { view.nextResponder() };
        let controller = PreviewController::new(mtm, view.clone(), panel_resigned);
        // SAFETY: The retained controller and previous responder outlive these unretained links.
        unsafe {
            controller.setNextResponder(previous.as_deref());
            view.setNextResponder(Some(&*controller));
        }

        if let Some(panel) = shared_panel_if_created(mtm) {
            // SAFETY: The responder chain changed without a key-window notification.
            unsafe { panel.updateController() };
        }

        Some(Self {
            view,
            controller,
            previous,
        })
    }

    pub fn show(&self, path: &Path) -> bool {
        self.controller.clear_item();
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        self.controller.set_item(url);
        // SAFETY: Quick Look's shared panel is main-thread-only.
        let Some(panel) = (unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }) else {
            self.controller.clear_item();
            return false;
        };
        // SAFETY: The owner chain changed when its selected URL was set; this asks QL to find us.
        unsafe { panel.updateController() };
        if !current_controller_is(&panel, &self.controller) {
            self.controller.clear_item();
            return false;
        }
        panel.makeKeyAndOrderFront(None);
        true
    }

    pub fn panel_is_key(&self) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let Some(panel) = shared_panel_if_created(mtm) else {
            return false;
        };
        current_controller_is(&panel, &self.controller) && panel.isKeyWindow()
    }

    pub fn owner_is_key(&self) -> bool {
        MainThreadMarker::new().is_some()
            && self
                .view
                .window()
                .is_some_and(|window: Retained<NSWindow>| window.isKeyWindow())
    }

    pub fn finish_focus_transfer(&self) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let panel = shared_panel_if_created(mtm);
        // SAFETY: This main-thread query retains the current controller while inspecting it.
        if panel.as_ref().is_some_and(|panel| unsafe {
            panel
                .currentController()
                .is_some_and(|current| !same_object(&*current, &*self.controller))
        }) {
            self.controller.clear_item();
            return self.owner_is_key();
        }
        let visible = panel.as_ref().is_some_and(|panel| panel.isVisible());
        if !visible {
            self.controller.clear_item();
        }
        let app = NSApplication::sharedApplication(mtm);
        let owner = self.view.window();
        match focus_transfer(
            panel
                .as_ref()
                .is_some_and(|panel| current_controller_is(panel, &self.controller) && panel.isKeyWindow()),
            visible,
            owner.as_ref().is_some_and(|window| window.isKeyWindow()),
            owner.as_ref().is_some_and(|window| window.isVisible()),
            app.isActive(),
            app.keyWindow().is_some(),
        ) {
            FocusTransfer::Keep => true,
            FocusTransfer::Restore => {
                if let Some(owner) = owner {
                    owner.makeKeyWindow();
                }
                true
            }
            FocusTransfer::Dismiss => false,
        }
    }

    pub fn close(&self) {
        self.controller.clear_item();
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(panel) = shared_panel_if_created(mtm) else {
            return;
        };
        if current_controller_is(&panel, &self.controller) {
            panel.orderOut(None);
        }
    }
}

impl Drop for Previewer {
    fn drop(&mut self) {
        self.controller.clear_item();
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        if let Some(panel) = shared_panel_if_created(mtm)
            && current_controller_is(&panel, &self.controller)
        {
            panel.orderOut(None);
        }

        // SAFETY: AppKit responder links are unretained. Restore only if our proxy is still the
        // link installed here, leaving any later framework change untouched.
        unsafe {
            if self
                .view
                .nextResponder()
                .is_some_and(|next| same_object(&*next, &*self.controller))
            {
                self.view.setNextResponder(self.previous.as_deref());
            }
            let next = self.controller.nextResponder();
            let points_to_previous = match (&next, &self.previous) {
                (None, None) => true,
                (Some(next), Some(previous)) => same_object(&**next, &**previous),
                _ => false,
            };
            if points_to_previous {
                self.controller.setNextResponder(None);
            }
        }

        if let Some(panel) = shared_panel_if_created(mtm) {
            // SAFETY: The responder chain changed without a key-window notification.
            unsafe { panel.updateController() };
        }
    }
}

#[derive(Debug, PartialEq)]
enum FocusTransfer {
    Keep,
    Restore,
    Dismiss,
}

fn focus_transfer(
    panel_key: bool,
    panel_visible: bool,
    owner_key: bool,
    owner_visible: bool,
    app_active: bool,
    any_key: bool,
) -> FocusTransfer {
    if panel_key || owner_key {
        FocusTransfer::Keep
    } else if !panel_visible && owner_visible && app_active && !any_key {
        FocusTransfer::Restore
    } else {
        FocusTransfer::Dismiss
    }
}

fn shared_panel_if_created(mtm: MainThreadMarker) -> Option<Retained<QLPreviewPanel>> {
    // SAFETY: `sharedPreviewPanelExists` is a main-thread-only query.
    unsafe {
        QLPreviewPanel::sharedPreviewPanelExists(mtm)
            .then(|| QLPreviewPanel::sharedPreviewPanel(mtm))
            .flatten()
    }
}

fn current_controller_is(panel: &QLPreviewPanel, controller: &PreviewController) -> bool {
    // SAFETY: The panel's controller is retained by Quick Look while it is current.
    unsafe {
        panel
            .currentController()
            .is_some_and(|current| same_object(&*current, controller))
    }
}

fn same_object<T: ?Sized, U: ?Sized>(object: &T, other: &U) -> bool {
    (std::ptr::from_ref(object) as *const ()) == (std::ptr::from_ref(other) as *const ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_transfer_restores_only_an_uncontested_in_app_close() {
        assert_eq!(
            focus_transfer(false, false, false, true, true, false),
            FocusTransfer::Restore
        );
        assert_eq!(
            focus_transfer(true, true, false, true, true, true),
            FocusTransfer::Keep
        );
        assert_eq!(
            focus_transfer(false, false, true, true, true, true),
            FocusTransfer::Keep
        );
        for (visible, owner_visible, active, any_key) in [
            (true, true, true, false),
            (false, true, false, false),
            (false, true, true, true),
            (false, false, true, false),
        ] {
            assert_eq!(
                focus_transfer(false, visible, false, owner_visible, active, any_key),
                FocusTransfer::Dismiss
            );
        }
    }

    #[test]
    fn adds_quick_look_to_path_results_before_copy_path() {
        let path = Path::new("/tmp/brief.md").to_path_buf();
        let mut item = Item::path(path.clone(), "brief.md".into(), Vec::new());

        add_action(&mut item);

        assert_eq!(
            item.actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            ["open", "reveal", ACTION_ID, "copy-path"]
        );
        assert_eq!(item.action(ACTION_ID), Some(&Action::Preview(path)));
    }

    #[test]
    fn does_not_add_quick_look_when_a_result_cannot_be_opened_as_a_path() {
        let mut item = Item::path(
            Path::new("/tmp/brief.md").to_path_buf(),
            "brief.md".into(),
            Vec::new(),
        );
        item.actions.retain(|action| action.id != "open");

        add_action(&mut item);

        assert!(item.item_action(ACTION_ID).is_none());
    }
}
