use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use super::*;
use crate::files::{Scope, ScopeKind, Target};

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(100);

fn text(text: &str) -> Content {
    Content::Text { text: text.into() }
}

fn clipboard(access: OsAccess) -> (Arc<Fixture>, Clipboard<Arc<Fixture>>) {
    let (board, clipboard, _) = following(access);
    (board, clipboard)
}

/// A clipboard that follows the policy the returned cell holds, as Dupin's follows clipboard
/// history's.
fn following(access: OsAccess) -> (Arc<Fixture>, Clipboard<Arc<Fixture>>, Arc<Mutex<Policy>>) {
    let board = Arc::new(Fixture::new(access));
    let policy = Arc::new(Mutex::new(Policy {
        ignored_apps: vec!["com.example.passwords".into()],
        ..Policy::default()
    }));
    let source = policy.clone();
    let clipboard = Clipboard::new(board.clone(), move || source.lock().unwrap().clone());
    (board, clipboard, policy)
}

#[test]
fn reading_returns_the_current_copy_after_an_activation() {
    let (board, clipboard) = clipboard(OsAccess::Allowed);
    assert_eq!(clipboard.read(Trigger::Activation), Ok(None));
    board.copy(
        text("hello"),
        &["public.utf8-plain-text"],
        Some("com.apple.Safari"),
        None,
    );
    assert_eq!(
        clipboard.read(Trigger::Activation),
        Ok(Some(Clip {
            change: 1,
            content: text("hello")
        }))
    );
    for trigger in [Trigger::Query, Trigger::Background] {
        assert_eq!(clipboard.read(trigger), Err(ServiceError::NeedsActivation));
    }
}

#[test]
fn concealed_transient_and_generated_copies_are_never_read() {
    let (board, clipboard) = clipboard(OsAccess::Allowed);
    for kind in WITHHELD_TYPES {
        board.copy(text("secret"), &["public.utf8-plain-text", kind], None, None);
        assert_eq!(clipboard.read(Trigger::Activation), Ok(None), "{kind}");
    }
    assert_eq!(board.reads(), 0);
    assert!(!withheld(&["public.utf8-plain-text".into()]));
}

#[test]
fn copies_from_ignored_apps_are_never_read() {
    let (board, clipboard, policy) = following(OsAccess::Allowed);
    board.copy(text("secret"), &[], Some("com.example.passwords"), None);
    assert_eq!(clipboard.read(Trigger::Activation), Ok(None));
    board.copy(
        text("secret"),
        &[],
        Some("com.apple.Terminal"),
        Some("com.example.passwords"),
    );
    assert_eq!(clipboard.read(Trigger::Activation), Ok(None));
    assert_eq!(board.reads(), 0);

    *policy.lock().unwrap() = Policy::default();
    assert!(clipboard.read(Trigger::Activation).unwrap().is_some());
}

#[test]
fn copies_from_an_unknown_app_are_never_read_while_any_app_is_ignored() {
    let (board, clipboard, policy) = following(OsAccess::Allowed);
    board.copy(text("secret"), &[], Some("com.example.passwords"), None);
    board.forget_apps();
    assert_eq!(clipboard.read(Trigger::Activation), Ok(None));
    assert_eq!(board.reads(), 0);

    *policy.lock().unwrap() = Policy {
        ignored_apps: Vec::new(),
        ..Policy::default()
    };
    assert!(clipboard.read(Trigger::Activation).unwrap().is_some());
}

#[test]
fn copies_beyond_the_policy_limit_are_left_out() {
    let (board, clipboard, policy) = following(OsAccess::Allowed);
    *policy.lock().unwrap() = Policy {
        ignored_apps: Vec::new(),
        max_text_bytes: 4,
    };
    board.copy(text("12345"), &[], None, None);
    assert_eq!(clipboard.read(Trigger::Activation), Ok(None));
    board.copy(text("1234"), &[], None, None);
    assert!(clipboard.read(Trigger::Activation).unwrap().is_some());
}

#[test]
fn reading_and_watching_need_the_pasteboard_policy() {
    for (access, expected) in [
        (
            OsAccess::NotDetermined,
            ServiceError::NeedsPermission(OsPermission::Pasteboard),
        ),
        (
            OsAccess::Denied,
            ServiceError::PermissionDenied(OsPermission::Pasteboard),
        ),
        (
            OsAccess::Restricted,
            ServiceError::PermissionRestricted(OsPermission::Pasteboard),
        ),
    ] {
        let (board, clipboard) = clipboard(access);
        board.copy(text("hello"), &[], None, None);
        assert_eq!(clipboard.read(Trigger::Activation), Err(expected.clone()));
        assert_eq!(clipboard.watch(|_| {}).err(), Some(expected));
        assert_eq!(board.reads(), 0);
        assert_eq!(board.watches(), 0);
    }
}

#[test]
fn writing_needs_an_activation_but_no_os_permission() {
    let (board, clipboard) = clipboard(OsAccess::Denied);
    let url = Write::url("https://example.com/a?b#c".into()).unwrap();
    assert_eq!(
        clipboard.write(Trigger::Query, &url),
        Err(ServiceError::NeedsActivation)
    );
    assert_eq!(
        clipboard.write(Trigger::Background, &url),
        Err(ServiceError::NeedsActivation)
    );
    assert!(board.written().is_empty());

    let (_folder, files, handle) = scoped_file();
    let files = Write::files(&files, &[handle]).unwrap();
    for content in [Write::text("hi".into()).unwrap(), url, files] {
        clipboard.write(Trigger::Activation, &content).unwrap();
    }
    assert_eq!(board.written().len(), 3);
}

#[test]
fn invalid_writes_are_refused() {
    assert!(Write::text(String::new()).is_err());
    assert!(Write::text("x".repeat(MAX_TEXT_BYTES + 1)).is_err());
    for url in [
        "",
        "example.com",
        "1http://x",
        "https://a b",
        "https://a\nb",
        "ht tp://x",
    ] {
        assert!(Write::url(url.into()).is_err(), "{url:?}");
    }
    assert!(Write::url(format!("https://{}", "a".repeat(MAX_URL_BYTES))).is_err());
    assert!(Write::url("mailto:me@example.com".into()).is_ok());
    let (_folder, files, handle) = scoped_file();
    assert!(matches!(Write::files(&files, &[]), Err(ServiceError::Invalid(_))));
    assert!(matches!(
        Write::files(&files, &[handle; MAX_FILES + 1]),
        Err(ServiceError::Invalid(_))
    ));
}

/// A folder granted as the `notes` scope, and a handle to `a.txt` inside it.
fn scoped_file() -> (tempfile::TempDir, Files, Handle) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("a.txt"), "a").unwrap();
    let mut files = Files::new();
    files.grant(Scope::pick("notes", ScopeKind::Directory, folder.path()).unwrap());
    let handle = files
        .read(
            Target::Path {
                scope: "notes",
                path: "a.txt",
            },
            1,
        )
        .unwrap()
        .handle;
    (folder, files, handle)
}

#[test]
fn only_files_inside_a_granted_scope_can_be_written() {
    let (folder, files, handle) = scoped_file();
    let Write::Files(written) = Write::files(&files, &[handle]).unwrap() else {
        unreachable!();
    };
    assert_eq!(
        written.paths(),
        [folder.path().canonicalize().unwrap().join("a.txt")]
    );

    let (_other, _elsewhere, foreign) = scoped_file();
    assert!(matches!(
        Write::files(&files, &[foreign]),
        Err(ServiceError::OutOfScope(_))
    ));

    let mut revoked = files;
    revoked.revoke("notes");
    assert!(matches!(
        Write::files(&revoked, &[handle]),
        Err(ServiceError::OutOfScope(_))
    ));
}

fn watching(clipboard: &Clipboard<Arc<Fixture>>) -> (Watch, mpsc::Receiver<Clip>) {
    let (sender, clips) = mpsc::channel();
    let watch = clipboard
        .watch(move |clip| {
            let _ = sender.send(clip.clone());
        })
        .unwrap();
    (watch, clips)
}

#[test]
fn a_watch_delivers_each_readable_copy_until_dropped() {
    let (board, clipboard) = clipboard(OsAccess::Allowed);
    let (watch, clips) = watching(&clipboard);
    board.copy(text("one"), &[], None, None);
    assert_eq!(clips.recv_timeout(WAIT).unwrap().content, text("one"));

    board.copy(text("secret"), &["org.nspasteboard.ConcealedType"], None, None);
    board.copy(text("secret"), &[], Some("com.example.passwords"), None);
    let file = Content::Files {
        files: vec![FileRef::new(Path::new("/Users/me/a.txt"))],
    };
    board.copy(file.clone(), &[], None, None);
    let clip = clips.recv_timeout(WAIT).unwrap();
    assert_eq!((clip.change, clip.content), (4, file));

    board.set_access(OsAccess::Denied);
    board.copy(text("after denial"), &[], None, None);
    board.set_access(OsAccess::Allowed);
    drop(watch);
    assert_eq!(board.watches(), 0);
    board.copy(text("after drop"), &[], None, None);
    assert!(matches!(
        clips.recv_timeout(QUIET),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn a_watch_follows_apps_ignored_after_it_was_subscribed() {
    let (board, clipboard, policy) = following(OsAccess::Allowed);
    let (_watch, clips) = watching(&clipboard);
    board.copy(text("before"), &[], Some("com.example.bank"), None);
    assert_eq!(clips.recv_timeout(WAIT).unwrap().content, text("before"));

    policy
        .lock()
        .unwrap()
        .ignored_apps
        .push("com.example.bank".into());
    board.copy(text("account"), &[], Some("com.example.bank"), None);
    board.copy(text("after"), &[], Some("com.apple.Safari"), None);
    assert_eq!(clips.recv_timeout(WAIT).unwrap().content, text("after"));
}

#[test]
fn watches_share_one_read_of_each_copy_and_are_capped() {
    let (board, clipboard) = clipboard(OsAccess::Allowed);
    let watches: Vec<_> = (0..MAX_WATCHES).map(|_| watching(&clipboard)).collect();
    assert!(matches!(
        clipboard.watch(|_| {}).err(),
        Some(ServiceError::Invalid(_))
    ));
    assert_eq!(board.watches(), 1);

    board.copy(text("one"), &[], None, None);
    for (_, clips) in &watches {
        assert_eq!(clips.recv_timeout(WAIT).unwrap().content, text("one"));
    }
    assert_eq!(board.reads(), 1);

    let mut watches = watches;
    watches.pop();
    let (_extra, extra) = watching(&clipboard);
    board.copy(text("two"), &[], None, None);
    assert_eq!(extra.recv_timeout(WAIT).unwrap().content, text("two"));
    drop(watches);
    drop(_extra);
    assert_eq!(board.watches(), 0);
}

#[test]
fn a_stalled_watch_holds_up_neither_the_monitor_nor_other_watches() {
    let (board, clipboard) = clipboard(OsAccess::Allowed);
    let (release, stalled) = mpsc::channel::<()>();
    let _stuck = clipboard
        .watch(move |_| {
            let _ = stalled.recv();
        })
        .unwrap();
    let (_watch, clips) = watching(&clipboard);
    for n in 0..WATCH_BACKLOG * 4 {
        let copy = text(&format!("copy {n}"));
        board.copy(copy.clone(), &[], None, None);
        assert_eq!(clips.recv_timeout(WAIT).unwrap().content, copy);
    }
    drop(release);
}

#[test]
fn clips_serialize_for_javascript() {
    let image = Clip {
        change: 7,
        content: Content::Image {
            media_type: "image/png".into(),
            data: b"\x89PNG".to_vec(),
        },
    };
    assert_eq!(
        serde_json::to_value(image).unwrap(),
        serde_json::json!({
            "change": 7,
            "content": { "type": "image", "mediaType": "image/png", "data": "iVBORw==" },
        })
    );
    assert_eq!(
        serde_json::to_value(Content::Url {
            url: "https://example.com".into()
        })
        .unwrap(),
        serde_json::json!({ "type": "url", "url": "https://example.com" })
    );
}
