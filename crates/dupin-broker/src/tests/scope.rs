use super::*;

#[test]
fn request_urls_reduce_to_a_canonical_origin() {
    for (url, expected) in [
        ("https://api.github.com/repos/x", "https://api.github.com"),
        ("HTTPS://API.GitHub.com:443/x?y#z", "https://api.github.com"),
        ("https://api.github.com:8443", "https://api.github.com:8443"),
        ("https://api.github.com?q", "https://api.github.com"),
    ] {
        assert_eq!(Origin::of_url(url).unwrap(), origin(expected), "{url}");
    }
    for url in [
        "http://api.github.com",
        "https://user:pass@api.github.com/",
        "https://api.github.com:99999",
        "https://api github.com",
        "ftp://api.github.com",
        "https",
    ] {
        assert!(Origin::of_url(url).is_err(), "{url}");
    }
    assert_eq!(origin("https://a.example").port(), 443);
    assert_eq!(origin("https://a.example:8443").host(), "a.example");
}

#[test]
fn scope_keys_round_trip_through_their_written_form() {
    for written in [
        "http:https://api.github.com",
        "http:https://a.example:8443",
        "files:notes",
        "exec:git-status",
        "script:open-in-zed",
        "storage",
        "storage:sync",
        "clipboard:read",
        "clipboard:write",
        "clipboard:watch",
        "clipboard:paste",
        "dictionary",
        "calculator",
        "snippets",
        "calendar",
        "windows:read",
        "windows:arrange",
        "context:frontmost",
        "context:document",
        "apps:read",
        "apps:control",
    ] {
        assert_eq!(key(written).to_string(), written);
    }
    for bad in [
        "http:http://a.example",
        "http:https://a.example/",
        "files:",
        "clipboard:type",
        "windows:move",
        "context",
        "context:selection",
        "context:Frontmost",
        "apps",
        "apps:quit",
        "windows",
        "network",
        "storage:x",
    ] {
        assert!(bad.parse::<ScopeKey>().is_err(), "{bad}");
    }
}

#[test]
fn identifiers_are_validated() {
    assert!(PackageId::new("github:x-ryoppippi/dupin-bookmarks").is_ok());
    assert!(PackageId::new("local:/Users/me/My Extension").is_ok());
    assert!(PackageId::new("").is_err());
    assert!(PackageId::new("a\nb").is_err());
    assert!(Revision::new("sha256:0123abcdef").is_ok());
    assert!(Revision::new("sha256 0123").is_err());
    assert!(Revision::new("x".repeat(129)).is_err());
    assert!(ScopeName::new("open-in-zed.v2").is_ok());
    assert!(ScopeName::new(".hidden").is_err());
    assert!(ScopeName::new("a/b").is_err());
}

#[test]
fn an_approval_covers_the_same_or_a_narrower_declaration_only() {
    let get = http("https://a.example", &[Method::Get]);
    let get_post = http("https://a.example", &[Method::Get, Method::Post]);
    assert!(get_post.covers(&get));
    assert!(get_post.covers(&get_post));
    assert!(!get.covers(&get_post));
    assert!(!get.covers(&http("https://b.example", &[Method::Get])));

    let files = |kind, access: &[FilesAccess]| Scope::Files {
        name: name("notes"),
        kind,
        access: access.iter().copied().collect(),
        path: None,
    };
    assert!(
        files(FilesKind::Directory, &[FilesAccess::Read, FilesAccess::List])
            .covers(&files(FilesKind::Directory, &[FilesAccess::Read]))
    );
    assert!(
        !files(FilesKind::Directory, &[FilesAccess::Read])
            .covers(&files(FilesKind::Directory, &[FilesAccess::List]))
    );
    assert!(
        !files(FilesKind::File, &[FilesAccess::Read])
            .covers(&files(FilesKind::Directory, &[FilesAccess::Read]))
    );

    let exec = |args: &[&str], output, ssh_agent| Scope::Exec {
        name: name("git"),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        output,
        program: None,
        background: false,
        ssh_agent,
        output_bytes: None,
    };
    assert!(exec(&["status"], true, false).covers(&exec(&["status"], false, false)));
    assert!(!exec(&["status"], false, false).covers(&exec(&["status"], true, false)));
    assert!(!exec(&["status"], true, false).covers(&exec(&["push"], true, false)));
    assert!(exec(&["status"], false, true).covers(&exec(&["status"], false, false)));
    assert!(
        !exec(&["status"], false, false).covers(&exec(&["status"], false, true)),
        "handing over the SSH agent widens the profile"
    );
    let bounded = |output_bytes| Scope::Exec {
        name: name("ps"),
        args: vec!["-ax".into()],
        output: true,
        program: None,
        background: false,
        ssh_agent: false,
        output_bytes,
    };
    assert!(bounded(Some(1 << 20)).covers(&bounded(None)));
    assert!(bounded(None).covers(&bounded(Some(1024))));
    assert!(
        !bounded(None).covers(&bounded(Some(1 << 20))),
        "reading more output asks again"
    );

    let script = |output| Scope::Script {
        name: name("zed"),
        output,
    };
    assert!(script(true).covers(&script(false)));
    assert!(!script(false).covers(&script(true)));

    let storage = |quota_bytes, retention_days| Scope::Storage {
        quota_bytes,
        retention_days,
    };
    assert!(storage(10, None).covers(&storage(5, Some(30))));
    assert!(storage(10, Some(30)).covers(&storage(10, Some(7))));
    assert!(!storage(10, Some(30)).covers(&storage(10, None)));
    assert!(!storage(10, Some(30)).covers(&storage(11, Some(30))));

    let calendar = |past_days, future_days| Scope::Calendar {
        past_days,
        future_days,
    };
    assert!(calendar(7, 30).covers(&calendar(0, 30)));
    assert!(!calendar(7, 30).covers(&calendar(8, 30)));
    assert!(!calendar(7, 30).covers(&calendar(7, 31)));

    let clipboard = |access| Scope::Clipboard { access };
    assert!(clipboard(ClipboardAccess::Read).covers(&clipboard(ClipboardAccess::Read)));
    assert!(!clipboard(ClipboardAccess::Write).covers(&clipboard(ClipboardAccess::Read)));
    assert!(Scope::Dictionary.covers(&Scope::Dictionary));
    assert!(!Scope::Dictionary.covers(&storage(1, None)));
}

#[test]
fn only_calendar_and_clipboard_reads_need_an_os_permission() {
    let permissions: Vec<_> = everything()
        .scopes()
        .filter_map(|scope| Some((scope.key().to_string(), scope.os_permission()?)))
        .collect();
    assert_eq!(
        permissions,
        [
            ("clipboard:read".to_string(), OsPermission::Pasteboard),
            ("clipboard:watch".to_string(), OsPermission::Pasteboard),
            ("calendar".to_string(), OsPermission::Calendars),
        ]
    );
}

#[test]
fn windows_scopes_need_accessibility_and_describe_themselves() {
    let windows = caps(json!({ "windows": ["read", "arrange"] }));
    let described: Vec<_> = windows
        .scopes()
        .map(|scope| (scope.key().to_string(), scope.to_string(), scope.os_permission()))
        .collect();
    assert_eq!(
        described,
        [
            (
                "windows:read".to_string(),
                "see your displays and the focused window".to_string(),
                Some(OsPermission::Accessibility)
            ),
            (
                "windows:arrange".to_string(),
                "move and resize the focused window".to_string(),
                Some(OsPermission::Accessibility)
            ),
        ]
    );
    let scope = |access| Scope::Windows { access };
    assert!(scope(WindowsAccess::Read).covers(&scope(WindowsAccess::Read)));
    assert!(!scope(WindowsAccess::Arrange).covers(&scope(WindowsAccess::Read)));
}

#[test]
fn apps_scopes_need_no_macos_access_and_only_control_needs_an_activation() {
    let apps = caps(json!({ "apps": ["read", "control"] }));
    let described: Vec<_> = apps
        .scopes()
        .map(|scope| (scope.key().to_string(), scope.to_string(), scope.os_permission()))
        .collect();
    assert_eq!(
        described,
        [
            (
                "apps:read".to_string(),
                "see which apps are running".to_string(),
                None
            ),
            (
                "apps:control".to_string(),
                "hide, quit and force quit running apps".to_string(),
                None
            ),
        ]
    );
    let scope = |access| Scope::Apps { access };
    assert!(
        !scope(AppsAccess::Read).covers(&scope(AppsAccess::Control)),
        "seeing the apps never lets a package quit one"
    );
    assert!(!Request::Apps(AppsAccess::Read).needs_activation(&scope(AppsAccess::Read)));
    assert!(Request::Apps(AppsAccess::Control).needs_activation(&scope(AppsAccess::Control)));
}

#[test]
fn context_scopes_need_no_macos_access_or_activation_and_describe_themselves() {
    let context = caps(json!({ "context": ["frontmost", "document"] }));
    let described: Vec<_> = context
        .scopes()
        .map(|scope| (scope.key().to_string(), scope.to_string(), scope.os_permission()))
        .collect();
    assert_eq!(
        described,
        [
            (
                "context:frontmost".to_string(),
                "see which app you were using when Dupin opened".to_string(),
                None
            ),
            (
                "context:document".to_string(),
                "see which file you were viewing when Dupin opened".to_string(),
                None
            ),
        ]
    );
    let scope = |access| Scope::Context { access };
    assert!(scope(ContextAccess::Frontmost).covers(&scope(ContextAccess::Frontmost)));
    assert!(
        !scope(ContextAccess::Frontmost).covers(&scope(ContextAccess::Document)),
        "seeing the app never lets a package see the file"
    );
    for access in [ContextAccess::Frontmost, ContextAccess::Document] {
        assert!(!Request::Context(access).needs_activation(&scope(access)));
    }
}

#[test]
fn exec_consent_shows_where_each_argument_starts_and_ends() {
    let exec = |args: &[&str]| {
        Scope::Exec {
            name: ScopeName::new("run").unwrap(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            output: false,
            program: None,
            background: false,
            ssh_agent: false,
            output_bytes: None,
        }
        .to_string()
    };
    assert_ne!(exec(&["a b"]), exec(&["a", "b"]));
    assert_eq!(
        exec(&["a`, and nothing else", "--install-extension", "x"]),
        "run the program you pick for “run” with \"a`, and nothing else\" \"--install-extension\" \"x\""
    );
    let agent = Scope::Exec {
        name: ScopeName::new("ssh").unwrap(),
        args: vec!["{host}".into()],
        output: true,
        program: None,
        background: false,
        ssh_agent: true,
        output_bytes: Some(1_048_576),
    };
    assert_eq!(
        agent.to_string(),
        "run the program you pick for “ssh” with \"{host}\", using the keys your SSH agent holds, and read up to 1048576 bytes of its output"
    );
}

#[test]
fn scopes_describe_themselves_for_consent() {
    let described: Vec<_> = everything().scopes().map(ToString::to_string).collect();
    assert_eq!(
        described,
        [
            "GET and POST requests to https://api.github.com",
            "read and list the folder you pick for “notes”",
            "run the program you pick for “git-status” with \"status\" \"--short\" \"{path}\" and read its output",
            "run your script command “open-in-zed”",
            "keep up to 1024 bytes of encrypted data for 30 days",
            "read the clipboard",
            "write to the clipboard",
            "see each new clipboard entry",
            "look words up in the system dictionaries",
            "read calendar events from 1 day back to 30 days ahead",
        ]
    );
}
