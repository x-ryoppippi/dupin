use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::manifest::Capabilities;
use crate::request::{Request, Resolved};
use crate::scope::{OsPermission, PackageId, Revision, Scope, ScopeKey};
use crate::store::{self, Base, BasePackage, Local, LocalPackage, Record, StoreError};

/// What prompted an operation. Only the host sets this, from input it received itself; guest
/// code can't claim an activation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Work nobody asked for just now, such as a watch callback or a refresh.
    Background,
    /// A search or view request while the user types.
    Query,
    /// A deliberate user action: Enter, a pressed action, a submitted form.
    Activation,
}

/// What the OS reports for an [`OsPermission`]. The host reads it without prompting and
/// passes it to [`Broker::set_os_access`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsAccess {
    Allowed,
    NotDetermined,
    Denied,
    Restricted,
}

/// Stamps a package's in-flight work, cached results and actions. Any loss of authority
/// moves the package to a new generation, so everything stamped with an older one is dropped.
/// Generations never repeat within a process, so granting again can't revive old work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

/// Authority for one operation. Check it again with [`Broker::validate`] before releasing the
/// operation's result or performing its effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permit {
    package: PackageId,
    revision: Revision,
    generation: Generation,
    scope: Scope,
    binding: Option<PathBuf>,
}

impl Permit {
    pub fn package(&self) -> &PackageId {
        &self.package
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// The declared scope this operation runs under, with its bounds (quota, retention,
    /// calendar window).
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The root or executable the user picked for a files or exec scope.
    pub fn binding(&self) -> Option<&Path> {
        self.binding.as_deref()
    }
}

#[derive(Debug)]
#[must_use]
pub enum Decision {
    Allow(Permit),
    Deny(Denial),
    NeedsConsent(ConsentNeeded),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denial {
    NotInstalled,
    /// The request came from a revision that has since been replaced.
    StaleRevision,
    /// The saved grants couldn't be read, so no extension gets access.
    GrantsUnreadable,
    Disabled,
    Invalid(String),
    Undeclared(ScopeKey),
    OutOfScope {
        key: ScopeKey,
        reason: String,
    },
    NeedsActivation,
    Revoked(ScopeKey),
    DeniedByBase(ScopeKey),
    OsDenied(OsPermission),
    OsRestricted(OsPermission),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentNeeded {
    pub key: ScopeKey,
    pub reason: ConsentReason,
    /// Whether the host may ask now. It asks only for a deliberate action, never while the
    /// user types or in the background.
    pub may_prompt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentReason {
    NotGranted,
    /// A new revision widened a scope the user had approved.
    Changed,
    OsAccess(OsPermission),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantSource {
    Local,
    Base,
    Bundled,
}

/// A declared scope's standing, for Settings and the lock file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsentState {
    Granted(GrantSource),
    Revoked,
    DeniedByBase,
    Changed,
    Undecided,
}

/// One package scope that also needs an OS permission, for Settings → Permissions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsConsumer {
    pub permission: OsPermission,
    pub package: PackageId,
    pub scope: ScopeKey,
    pub enabled: bool,
    pub granted: bool,
}

/// What a manifest load or update did to the package's grants. It never grants anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reconciliation {
    /// Approved scopes this revision declares no wider, still granted.
    pub kept: Vec<ScopeKey>,
    /// Approved scopes this revision widens, which need consent again.
    pub changed: Vec<ScopeKey>,
    /// Decisions about scopes this revision no longer declares, forgotten.
    pub dropped: Vec<ScopeKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrokerError {
    NotInstalled,
    Undeclared(ScopeKey),
    DeniedByBase(ScopeKey),
    DisabledByBase,
    BindingRequired(ScopeKey),
    UnexpectedBinding(ScopeKey),
    RelativeBinding(ScopeKey),
    GrantsUnreadable,
}

impl fmt::Display for BrokerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => f.write_str("the extension isn't installed"),
            Self::Undeclared(key) => write!(f, "the extension doesn't declare `{key}`"),
            Self::DeniedByBase(key) => write!(f, "`{key}` is denied by your declared configuration"),
            Self::DisabledByBase => f.write_str("the extension is turned off by your declared configuration"),
            Self::BindingRequired(key) => write!(f, "`{key}` needs a file, folder or program you pick"),
            Self::UnexpectedBinding(key) => write!(f, "`{key}` isn't bound to a path"),
            Self::RelativeBinding(key) => write!(f, "the path picked for `{key}` must be absolute"),
            Self::GrantsUnreadable => StoreError::Unreadable.fmt(f),
        }
    }
}

impl std::error::Error for BrokerError {}

impl fmt::Display for Denial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => f.write_str("The extension isn't installed."),
            Self::StaleRevision => f.write_str("The extension changed; try again."),
            Self::GrantsUnreadable => f.write_str(
                "Dupin couldn't read its saved extension permissions, so extensions have no access.",
            ),
            Self::Disabled => f.write_str("The extension is turned off."),
            Self::Invalid(reason) => write!(f, "The request is invalid: {reason}."),
            Self::Undeclared(key) => write!(f, "The extension doesn't declare `{key}`."),
            Self::OutOfScope { reason, .. } => write!(f, "Not allowed: {reason}."),
            Self::NeedsActivation => f.write_str("This needs you to choose the action."),
            Self::Revoked(key) => write!(f, "You turned off `{key}` for this extension."),
            Self::DeniedByBase(key) => write!(f, "Your declared configuration denies `{key}`."),
            Self::OsDenied(permission) | Self::OsRestricted(permission) => {
                let name = match permission {
                    OsPermission::Calendars => "Calendars",
                    OsPermission::Pasteboard => "Paste from Other Apps",
                };
                let by = if matches!(self, Self::OsDenied(_)) {
                    "denied"
                } else {
                    "restricted"
                };
                write!(f, "macOS {by} {name} access for Dupin.")
            }
        }
    }
}

struct Installed {
    revision: Revision,
    capabilities: Capabilities,
    generation: Generation,
}

enum Effective {
    Granted {
        source: GrantSource,
        binding: Option<PathBuf>,
    },
    Revoked,
    DeniedByBase,
    Changed,
    Undecided,
    Unreadable,
}

/// The host's single authority over extension access. Every state change goes through an
/// explicit call: loading or reloading a manifest ([`Broker::install`]) only ever narrows
/// what a package holds.
pub struct Broker {
    base: Base,
    // The base read at startup. A base replaced while running may take authority away but
    // grants nothing new until the next launch, as a reload must never grant.
    ceiling: Base,
    local: Local,
    bundled: BTreeMap<PackageId, Revision>,
    installed: BTreeMap<PackageId, Installed>,
    os: BTreeMap<OsPermission, OsAccess>,
    sequence: u64,
}

impl Broker {
    pub fn new(base: Base, local: Local) -> Self {
        Self {
            ceiling: base.clone(),
            base,
            local,
            bundled: BTreeMap::new(),
            installed: BTreeMap::new(),
            os: BTreeMap::new(),
            sequence: 0,
        }
    }

    fn next(&mut self) -> Generation {
        self.sequence = self.sequence.checked_add(1).expect("broker generation exhausted");
        Generation(self.sequence)
    }

    fn bump(&mut self, id: &PackageId) {
        let generation = self.next();
        if let Some(installed) = self.installed.get_mut(id) {
            installed.generation = generation;
        }
    }

    /// Records a first-party package that ships inside Dupin. While the installed revision of
    /// `id` is exactly this one, the package is on by default and its scopes are pre-granted;
    /// a modified copy is any other revision and gets neither.
    pub fn set_bundled(&mut self, id: PackageId, revision: Revision) {
        self.bundled.insert(id, revision);
    }

    /// Loads or updates a package's declaration. Scopes it no longer declares lose their
    /// decisions; approvals it widens need consent again; nothing new is granted.
    pub fn install(
        &mut self,
        id: PackageId,
        revision: Revision,
        capabilities: Capabilities,
    ) -> Reconciliation {
        let mut report = Reconciliation::default();
        if let Some(package) = self
            .local
            .packages
            .as_mut()
            .and_then(|packages| packages.get_mut(&id))
        {
            package.scopes.retain(|key, record| {
                let Some(declared) = capabilities.get(key) else {
                    report.dropped.push(key.clone());
                    return false;
                };
                if let Record::Granted {
                    revision: approved_at,
                    approved,
                    ..
                } = record
                {
                    if approved.covers(declared) {
                        *approved_at = revision.clone();
                        *approved = declared.clone();
                        report.kept.push(key.clone());
                    } else {
                        report.changed.push(key.clone());
                    }
                }
                true
            });
        }
        let same = self
            .installed
            .get(&id)
            .filter(|installed| installed.revision == revision && installed.capabilities == capabilities)
            .map(|installed| installed.generation);
        let generation = match same {
            Some(generation) => generation,
            None => self.next(),
        };
        self.installed.insert(
            id,
            Installed {
                revision,
                capabilities,
                generation,
            },
        );
        report
    }

    /// Forgets the package and the user's decisions about it; reinstalling starts over.
    pub fn uninstall(&mut self, id: &PackageId) -> bool {
        let removed = self.installed.remove(id).is_some();
        if let Some(packages) = self.local.packages.as_mut() {
            packages.remove(id);
        }
        self.next();
        removed
    }

    pub fn set_enabled(&mut self, id: &PackageId, enabled: bool) -> Result<(), BrokerError> {
        let was = self.is_enabled(id);
        let Some(installed) = self.installed.get(id) else {
            return Err(BrokerError::NotInstalled);
        };
        if enabled && self.base_enabled(id, &installed.revision) == Some(false) {
            return Err(BrokerError::DisabledByBase);
        }
        let packages = self
            .local
            .packages
            .as_mut()
            .ok_or(BrokerError::GrantsUnreadable)?;
        packages.entry(id.clone()).or_default().enabled = Some(enabled);
        if was && !enabled {
            self.bump(id);
        }
        Ok(())
    }

    pub fn is_enabled(&self, id: &PackageId) -> bool {
        let Some(installed) = self.installed.get(id) else {
            return false;
        };
        let Some(packages) = &self.local.packages else {
            return false;
        };
        let local = packages.get(id).and_then(|package| package.enabled);
        let base = self.base_enabled(id, &installed.revision);
        match (local, base) {
            (Some(false), _) | (_, Some(false)) => false,
            (Some(true), _) | (_, Some(true)) => true,
            (None, None) => self.pristine(id, &installed.revision),
        }
    }

    /// Records the user's approval of a declared scope for the installed revision. `binding`
    /// is the absolute path the user picked for a files or exec scope.
    pub fn consent(
        &mut self,
        id: &PackageId,
        key: &ScopeKey,
        binding: Option<PathBuf>,
    ) -> Result<(), BrokerError> {
        let installed = self.installed.get(id).ok_or(BrokerError::NotInstalled)?;
        let declared = installed
            .capabilities
            .get(key)
            .ok_or_else(|| BrokerError::Undeclared(key.clone()))?;
        if self.base_denies(id, key) {
            return Err(BrokerError::DeniedByBase(key.clone()));
        }
        match (&binding, declared.needs_binding()) {
            (None, true) => return Err(BrokerError::BindingRequired(key.clone())),
            (Some(_), false) => return Err(BrokerError::UnexpectedBinding(key.clone())),
            (Some(path), true) if !path.is_absolute() => {
                return Err(BrokerError::RelativeBinding(key.clone()));
            }
            _ => {}
        }
        let record = Record::Granted {
            revision: installed.revision.clone(),
            approved: declared.clone(),
            binding,
        };
        let packages = self
            .local
            .packages
            .as_mut()
            .ok_or(BrokerError::GrantsUnreadable)?;
        let previous = packages
            .entry(id.clone())
            .or_default()
            .scopes
            .insert(key.clone(), record.clone());
        let rebound = matches!(
            (&previous, &record),
            (Some(Record::Granted { binding: old, .. }), Record::Granted { binding: new, .. }) if old != new
        );
        if rebound {
            self.bump(id);
        }
        Ok(())
    }

    /// Takes a scope away at once, whoever granted it. Work stamped with the package's current
    /// generation is dropped.
    pub fn revoke(&mut self, id: &PackageId, key: &ScopeKey) -> Result<(), BrokerError> {
        let installed = self.installed.get(id).ok_or(BrokerError::NotInstalled)?;
        if installed.capabilities.get(key).is_none() {
            return Err(BrokerError::Undeclared(key.clone()));
        }
        let packages = self
            .local
            .packages
            .as_mut()
            .ok_or(BrokerError::GrantsUnreadable)?;
        packages
            .entry(id.clone())
            .or_default()
            .scopes
            .insert(key.clone(), Record::Revoked);
        self.bump(id);
        Ok(())
    }

    /// Takes a newly read declarative base. Denials and removals apply at once; grants and
    /// enables it adds wait for the next launch.
    pub fn replace_base(&mut self, base: Base) {
        if base == self.base {
            return;
        }
        self.base = base;
        let ids: Vec<_> = self.installed.keys().cloned().collect();
        for id in ids {
            self.bump(&id);
        }
    }

    pub fn set_os_access(&mut self, permission: OsPermission, access: OsAccess) {
        let previous = self
            .os
            .insert(permission, access)
            .unwrap_or(OsAccess::NotDetermined);
        if previous != OsAccess::Allowed || access == OsAccess::Allowed {
            return;
        }
        let affected: Vec<_> = self
            .installed
            .iter()
            .filter(|(_, installed)| {
                installed
                    .capabilities
                    .scopes()
                    .any(|scope| scope.os_permission() == Some(permission))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in affected {
            self.bump(&id);
        }
    }

    pub fn generation(&self, id: &PackageId) -> Option<Generation> {
        self.installed.get(id).map(|installed| installed.generation)
    }

    /// Decides one operation at dispatch.
    pub fn check(
        &self,
        id: &PackageId,
        revision: &Revision,
        request: &Request,
        trigger: Trigger,
    ) -> Decision {
        let Some(installed) = self.installed.get(id) else {
            return Decision::Deny(Denial::NotInstalled);
        };
        if installed.revision != *revision {
            return Decision::Deny(Denial::StaleRevision);
        }
        if self.local.packages.is_none() {
            return Decision::Deny(Denial::GrantsUnreadable);
        }
        if !self.is_enabled(id) {
            return Decision::Deny(Denial::Disabled);
        }
        let key = match request.key() {
            Resolved::Scope(key) => key,
            Resolved::Invalid(reason) => return Decision::Deny(Denial::Invalid(reason)),
        };
        let Some(declared) = installed.capabilities.get(&key) else {
            return Decision::Deny(Denial::Undeclared(key));
        };
        if let Some(reason) = request.exceeds(declared) {
            return Decision::Deny(Denial::OutOfScope { key, reason });
        }
        if request.needs_activation() && trigger != Trigger::Activation {
            return Decision::Deny(Denial::NeedsActivation);
        }
        let needs = |reason| {
            Decision::NeedsConsent(ConsentNeeded {
                key: key.clone(),
                reason,
                may_prompt: trigger == Trigger::Activation,
            })
        };
        let binding = match self.effective(id, installed, &key, declared) {
            Effective::Granted { binding, .. } => binding,
            Effective::Revoked => return Decision::Deny(Denial::Revoked(key)),
            Effective::DeniedByBase => return Decision::Deny(Denial::DeniedByBase(key)),
            Effective::Changed => return needs(ConsentReason::Changed),
            Effective::Undecided => return needs(ConsentReason::NotGranted),
            Effective::Unreadable => return Decision::Deny(Denial::GrantsUnreadable),
        };
        if let Some(permission) = declared.os_permission() {
            match self.os_access(permission) {
                OsAccess::Allowed => {}
                OsAccess::NotDetermined => return needs(ConsentReason::OsAccess(permission)),
                OsAccess::Denied => return Decision::Deny(Denial::OsDenied(permission)),
                OsAccess::Restricted => return Decision::Deny(Denial::OsRestricted(permission)),
            }
        }
        Decision::Allow(Permit {
            package: id.clone(),
            revision: revision.clone(),
            generation: installed.generation,
            scope: declared.clone(),
            binding,
        })
    }

    /// Whether a permit still holds, checked before an operation's result is released or its
    /// effect performed.
    #[must_use]
    pub fn validate(&self, permit: &Permit) -> bool {
        let Some(installed) = self.installed.get(&permit.package) else {
            return false;
        };
        if installed.revision != permit.revision
            || installed.generation != permit.generation
            || !self.is_enabled(&permit.package)
        {
            return false;
        }
        let key = permit.scope.key();
        let Some(declared) = installed.capabilities.get(&key) else {
            return false;
        };
        let granted = matches!(
            self.effective(&permit.package, installed, &key, declared),
            Effective::Granted { binding, .. } if binding == permit.binding
        );
        granted
            && declared
                .os_permission()
                .is_none_or(|permission| self.os_access(permission) == OsAccess::Allowed)
    }

    /// Every declared scope of the package and where it stands.
    pub fn consents(&self, id: &PackageId) -> Vec<(Scope, ConsentState)> {
        let Some(installed) = self.installed.get(id) else {
            return Vec::new();
        };
        installed
            .capabilities
            .scopes()
            .map(|scope| {
                let state = match self.effective(id, installed, &scope.key(), scope) {
                    Effective::Granted { source, .. } => ConsentState::Granted(source),
                    Effective::Revoked => ConsentState::Revoked,
                    Effective::DeniedByBase => ConsentState::DeniedByBase,
                    Effective::Changed => ConsentState::Changed,
                    Effective::Undecided | Effective::Unreadable => ConsentState::Undecided,
                };
                (scope.clone(), state)
            })
            .collect()
    }

    /// Installed package scopes that need an OS permission, so Settings → Permissions can list
    /// extensions beside the built-in features that use the same permission.
    pub fn os_consumers(&self) -> Vec<OsConsumer> {
        self.installed
            .iter()
            .flat_map(|(id, installed)| {
                let enabled = self.is_enabled(id);
                installed.capabilities.scopes().filter_map(move |scope| {
                    let key = scope.key();
                    Some(OsConsumer {
                        permission: scope.os_permission()?,
                        package: id.clone(),
                        granted: matches!(
                            self.effective(id, installed, &key, scope),
                            Effective::Granted { .. }
                        ),
                        scope: key,
                        enabled,
                    })
                })
            })
            .collect()
    }

    /// Writes the user's decisions to `path`, replacing it whole.
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        store::write_atomically(path, &self.local.to_toml()?)
    }

    #[cfg(test)]
    pub(crate) fn local_toml(&self) -> Result<String, StoreError> {
        self.local.to_toml()
    }

    fn os_access(&self, permission: OsPermission) -> OsAccess {
        self.os
            .get(&permission)
            .copied()
            .unwrap_or(OsAccess::NotDetermined)
    }

    fn pristine(&self, id: &PackageId, revision: &Revision) -> bool {
        self.bundled.get(id) == Some(revision)
    }

    fn base_packages<'a>(&'a self, id: &'a PackageId) -> impl Iterator<Item = &'a BasePackage> + 'a {
        [&self.base, &self.ceiling]
            .into_iter()
            .filter_map(move |base| base.package(id))
    }

    fn base_denies(&self, id: &PackageId, key: &ScopeKey) -> bool {
        self.base_packages(id).any(|package| package.deny.contains(key))
    }

    fn base_enabled(&self, id: &PackageId, revision: &Revision) -> Option<bool> {
        let applies = |package: &BasePackage| package.revision.as_ref().is_none_or(|r| r == revision);
        let current = self
            .base
            .package(id)
            .filter(|p| applies(p))
            .and_then(|p| p.enabled);
        let ceiling = self
            .ceiling
            .package(id)
            .filter(|p| applies(p))
            .and_then(|p| p.enabled);
        match (current, ceiling) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        }
    }

    /// The base's grant of `key` for this revision, which both the current base and the one
    /// read at startup must make, with the same binding.
    fn base_grant(&self, id: &PackageId, revision: &Revision, key: &ScopeKey) -> Option<Option<PathBuf>> {
        let grant = |base: &Base| {
            base.package(id)
                .filter(|package| package.revision.as_ref() == Some(revision) && package.grant.contains(key))
                .map(|package| package.bindings.get(key).cloned())
        };
        let current = grant(&self.base)?;
        (grant(&self.ceiling)? == current).then_some(current)
    }

    fn effective(
        &self,
        id: &PackageId,
        installed: &Installed,
        key: &ScopeKey,
        declared: &Scope,
    ) -> Effective {
        let Some(packages) = &self.local.packages else {
            return Effective::Unreadable;
        };
        let record = packages
            .get(id)
            .and_then(|package: &LocalPackage| package.scopes.get(key));
        if matches!(record, Some(Record::Revoked)) {
            return Effective::Revoked;
        }
        if self.base_denies(id, key) {
            return Effective::DeniedByBase;
        }
        let mut changed = false;
        if let Some(Record::Granted {
            revision,
            approved,
            binding,
        }) = record
        {
            if *revision == installed.revision && approved.covers(declared) {
                return Effective::Granted {
                    source: GrantSource::Local,
                    binding: binding.clone(),
                };
            }
            changed = true;
        }
        if let Some(binding) = self.base_grant(id, &installed.revision, key) {
            return Effective::Granted {
                source: GrantSource::Base,
                binding,
            };
        }
        if self.pristine(id, &installed.revision) && !declared.needs_binding() {
            return Effective::Granted {
                source: GrantSource::Bundled,
                binding: None,
            };
        }
        if changed {
            Effective::Changed
        } else {
            Effective::Undecided
        }
    }
}
