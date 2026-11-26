//! The authority model every extension host service goes through.
//!
//! An extension declares the most it may do in the `capabilities` field of its manifest
//! ([`Capabilities::parse`]). Each declaration is a set of [`Scope`]s the user grants or
//! revokes one at a time. The [`Broker`] keeps those decisions per installed package and
//! approved revision, layered over a read-only declarative base ([`Base`]) and persisted, one
//! record per package, in a [`LocalStore`] the host provides ([`Local`]). Host services ask
//! [`Broker::check`] before an operation and [`Broker::validate`] before releasing its result.
//!
//! The crate performs no OS, network, file or process operations except reading the base file
//! a caller names; the host keeps the local grants. OS access is reported to it
//! ([`Broker::set_os_access`]), never requested by it.

mod broker;
mod manifest;
mod request;
mod scope;
mod store;

#[cfg(test)]
mod tests;

pub use broker::{
    Broker, BrokerError, ConsentNeeded, ConsentReason, ConsentState, Decision, Denial, Generation,
    GrantSource, OsAccess, OsConsumer, Permit, Reconciliation, SaveError, Trigger,
};
pub use manifest::{Capabilities, Diagnostic};
pub use request::Request;
pub use scope::{
    ClipboardAccess, ContextAccess, FilesAccess, FilesKind, Method, Origin, OsPermission, PackageId,
    Revision, RowField, RowKind, Scope, ScopeKey, ScopeName, StorageOp, WindowsAccess, displayable,
};
pub use store::{Base, Local, LocalStore, MemoryLocal, StoreError};
