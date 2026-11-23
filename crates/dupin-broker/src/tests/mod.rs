mod broker;
mod manifest;
mod scope;
mod store;

use std::collections::BTreeSet;

use serde_json::json;

use crate::*;

fn id(name: &str) -> PackageId {
    PackageId::new(name).unwrap()
}

fn rev(value: &str) -> Revision {
    Revision::new(value).unwrap()
}

fn name(value: &str) -> ScopeName {
    ScopeName::new(value).unwrap()
}

fn origin(value: &str) -> Origin {
    Origin::parse(value).unwrap()
}

fn methods(list: &[Method]) -> BTreeSet<Method> {
    list.iter().copied().collect()
}

fn http(origin_value: &str, list: &[Method]) -> Scope {
    Scope::Http {
        origin: origin(origin_value),
        methods: methods(list),
    }
}

fn key(value: &str) -> ScopeKey {
    value.parse().unwrap()
}

fn caps(value: serde_json::Value) -> Capabilities {
    Capabilities::parse(&value).unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"))
}

/// Every capability, as the examples in docs/features/extensions.md declare them.
fn everything() -> Capabilities {
    caps(json!({
        "http": [{ "origin": "https://api.github.com", "methods": ["GET", "POST"] }],
        "files": { "notes": { "kind": "directory", "access": ["read", "list"] } },
        "exec": {
            "git-status": { "args": ["status", "--short", "{path}"], "output": true },
            "@script:open-in-zed": {}
        },
        "storage": { "quotaBytes": 1024, "retentionDays": 30 },
        "clipboard": ["read", "write", "watch"],
        "dictionary": ["lookup"],
        "calendar": { "access": ["read"], "pastDays": 1, "futureDays": 30 }
    }))
}
