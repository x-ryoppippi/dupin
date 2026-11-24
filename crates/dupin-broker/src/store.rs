use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::scope::{PackageId, Revision, Scope, ScopeKey};

const VERSION: u32 = 1;

#[derive(Debug)]
pub enum StoreError {
    /// The file isn't valid TOML of the expected shape.
    Invalid(String),
    /// The local grants couldn't be read, so nothing may overwrite them.
    Unreadable,
    Io(std::io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Unreadable => f.write_str("the saved extension grants couldn't be read"),
            Self::Io(error) => write!(f, "couldn't save extension grants: {error}"),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BaseFile {
    version: u32,
    #[serde(default)]
    packages: BTreeMap<PackageId, BasePackage>,
}

impl Base {
    pub fn parse(source: &str) -> Result<Self, StoreError> {
        let file: BaseFile = toml::from_str(source).map_err(|e| invalid("the declared grants", e))?;
        check_version(file.version)?;
        for (id, package) in &file.packages {
            let problem = |message: String| {
                Err(StoreError::Invalid(format!(
                    "declared grants for {id}: {message}"
                )))
            };
            if package.revision.is_none() && !package.grant.is_empty() {
                return problem("grants need the `revision` they approve".into());
            }
            if let Some(key) = package.grant.intersection(&package.deny).next() {
                return problem(format!("`{key}` is both granted and denied"));
            }
            for key in &package.grant {
                if matches!(key, ScopeKey::Files(_) | ScopeKey::Exec(_))
                    && !package.bindings.contains_key(key)
                {
                    return problem(format!("`{key}` needs a path in `bindings`"));
                }
            }
            for (key, path) in &package.bindings {
                if !package.grant.contains(key) {
                    return problem(format!("`{key}` has a binding but no grant"));
                }
                if !path.is_absolute() {
                    return problem(format!("the binding for `{key}` must be an absolute path"));
                }
            }
        }
        Ok(Self {
            packages: file.packages,
        })
    }

    pub(crate) fn package(&self, id: &PackageId) -> Option<&BasePackage> {
        self.packages.get(id)
    }
}

/// The user's own choices, kept in the writable data directory.
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalFile {
    version: u32,
    #[serde(default)]
    packages: BTreeMap<PackageId, LocalPackage>,
}

impl Default for Local {
    fn default() -> Self {
        Self {
            packages: Some(BTreeMap::new()),
        }
    }
}

impl Local {
    pub fn parse(source: &str) -> Result<Self, StoreError> {
        let file: LocalFile = toml::from_str(source).map_err(|e| invalid("the saved grants", e))?;
        check_version(file.version)?;
        for (id, package) in &file.packages {
            for (key, record) in &package.scopes {
                if let Record::Granted {
                    approved, binding, ..
                } = record
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
        }
        Ok(Self {
            packages: Some(file.packages),
        })
    }

    /// Stands in for a grant file that exists but couldn't be read or parsed. Every grant it
    /// might have revoked is unknown, so nothing is granted, bundled packages lose their
    /// pre-grants, and saving is refused rather than overwriting the user's file.
    pub fn unreadable() -> Self {
        Self { packages: None }
    }

    pub(crate) fn to_toml(&self) -> Result<String, StoreError> {
        let packages = self.packages.clone().ok_or(StoreError::Unreadable)?;
        let file = LocalFile {
            version: VERSION,
            packages: packages
                .into_iter()
                .filter(|(_, package)| package.enabled.is_some() || !package.scopes.is_empty())
                .collect(),
        };
        toml::to_string(&file).map_err(|e| StoreError::Invalid(format!("couldn't encode grants: {e}")))
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

/// Replaces `path` whole, so a crash leaves the old grants or the new ones, never a mix.
pub(crate) fn write_atomically(path: &Path, contents: &str) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let mut file = std::fs::File::create(&temp).map_err(StoreError::Io)?;
    file.write_all(contents.as_bytes()).map_err(StoreError::Io)?;
    file.sync_all().map_err(StoreError::Io)?;
    std::fs::rename(&temp, path).map_err(StoreError::Io)
}
