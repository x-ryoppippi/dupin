use std::path::PathBuf;

use super::*;

#[test]
fn local_choices_survive_a_relaunch() {
    let package = id("bookmarks");
    let mut broker = Broker::new(Base::default(), Local::default());
    let _ = broker.install(package.clone(), rev("sha256:1111"), everything());
    broker.set_enabled(&package, true).unwrap();
    broker
        .approve(
            &package,
            &key("files:notes"),
            Some(PathBuf::from("/Users/me/Notes")),
        )
        .unwrap();
    broker
        .approve(&package, &key("http:https://api.github.com"), None)
        .unwrap();
    broker.revoke(&package, &ScopeKey::Dictionary).unwrap();

    let mut store = MemoryLocal::default();
    broker.save(&mut store, &Local::default()).unwrap();
    assert_eq!(store.records().len(), 1);

    let mut relaunched = Broker::new(Base::default(), Local::load(&mut store).unwrap());
    let report = relaunched.install(package.clone(), rev("sha256:1111"), everything());
    assert_eq!(report.kept.len(), 2);
    assert!(relaunched.is_enabled(&package));
    assert_eq!(relaunched.consents(&package), broker.consents(&package));
    assert_eq!(
        relaunched.decisions().records().unwrap(),
        broker.decisions().records().unwrap()
    );
}

#[test]
fn a_record_holds_the_approved_scope_and_revision() {
    let package = id("bookmarks");
    let mut broker = Broker::new(Base::default(), Local::default());
    let _ = broker.install(package.clone(), rev("sha256:1111"), everything());
    broker
        .approve(&package, &key("http:https://api.github.com"), None)
        .unwrap();
    broker.revoke(&package, &ScopeKey::Dictionary).unwrap();
    let [record] = broker.decisions().records().unwrap().try_into().unwrap();
    assert_eq!(
        record,
        json!({
            "package": "bookmarks",
            "scopes": {
                "dictionary": { "decision": "revoked" },
                "http:https://api.github.com": {
                    "decision": "granted",
                    "revision": "sha256:1111",
                    "approved": {
                        "type": "http",
                        "origin": "https://api.github.com",
                        "methods": ["GET", "POST"]
                    }
                }
            }
        })
    );
    assert_eq!(Local::record_package(&record), Some(package));
}

#[test]
fn only_the_packages_that_changed_are_written_and_one_left_with_no_decisions_is_deleted() {
    let (bookmarks, notes) = (id("bookmarks"), id("notes"));
    let mut broker = Broker::new(Base::default(), Local::default());
    let _ = broker.install(bookmarks.clone(), rev("sha256:1111"), everything());
    let _ = broker.install(notes.clone(), rev("sha256:1111"), everything());
    let mut store = MemoryLocal::default();
    broker
        .change_saved(&mut store, |broker| {
            broker.approve(&bookmarks, &ScopeKey::Dictionary, None)?;
            broker.set_enabled(&notes, true)
        })
        .unwrap();
    assert_eq!(store.records().len(), 2);

    let forged = json!({ "package": "notes", "enabled": false });
    store.insert(notes.clone(), forged.clone());
    broker
        .change_saved(&mut store, |broker| {
            broker.revoke(&bookmarks, &ScopeKey::Calendar)
        })
        .unwrap();
    assert_eq!(
        store.records()[&notes],
        forged,
        "a package whose decisions didn't change isn't written"
    );

    let before = broker.decisions();
    assert!(broker.uninstall(&bookmarks));
    broker.save(&mut store, &before).unwrap();
    assert!(
        !store.records().contains_key(&bookmarks),
        "a package left with no decisions has no record"
    );

    let before = broker.decisions();
    let generation = broker.generation(&notes);
    broker.forget(&notes).unwrap();
    broker.save(&mut store, &before).unwrap();
    assert!(store.records().is_empty());
    assert!(!broker.is_enabled(&notes));
    assert_ne!(broker.generation(&notes), generation, "in-flight work is dropped");
}

#[test]
fn saved_records_are_validated() {
    let granted = |key: &str, approved: serde_json::Value, binding: Option<&str>| {
        let mut decision = json!({ "decision": "granted", "revision": "r", "approved": approved });
        if let Some(binding) = binding {
            decision["binding"] = binding.into();
        }
        json!({ "package": "a", "scopes": { key: decision } })
    };
    let notes = json!({ "type": "files", "name": "notes", "kind": "file", "access": ["read"] });
    for (record, problem) in [
        (json!({ "package": "a", "extra": true }), "unknown field"),
        (json!({ "scopes": {} }), "missing field"),
        (
            granted("dictionary", json!({ "type": "storage", "quota_bytes": 1 }), None),
            "records another scope",
        ),
        (granted("files:notes", notes.clone(), None), "wrong binding"),
        (granted("files:notes", notes, Some("Notes")), "absolute path"),
        (
            json!({ "package": "a", "scopes": { "network": { "decision": "revoked" } } }),
            "isn't a scope",
        ),
    ] {
        let error = Local::from_records(vec![record.clone()]).unwrap_err().to_string();
        assert!(error.contains(problem), "{record}: {error}");
    }
    let twice = json!({ "package": "a", "enabled": true });
    assert!(
        Local::from_records(vec![twice.clone(), twice])
            .unwrap_err()
            .to_string()
            .contains("twice")
    );

    let mut store = MemoryLocal::default();
    store.set_unreadable(Some("the Keychain is locked"));
    assert!(matches!(Local::load(&mut store), Err(StoreError::Read(_))));
}

#[test]
fn declared_grants_are_validated() {
    for (source, problem) in [
        (
            "version = 1\n[packages.a]\ngrant = [\"dictionary\"]",
            "need the `revision`",
        ),
        (
            "version = 1\n[packages.a]\nrevision = \"r\"\ngrant = [\"dictionary\"]\ndeny = [\"dictionary\"]",
            "both granted and denied",
        ),
        (
            "version = 1\n[packages.a]\nrevision = \"r\"\ngrant = [\"files:notes\"]",
            "needs a path",
        ),
        (
            "version = 1\n[packages.a]\nrevision = \"r\"\nbindings = { \"files:notes\" = \"/x\" }",
            "no grant",
        ),
        (
            "version = 1\n[packages.a]\nrevision = \"r\"\ngrant = [\"exec:git\"]\nbindings = { \"exec:git\" = \"git\" }",
            "absolute path",
        ),
        (
            "version = 1\n[packages.a]\nrevision = \"r\"\nallow = []",
            "unknown field",
        ),
        (
            "version = 1\n[packages.a]\ndeny = [\"calender\"]",
            "isn't a scope",
        ),
    ] {
        let base = Base::parse(&format!("{source}\n[packages.b]\ndeny = [\"dictionary\"]")).unwrap();
        let [error] = base.problems().try_into().unwrap();
        assert!(
            error.contains("for a ") && error.contains(problem),
            "{source}: {error}"
        );
        assert!(base.rejects(&id("a")));
        assert!(
            base.package(&id("b")).is_some(),
            "{source}: an invalid entry leaves the others in force"
        );
    }
    let error = Base::parse("[packages.a]\nrevision = \"r\"")
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing field"), "{error}");
    assert_eq!(Base::parse("version = 1").unwrap(), Base::default());
}
