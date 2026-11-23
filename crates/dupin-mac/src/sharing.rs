use std::cell::Cell;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use dupin_core::ShareTarget;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSSharingService, NSSharingServiceNameComposeEmail, NSSharingServiceNameSendViaAirDrop,
    NSSharingServicePicker, NSSharingServicePickerDelegate,
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

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = Cell<bool>]
    struct PickerDelegate;

    unsafe impl NSObjectProtocol for PickerDelegate {}

    unsafe impl NSSharingServicePickerDelegate for PickerDelegate {
        #[unsafe(method(sharingServicePicker:didChooseSharingService:))]
        fn did_choose_sharing_service(
            &self,
            _picker: &NSSharingServicePicker,
            _service: Option<&NSSharingService>,
        ) {
            self.ivars().set(false);
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
            .is_some_and(|(_, delegate)| delegate.ivars().get())
    }

    pub fn close(&mut self) {
        if let Some((picker, delegate)) = self.picker.take()
            && delegate.ivars().replace(false)
        {
            picker.close();
        }
    }

    pub fn share(&mut self, target: ShareTarget, paths: &[PathBuf]) -> Result<(), String> {
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
                let view = NSApplication::sharedApplication(main)
                    .keyWindow()
                    .and_then(|window| window.contentView())
                    .ok_or("Couldn't find a window for the share sheet.")?;
                self.close();
                // SAFETY: The delegate's superclass is NSObject and its ivars are set before init.
                let delegate: Retained<PickerDelegate> =
                    unsafe { msg_send![super(PickerDelegate::alloc().set_ivars(Cell::new(true))), init] };
                // SAFETY: File URLs conform to NSPasteboardWriting.
                let picker =
                    unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), items) };
                picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                self.picker = Some((picker, delegate));
                let (picker, _) = self.picker.as_ref().unwrap();
                picker.showRelativeToRect_ofView_preferredEdge(view.bounds(), &view, NSRectEdge::MaxX);
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
