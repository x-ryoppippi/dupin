use std::cell::RefCell;
use std::ffi::{CString, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use dupin_core::ShareTarget;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSSharingService, NSSharingServiceNameComposeEmail, NSSharingServiceNameSendViaAirDrop,
    NSSharingServicePicker, NSSharingServicePickerDelegate, NSView,
};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSRectEdge, NSString, NSURL};

pub fn targets() -> [(&'static str, &'static str, ShareTarget); 3] {
    let service = |name: &NSString, label: &str| ShareTarget::Service {
        name: name.to_string(),
        label: label.into(),
    };
    // SAFETY: AppKit exports these immutable service names.
    unsafe {
        [
            ("share", "Share…", ShareTarget::Picker),
            (
                "airdrop",
                "AirDrop",
                service(NSSharingServiceNameSendViaAirDrop, "AirDrop"),
            ),
            (
                "mail",
                "New Mail with Attachment",
                service(NSSharingServiceNameComposeEmail, "Mail"),
            ),
        ]
    }
}

struct PickerCompletion(RefCell<Option<Box<dyn FnOnce()>>>);

impl PickerCompletion {
    fn is_pending(&self) -> bool {
        self.0.borrow().is_some()
    }

    fn take(&self) -> Option<Box<dyn FnOnce()>> {
        self.0.borrow_mut().take()
    }

    fn finish(&self) {
        if let Some(callback) = self.take() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback));
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PickerCompletion]
    struct PickerDelegate;

    unsafe impl NSObjectProtocol for PickerDelegate {}

    unsafe impl NSSharingServicePickerDelegate for PickerDelegate {
        #[unsafe(method(sharingServicePicker:didChooseSharingService:))]
        fn did_choose_sharing_service(
            &self,
            _picker: &NSSharingServicePicker,
            _service: Option<&NSSharingService>,
        ) {
            self.ivars().finish();
        }
    }
);

#[derive(Default)]
pub struct Picker {
    picker: Option<(Retained<NSSharingServicePicker>, Retained<PickerDelegate>)>,
}

impl Picker {
    pub fn is_active(&self) -> bool {
        self.picker
            .as_ref()
            .is_some_and(|(_, delegate)| delegate.ivars().is_pending())
    }

    pub fn close(&mut self) {
        if let Some((picker, delegate)) = self.picker.take()
            && delegate.ivars().take().is_some()
        {
            picker.close();
        }
    }

    /// # Safety
    /// `ns_view` must point to the launcher's live AppKit view for the duration of the call.
    pub unsafe fn share(
        &mut self,
        target: ShareTarget,
        paths: &[PathBuf],
        ns_view: NonNull<c_void>,
        on_picker_finished: impl FnOnce() + 'static,
    ) -> Result<(), String> {
        let main = MainThreadMarker::new().ok_or("Sharing requires the main thread.")?;
        let urls = paths
            .iter()
            .map(|path| file_url(path))
            .collect::<Result<Vec<_>, _>>()?;
        let urls = NSArray::from_retained_slice(&urls);
        // SAFETY: Erasing the array's element type leaves the immutable file URLs intact.
        let items: &NSArray = unsafe { urls.cast_unchecked() };
        match target {
            ShareTarget::Picker => {
                // SAFETY: The caller lends the launcher view while Rune keeps it alive.
                let view = unsafe { ns_view.cast::<NSView>().as_ref() };
                self.close();
                let completion = PickerCompletion(RefCell::new(Some(Box::new(on_picker_finished))));
                // SAFETY: The delegate's superclass is NSObject and its ivars are set before init.
                let delegate: Retained<PickerDelegate> =
                    unsafe { msg_send![super(PickerDelegate::alloc(main).set_ivars(completion)), init] };
                // SAFETY: File URLs conform to NSPasteboardWriting.
                let picker =
                    unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), items) };
                picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                self.picker = Some((picker, delegate));
                let (picker, _) = self.picker.as_ref().unwrap();
                picker.showRelativeToRect_ofView_preferredEdge(view.bounds(), view, NSRectEdge::MaxX);
            }
            ShareTarget::Service { name, label } => {
                let service = NSSharingService::sharingServiceNamed(&NSString::from_str(&name))
                    .ok_or_else(|| format!("{label} is unavailable."))?;
                // SAFETY: File URLs conform to NSPasteboardWriting.
                unsafe {
                    if !service.canPerformWithItems(Some(items)) {
                        return Err(format!("{label} can't share the selected items."));
                    }
                    service.performWithItems(items);
                }
            }
        }
        Ok(())
    }
}

fn file_url(path: &Path) -> Result<Retained<NSURL>, String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("Couldn't share “{}”: {error}", path.display()))?;
    let bytes = CString::new(path.as_os_str().as_bytes()).map_err(|_| "Invalid file path.")?;
    // SAFETY: CString keeps the original filesystem bytes NUL-terminated for this call.
    Ok(unsafe {
        NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
            NonNull::from(bytes.as_c_str()).cast(),
            metadata.is_dir(),
            None,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn picker_completion_rechecks_focus_once_after_releasing_it() {
        let calls = Rc::new(Cell::new(0));
        let completion = Rc::new(PickerCompletion(RefCell::new(None)));
        let pending = Rc::downgrade(&completion);
        *completion.0.borrow_mut() = Some(Box::new({
            let calls = calls.clone();
            move || {
                assert!(!pending.upgrade().unwrap().is_pending());
                calls.set(calls.get() + 1);
            }
        }));

        assert!(completion.is_pending());
        completion.finish();
        completion.finish();
        assert!(!completion.is_pending());
        assert_eq!(calls.get(), 1);
    }
}
