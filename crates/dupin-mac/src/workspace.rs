//! Running actions through NSWorkspace.

use std::path::{Path, PathBuf};

use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSFileManager, NSString, NSURL};

#[derive(Debug)]
pub enum Action {
    Open(PathBuf),
    OpenApp {
        bundle_id: String,
        hide_if_frontmost: bool,
    },
    Reveal(PathBuf),
    OpenUrl(String),
    OpenWith {
        path: PathBuf,
        application: PathBuf,
    },
    Trash(PathBuf),
}

pub fn run(action: &Action) -> Result<(), String> {
    autoreleasepool(|_| match action {
        Action::Open(path) => open(path)
            .then_some(())
            .ok_or_else(|| format!("Couldn't open “{}”.", path.display())),
        Action::OpenApp {
            bundle_id,
            hide_if_frontmost,
        } => open_app(bundle_id, *hide_if_frontmost),
        Action::OpenUrl(url) => open_url(url)
            .then_some(())
            .ok_or_else(|| "Couldn't open this link.".into()),
        Action::Reveal(path) => {
            if !path.exists() {
                return Err(format!("“{}” no longer exists.", path.display()));
            }
            reveal(path);
            Ok(())
        }
        Action::OpenWith { path, application } => {
            let (sender, receiver) = std::sync::mpsc::channel();
            let completion = block2::RcBlock::new(
                move |app: *mut objc2_app_kit::NSRunningApplication,
                      error: *mut objc2_foundation::NSError| {
                    let result = if let Some(error) = unsafe { error.as_ref() } {
                        Err(error.localizedDescription().to_string())
                    } else if app.is_null() {
                        Err("The application did not open.".into())
                    } else {
                        Ok(())
                    };
                    let _ = sender.send(result);
                },
            );
            let configuration = NSWorkspaceOpenConfiguration::configuration();
            configuration.setActivates(true);
            NSWorkspace::sharedWorkspace().openURLs_withApplicationAtURL_configuration_completionHandler(
                &NSArray::from_retained_slice(&[file_url(path)]),
                &file_url(application),
                &configuration,
                Some(&completion),
            );
            receiver
                .recv()
                .map_err(|_| "The application did not report its result.".to_owned())?
                .map_err(|error| {
                    format!(
                        "Couldn't open “{}” with “{}”: {error}",
                        path.display(),
                        application.display()
                    )
                })
        }
        Action::Trash(path) => NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&file_url(path), None)
            .map_err(|error| error.localizedDescription().to_string()),
    })
}

fn open_app(bundle_id: &str, hide_if_frontmost: bool) -> Result<(), String> {
    let workspace = NSWorkspace::sharedWorkspace();
    let identifier = NSString::from_str(bundle_id);
    if hide_if_frontmost
        && let Some(app) = workspace.frontmostApplication()
        && let Some(frontmost) = app.bundleIdentifier()
        && frontmost.caseInsensitiveCompare(&identifier) == objc2_foundation::NSComparisonResult::Same
    {
        return app
            .hide()
            .then_some(())
            .ok_or_else(|| format!("Couldn't hide “{bundle_id}”."));
    }
    let url = workspace
        .URLForApplicationWithBundleIdentifier(&identifier)
        .ok_or_else(|| format!("Couldn't find the app “{bundle_id}”."))?;
    workspace
        .openURL(&url)
        .then_some(())
        .ok_or_else(|| format!("Couldn't open “{bundle_id}”."))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub path: PathBuf,
    pub name: String,
    pub default: bool,
}

pub fn applications_for(path: &Path) -> Result<Vec<Application>, String> {
    if !path.exists() {
        return Err("This item no longer exists.".into());
    }
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let url = file_url(path);
        let default = workspace
            .URLForApplicationToOpenURL(&url)
            .and_then(|url| url.path())
            .map(|path| PathBuf::from(path.to_string()));
        let mut seen = std::collections::HashSet::new();
        let mut apps: Vec<_> = workspace
            .URLsForApplicationsToOpenURL(&url)
            .iter()
            .filter_map(|url| {
                let path = PathBuf::from(url.path()?.to_string());
                if !seen.insert(path.clone()) {
                    return None;
                }
                let display = NSFileManager::defaultManager()
                    .displayNameAtPath(&NSString::from_str(&path.to_string_lossy()))
                    .to_string();
                Some(Application {
                    default: default.as_ref() == Some(&path),
                    path,
                    name: display.strip_suffix(".app").unwrap_or(&display).to_owned(),
                })
            })
            .collect();
        apps.sort_by_key(|app| (!app.default, app.name.to_lowercase(), app.path.clone()));
        Ok(apps)
    })
}

/// Open a URL with its registered handler.
pub(crate) fn open_url(value: &str) -> bool {
    let Some(url) = NSURL::URLWithString(&NSString::from_str(value)) else {
        return false;
    };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

/// Open with the default handler: launches an app bundle, opens a document or folder.
pub fn open(path: &Path) -> bool {
    NSWorkspace::sharedWorkspace().openURL(&file_url(path))
}

/// Show in Finder with the item selected.
fn reveal(path: &Path) {
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[file_url(path)]));
}

fn file_url(path: &Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}
