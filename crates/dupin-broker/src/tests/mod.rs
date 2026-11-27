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

impl Broker {
    /// Consents to `key` exactly as the installed revision declares it, as the host does with
    /// the prompt it just showed.
    #[track_caller]
    fn approve(
        &mut self,
        package: &PackageId,
        key: &ScopeKey,
        binding: Option<std::path::PathBuf>,
    ) -> Result<(), BrokerError> {
        let revision = self.revision(package).expect("installed").clone();
        let (scope, _) = self
            .consents(package)
            .into_iter()
            .find(|(scope, _)| scope.key() == *key)
            .unwrap_or_else(|| panic!("`{key}` isn't declared"));
        self.consent(package, &revision, &scope, binding)
    }
}

fn caps(value: serde_json::Value) -> Capabilities {
    Capabilities::parse(&value).unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"))
}

/// Every capability, as the example in docs/capability-broker.md declares them.
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
