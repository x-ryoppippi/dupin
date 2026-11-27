//! Sync between Macs through a folder some other tool keeps in step, end-to-end encrypted with
//! one vault key. See `docs/design/sync.md` for the design and `docs/features/sync.md` for
//! what it does.
//!
//! Every call blocks on file I/O and must run on a worker thread, never the UI thread.

mod descriptor;
mod engine;
mod error;
mod folder;
mod fsx;
mod keys;
mod passphrase;
mod seal;
mod setup;
mod vault;
mod words;

#[cfg(test)]
mod tests;

pub use engine::{DeviceInfo, DeviceStatus, Engine, Fold, Mode, Namespace, Report};
pub use error::{Error, Refusal};
pub use folder::{Folder, FolderOptions, icloud_drive_default};
pub use keys::{LocalKeyStore, SharedKeyStore, account};
pub use passphrase::{Passphrase, Weakness};
pub use setup::{Joined, create, join, resume};
pub use vault::{KdfParams, KeyMode, VaultHeader};
pub use words::WordList;
