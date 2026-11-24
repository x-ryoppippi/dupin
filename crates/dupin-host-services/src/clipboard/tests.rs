use super::*;

fn text(text: &str) -> Content {
    Content::Text { text: text.into() }
}

fn clipboard(access: OsAccess) -> (Arc<Fixture>, Clipboard<Arc<Fixture>>) {
    let board = Arc::new(Fixture::new(access));
    let policy = Policy {
        ignored_apps: vec!["com.example.passwords".into()],
        ..Policy::default()
    };
    (board.clone(), Clipboard::new(board, policy))
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
    let (board, clipboard) = clipboard(OsAccess::Allowed);
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

    clipboard.set_policy(Policy::default());
    assert!(clipboard.read(Trigger::Activation).unwrap().is_some());
}

#[test]
fn copies_beyond_the_policy_limit_are_left_out() {
    let (board, clipboard) = clipboard(OsAccess::Allowed);
    clipboard.set_policy(Policy {
        ignored_apps: Vec::new(),
        max_text_bytes: 4,
    });
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

    let files = Write::files(vec!["/Users/me/a.txt".into()]).unwrap();
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
    for path in ["relative.txt", "/a/../b", "/a\0b"] {
        assert!(Write::files(vec![path.into()]).is_err(), "{path:?}");
    }
    assert!(Write::files(Vec::new()).is_err());
    assert!(Write::files(vec!["/a".into(); MAX_FILES + 1]).is_err());
}

#[test]
fn a_watch_delivers_each_readable_copy_until_dropped() {
    let (board, clipboard) = clipboard(OsAccess::Allowed);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let watch = clipboard
        .watch({
            let seen = seen.clone();
            move |clip| seen.lock().unwrap().push(clip)
        })
        .unwrap();
    board.copy(text("one"), &[], None, None);
    board.copy(text("secret"), &["org.nspasteboard.ConcealedType"], None, None);
    board.copy(text("secret"), &[], Some("com.example.passwords"), None);
    board.copy(
        Content::Files {
            files: vec![FileRef::new(Path::new("/Users/me/a.txt"))],
        },
        &[],
        None,
        None,
    );
    board.set_access(OsAccess::Denied);
    board.copy(text("after denial"), &[], None, None);
    board.set_access(OsAccess::Allowed);
    drop(watch);
    assert_eq!(board.watches(), 0);
    board.copy(text("after drop"), &[], None, None);

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.iter().map(|clip| clip.content.clone()).collect::<Vec<_>>(),
        [
            text("one"),
            Content::Files {
                files: vec![FileRef {
                    path: "/Users/me/a.txt".into(),
                    name: "a.txt".into()
                }]
            },
        ]
    );
    assert_eq!(seen[1].change, 4);
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
