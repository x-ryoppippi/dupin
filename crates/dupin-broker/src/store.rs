use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::scope::{PackageId, Revision, Scope, ScopeKey};

const VERSION: u32 = 1;

#[derive(Debug)]
pub enum StoreError {
    /// The declared base or a saved record isn't of the expected shape.
    Invalid(String),
    /// The local grants couldn't be read, so nothing may overwrite them.
    Unreadable,
    /// The [`LocalStore`] couldn't be read.
    Read(String),
    /// The [`LocalStore`] couldn't be written.
    Write(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Unreadable => f.write_str("the saved extension grants couldn't be read"),
            Self::Read(error) => write!(f, "couldn't read extension grants: {error}"),
            Self::Write(error) => write!(f, "couldn't save extension grants: {error}"),
        }
    }
}

impl std::error::Error for StoreError {}

fn invalid(name: &str, error: toml::de::Error) -> StoreError {
    StoreError::Invalid(format!("{name} is invalid: {}", error.message()))
}

/// The read-only declarative grants, such as a Home Manager module writes. They apply only to
/// the revision they name, so changing a package's contents needs a new declaration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Base {
    pub(crate) packages: BTreeMap<PackageId, BasePackage>,
    /// Entries that couldn't be read, by the key they were written under, with why. What such
    /// an entry meant to deny is unknown, so the broker keeps its package off.
    rejected: BTreeMap<String, String>,
    /// Why a file that exists couldn't be used at all. It carries the user's denials, none of
    /// which are known, so the broker gives no package anything from the base or a bundled
    /// pre-grant while it stands.
    unusable: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BasePackage {
    pub revision: Option<Revision>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub grant: BTreeSet<ScopeKey>,
    #[serde(default)]
    pub deny: BTreeSet<ScopeKey>,
    #[serde(default)]
    pub bindings: BTreeMap<ScopeKey, PathBuf>,
}

impl BasePackage {
    fn read(value: toml::Value) -> Result<Self, String> {
        let package: Self = value
            .try_into()
            .map_err(|e: toml::de::Error| e.message().to_owned())?;
        if package.revision.is_none() && !package.grant.is_empty() {
            return Err("grants need the `revision` they approve".into());
        }
        if let Some(key) = package.grant.intersection(&package.deny).next() {
            return Err(format!("`{key}` is both granted and denied"));
        }
        for key in &package.grant {
            if matches!(key, ScopeKey::Files(_) | ScopeKey::Exec(_)) && !package.bindings.contains_key(key) {
                return Err(format!("`{key}` needs a path in `bindings`"));
            }
        }
        for (key, path) in &package.bindings {
            if !package.grant.contains(key) {
                return Err(format!("`{key}` has a binding but no grant"));
            }
            if !path.is_absolute() {
                return Err(format!("the binding for `{key}` must be an absolute path"));
            }
        }
        Ok(package)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BaseFile {
    version: u32,
    #[serde(default)]
    packages: BTreeMap<String, toml::Value>,
}

impl Base {
    /// Reads the file, setting aside each package entry that is invalid rather than the whole
    /// file, so one mistyped scope leaves every other declaration in force.
    pub fn parse(source: &str) -> Result<Self, StoreError> {
        let file: BaseFile = toml::from_str(source).map_err(|e| invalid("the declared grants", e))?;
        check_version(file.version)?;
        let mut base = Self::default();
        for (key, value) in file.packages {
            let read = PackageId::new(key.as_str()).and_then(|id| Ok((id, BasePackage::read(value)?)));
            match read {
                Ok((id, package)) => {
                    base.packages.insert(id, package);
                }
                Err(message) => {
                    base.rejected.insert(key, message);
                }
            }
        }
        Ok(base)
    }

    pub(crate) fn package(&self, id: &PackageId) -> Option<&BasePackage> {
        self.packages.get(id)
    }

    /// Reads the base at `path`. A missing file declares nothing; one that can't be read or
    /// parsed is [`Base::unusable`].
    pub fn read(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(source) => Self::parse(&source).unwrap_or_else(|error| Self::unusable(error.to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => Self::unusable(error.to_string()),
        }
    }

    /// Stands in for a base file that exists but couldn't be used, for `reason`.
    pub fn unusable(reason: impl Into<String>) -> Self {
        Self {
            unusable: Some(reason.into()),
            ..Self::default()
        }
    }

    pub(crate) fn is_unusable(&self) -> bool {
        self.unusable.is_some()
    }

    pub(crate) fn rejects(&self, id: &PackageId) -> bool {
        self.rejected.contains_key(id.as_str())
    }

    /// Why the file, or each entry set aside, couldn't be used, for Settings to show.
    pub fn problems(&self) -> Vec<String> {
        if let Some(reason) = &self.unusable {
            return vec![format!(
                "the declared grants can't be used, so no extension gets anything from them or \
                 anything bundled extensions ship with, only what you allow in Dupin: {reason}"
            )];
        }
        self.rejected
            .iter()
            .map(|(key, message)| {
                format!("the declared grants for {key} are invalid, so it stays off: {message}")
            })
            .collect()
    }
}

/// The user's own choices, one record per package in a [`LocalStore`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    pub(crate) packages: Option<BTreeMap<PackageId, LocalPackage>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalPackage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub scopes: BTreeMap<ScopeKey, Record>,
}

impl LocalPackage {
    fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.scopes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Record {
    /// `approved` is the scope as declared when the user approved it, so a later revision that
    /// widens it asks again.
    Granted {
        revision: Revision,
        approved: Scope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binding: Option<PathBuf>,
    },
    Revoked,
}

/// One package's decisions as a [`LocalStore`] keeps them: `{ "package": …, "enabled": …,
/// "scopes": { … } }`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalRecord {
    package: PackageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(default)]
    scopes: BTreeMap<ScopeKey, Record>,
}

/// Where the user's decisions are kept: one record per package with any, which only the broker
/// encodes and reads. The store itself must keep a record from being read as another
/// package's.
pub trait LocalStore: Send {
    /// Every record [`LocalStore::write`] was handed and didn't delete since.
    fn read(&mut self) -> Result<Vec<serde_json::Value>, String>;
    /// Replaces the record of each package in `changes`, or deletes it for `None`.
    fn write(&mut self, changes: Vec<(PackageId, Option<serde_json::Value>)>) -> Result<(), String>;
}

impl Default for Local {
    fn default() -> Self {
        Self {
            packages: Some(BTreeMap::new()),
        }
    }
}

impl Local {
    /// Reads every package's decisions from `store`. A store that can't be read, or a record
    /// that isn't valid, is an error, since what it would deny is unknown.
    pub fn load(store: &mut dyn LocalStore) -> Result<Self, StoreError> {
        Self::from_records(store.read().map_err(StoreError::Read)?)
    }

    pub fn from_records(records: Vec<serde_json::Value>) -> Result<Self, StoreError> {
        let mut packages = BTreeMap::new();
        for record in records {
            let record: LocalRecord = serde_json::from_value(record)
                .map_err(|error| StoreError::Invalid(format!("the saved grants are invalid: {error}")))?;
            let id = record.package;
            for (key, decision) in &record.scopes {
                if let Record::Granted {
                    approved, binding, ..
                } = decision
                {
                    let problem = |message: &str| {
                        Err(StoreError::Invalid(format!(
                            "saved grants for {id}: `{key}` {message}"
                        )))
                    };
                    if approved.key() != *key {
                        return problem("records another scope");
                    }
                    if approved.needs_binding() != binding.is_some() {
                        return problem("has the wrong binding");
                    }
                    if binding.as_deref().is_some_and(|path| !path.is_absolute()) {
                        return problem("must bind an absolute path");
                    }
                }
            }
            let package = LocalPackage {
                enabled: record.enabled,
                scopes: record.scopes,
            };
            if packages.insert(id.clone(), package).is_some() {
                return Err(StoreError::Invalid(format!(
                    "the saved grants for {id} are saved twice"
                )));
            }
        }
        Ok(Self {
            packages: Some(packages),
        })
    }

    /// The package a record of [`Local::record`] names, for a store that checks it was kept
    /// under that package.
    pub fn record_package(record: &serde_json::Value) -> Option<PackageId> {
        PackageId::new(record.get("package")?.as_str()?).ok()
    }

    /// Stands in for grants that exist but couldn't be read. Every grant they might have
    /// revoked is unknown, so nothing is granted, bundled packages lose their pre-grants, and
    /// saving is refused rather than overwriting what is stored.
    pub fn unreadable() -> Self {
        Self { packages: None }
    }

    fn record(id: &PackageId, package: &LocalPackage) -> Result<serde_json::Value, StoreError> {
        serde_json::to_value(LocalRecord {
            package: id.clone(),
            enabled: package.enabled,
            scopes: package.scopes.clone(),
        })
        .map_err(|error| StoreError::Invalid(format!("couldn't encode grants: {error}")))
    }

    /// The record of each package whose decisions differ from `before`, or `None` for one left
    /// with none, which has no record.
    pub(crate) fn changes(
        &self,
        before: &Self,
    ) -> Result<Vec<(PackageId, Option<serde_json::Value>)>, StoreError> {
        let packages = self.packages.as_ref().ok_or(StoreError::Unreadable)?;
        let empty = BTreeMap::new();
        let previous = before.packages.as_ref().unwrap_or(&empty);
        let ids: BTreeSet<_> = packages.keys().chain(previous.keys()).collect();
        let mut changes = Vec::new();
        for id in ids {
            let now = packages.get(id).filter(|package| !package.is_empty());
            let was = previous.get(id).filter(|package| !package.is_empty());
            if now == was && before.packages.is_some() {
                continue;
            }
            changes.push((
                id.clone(),
                now.map(|package| Self::record(id, package)).transpose()?,
            ));
        }
        Ok(changes)
    }

    /// Every package's record, as [`LocalStore::write`] is handed them.
    pub fn records(&self) -> Result<Vec<serde_json::Value>, StoreError> {
        let packages = self.packages.as_ref().ok_or(StoreError::Unreadable)?;
        packages
            .iter()
            .filter(|(_, package)| !package.is_empty())
            .map(|(id, package)| Self::record(id, package))
            .collect()
    }
}

fn check_version(version: u32) -> Result<(), StoreError> {
    if version == VERSION {
        Ok(())
    } else {
        Err(StoreError::Invalid(format!(
            "grant file version {version} isn't supported; expected {VERSION}"
        )))
    }
}

#[derive(Debug, Default)]
struct Memory {
    records: BTreeMap<PackageId, serde_json::Value>,
    unreadable: Option<String>,
    unwritable: Option<String>,
}

/// Decisions kept in memory, for tests and fixtures. Clones share the records.
#[derive(Clone, Debug, Default)]
pub struct MemoryLocal(Arc<Mutex<Memory>>);

impl MemoryLocal {
    fn memory(&self) -> MutexGuard<'_, Memory> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn records(&self) -> BTreeMap<PackageId, serde_json::Value> {
        self.memory().records.clone()
    }

    /// Stores `record` under `id`, as another writer would.
    pub fn insert(&self, id: PackageId, record: serde_json::Value) {
        self.memory().records.insert(id, record);
    }

    /// Makes every read fail with `problem` while it is set.
    pub fn set_unreadable(&self, problem: Option<&str>) {
        self.memory().unreadable = problem.map(str::to_owned);
    }

    /// Makes every write fail with `problem` while it is set.
    pub fn set_unwritable(&self, problem: Option<&str>) {
        self.memory().unwritable = problem.map(str::to_owned);
    }
}

impl LocalStore for MemoryLocal {
    fn read(&mut self) -> Result<Vec<serde_json::Value>, String> {
        let memory = self.memory();
        match &memory.unreadable {
            Some(problem) => Err(problem.clone()),
            None => Ok(memory.records.values().cloned().collect()),
        }
    }

    fn write(&mut self, changes: Vec<(PackageId, Option<serde_json::Value>)>) -> Result<(), String> {
        let mut memory = self.memory();
        if let Some(problem) = &memory.unwritable {
            return Err(problem.clone());
        }
        for (id, record) in changes {
            match record {
                Some(record) => memory.records.insert(id, record),
                None => memory.records.remove(&id),
            };
        }
        Ok(())
    }
}
