use std::path::PathBuf;

use super::*;

#[test]
fn local_choices_survive_a_relaunch() {
    let package = id("bookmarks");
    let mut broker = Broker::new(Base::default(), Local::default());
    let _ = broker.install(package.clone(), rev("sha256:1111"), everything());
    broker.set_enabled(&package, true).unwrap();
    broker
        .consent(
            &package,
            &key("files:notes"),
            Some(PathBuf::from("/Users/me/Notes")),
        )
        .unwrap();
    broker
        .consent(&package, &key("http:https://api.github.com"), None)
        .unwrap();
    broker.revoke(&package, &ScopeKey::Dictionary).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/grants.toml");
    broker.save(&path).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!dir.path().join("nested/grants.toml.tmp").exists());

    let mut relaunched = Broker::new(Base::default(), Local::parse(&saved).unwrap());
    let report = relaunched.install(package.clone(), rev("sha256:1111"), everything());
    assert_eq!(report.kept.len(), 2);
    assert!(relaunched.is_enabled(&package));
    assert_eq!(relaunched.consents(&package), broker.consents(&package));
    assert_eq!(relaunched.local_toml().unwrap(), saved);
}

#[test]
fn the_saved_file_records_the_approved_scope_and_revision() {
    let package = id("bookmarks");
    let mut broker = Broker::new(Base::default(), Local::default());
    let _ = broker.install(package.clone(), rev("sha256:1111"), everything());
    broker
        .consent(&package, &key("http:https://api.github.com"), None)
        .unwrap();
    broker.revoke(&package, &ScopeKey::Dictionary).unwrap();
    let saved: toml::Table = broker.local_toml().unwrap().parse().unwrap();
    let expected: toml::Table = r#"
version = 1

[packages.bookmarks.scopes.dictionary]
decision = "revoked"

[packages.bookmarks.scopes."http:https://api.github.com"]
decision = "granted"
revision = "sha256:1111"

[packages.bookmarks.scopes."http:https://api.github.com".approved]
type = "http"
origin = "https://api.github.com"
methods = ["GET", "POST"]
"#
    .parse()
    .unwrap();
    assert_eq!(saved, expected);
}

#[test]
fn packages_with_no_decisions_are_not_saved() {
    let package = id("bookmarks");
    let mut broker = Broker::new(Base::default(), Local::default());
    let _ = broker.install(package.clone(), rev("sha256:1111"), everything());
    broker.consent(&package, &ScopeKey::Dictionary, None).unwrap();
    assert!(broker.uninstall(&package));
    assert_eq!(broker.local_toml().unwrap(), "version = 1\n\n[packages]\n");
}

#[test]
fn saved_grants_are_validated() {
    for (source, problem) in [
        ("version = 2", "version 2"),
        ("version = 1\nextra = true", "unknown field"),
        (
            "version = 1\n[packages.a.scopes.dictionary]\ndecision = \"granted\"\nrevision = \"r\"\napproved = { type = \"storage\", quota_bytes = 1 }",
            "records another scope",
        ),
        (
            "version = 1\n[packages.a.scopes.\"files:notes\"]\ndecision = \"granted\"\nrevision = \"r\"\napproved = { type = \"files\", name = \"notes\", kind = \"file\", access = [\"read\"] }",
            "wrong binding",
        ),
        (
            "version = 1\n[packages.a.scopes.\"files:notes\"]\ndecision = \"granted\"\nrevision = \"r\"\nbinding = \"Notes\"\napproved = { type = \"files\", name = \"notes\", kind = \"file\", access = [\"read\"] }",
            "absolute path",
        ),
        (
            "version = 1\n[packages.a.scopes.network]\ndecision = \"revoked\"",
            "isn't a scope",
        ),
    ] {
        let error = Local::parse(source).unwrap_err().to_string();
        assert!(error.contains(problem), "{source}: {error}");
    }
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
        ("[packages.a]\nrevision = \"r\"", "missing field"),
    ] {
        let error = Base::parse(source).unwrap_err().to_string();
        assert!(error.contains(problem), "{source}: {error}");
    }
    assert_eq!(Base::parse("version = 1").unwrap(), Base::default());
}
