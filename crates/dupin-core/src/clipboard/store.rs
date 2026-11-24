//! Where the clipboard history persists. `History` keeps every entry in memory to search it
//! and writes each change through to a `Store`, which decides where the records live and how
//! they are protected. A store holds one owner's records: the native history today, and each
//! extension granted `storage` and `clipboard` once the broker exposes the history to them.

use std::fmt;
use std::sync::Arc;

use super::Record;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

pub trait Store: Send {
    /// Every record kept, in any order.
    fn list(&mut self) -> Result<Vec<Arc<Record>>, StoreError>;
    /// Keep a record, replacing any with the same ID.
    fn put(&mut self, record: &Arc<Record>) -> Result<(), StoreError>;
    fn delete(&mut self, id: u64) -> Result<(), StoreError>;
    fn clear(&mut self) -> Result<(), StoreError>;
    /// Delete every record copied before `before`, in Unix seconds.
    fn expire(&mut self, before: u64) -> Result<(), StoreError>;
}

/// Keeps records for as long as Dupin runs and writes nothing to disk.
#[derive(Debug, Default)]
pub struct MemoryStore {
    records: Vec<Arc<Record>>,
}

impl Store for MemoryStore {
    fn list(&mut self) -> Result<Vec<Arc<Record>>, StoreError> {
        Ok(self.records.clone())
    }

    fn put(&mut self, record: &Arc<Record>) -> Result<(), StoreError> {
        self.records.retain(|kept| kept.id != record.id);
        self.records.push(record.clone());
        Ok(())
    }

    fn delete(&mut self, id: u64) -> Result<(), StoreError> {
        self.records.retain(|record| record.id != id);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), StoreError> {
        self.records.clear();
        Ok(())
    }

    fn expire(&mut self, before: u64) -> Result<(), StoreError> {
        self.records.retain(|record| record.copied_at >= before);
        Ok(())
    }
}
