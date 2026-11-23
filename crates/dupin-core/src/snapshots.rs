use crate::View;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

pub const MAX_ENTRIES: usize = 100;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ENTRY_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    pub id: String,
    pub saved_at: u64,
    pub mode: String,
    pub explicit: bool,
    pub query: String,
    pub title: String,
    pub views: Vec<View>,
    pub state: Option<dupin_plugin_protocol::State>,
}

impl Snapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.title.is_empty()
            || self.title.len() > 4096
            || self.query.len() > 4096
            || crate::CommandId::new(self.mode.clone()).is_err()
            || self.views.is_empty()
            || self.views.len() > 500
            || serde_json::to_vec(self)
                .map_err(|_| "Cannot encode snapshot.")?
                .len()
                > MAX_ENTRY_BYTES
        {
            return Err("Snapshot exceeds the supported size or schema.".into());
        }
        if let Some(state) = &self.state
            && (state.version == 0 || serde_json::to_vec(state).map_err(|_| "Invalid state.")?.len() > 8192)
        {
            return Err("Snapshot state exceeds the supported size or schema.".into());
        }
        Ok(())
    }

    pub fn content(&self) -> String {
        self.views
            .iter()
            .map(|v| match v {
                View::Text { title, subtitle } => format!("{title}\n{subtitle}"),
                View::Detail { title, body } => format!("{title}\n{body}"),
                View::Pair { source, result } => format!("{source}\n{result}"),
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn search_text(&self) -> String {
        normalize(&format!("{}\n{}\n{}", self.title, self.query, self.content()))
    }

    pub fn matches(&self, query: &str) -> bool {
        self.search_text().contains(&normalize(query))
    }
}

pub fn normalize(s: &str) -> String {
    s.nfkc().flat_map(char::to_lowercase).collect()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    version: u32,
    entries: Vec<Snapshot>,
}

/// A private directory dedicated to snapshots. Errors retain the original archive.
pub struct Store {
    directory: PathBuf,
    entries: Vec<Snapshot>,
    problem: Option<String>,
}

impl Store {
    pub fn from_base(base: PathBuf, owned: &Path) -> Self {
        let resolved = (|| {
            if !base.is_absolute() {
                return Err("Snapshot base directory must be absolute.".to_owned());
            }
            if owned
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err("Snapshot owned directory must be a relative leaf path.".to_owned());
            }
            std::fs::create_dir_all(&base).map_err(|_| "Cannot create snapshot base directory.")?;
            let base = base
                .canonicalize()
                .map_err(|_| "Cannot resolve snapshot base directory.")?;
            Ok(base.join(owned))
        })();
        match resolved {
            Ok(directory) => Self::load(directory),
            Err(problem) => Self {
                directory: PathBuf::new(),
                entries: Vec::new(),
                problem: Some(problem),
            },
        }
    }

    pub fn load(directory: PathBuf) -> Self {
        let mut store = Self {
            directory,
            entries: Vec::new(),
            problem: None,
        };
        match store.read() {
            Ok(entries) => store.entries = entries,
            Err(problem) => store.problem = Some(problem),
        }
        store
    }

    pub fn list(&self) -> &[Snapshot] {
        &self.entries
    }
    pub fn open(&self, id: &str) -> Option<&Snapshot> {
        self.entries.iter().find(|s| s.id == id)
    }
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    fn safe_directory(&self) -> Result<(), String> {
        for parent in self.directory.ancestors() {
            if let Ok(meta) = std::fs::symlink_metadata(parent)
                && (meta.file_type().is_symlink() || !meta.is_dir())
            {
                return Err("Snapshot directory must not contain symbolic links.".into());
            }
        }
        Ok(())
    }

    fn read(&self) -> Result<Vec<Snapshot>, String> {
        self.safe_directory()?;
        let path = self.directory.join("archive.json");
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Ok(m) if m.is_file() && !m.file_type().is_symlink() => {}
            _ => return Err("Cannot read snapshot archive safely.".into()),
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|f| f.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes))
            .map_err(|_| "Cannot read snapshot archive.")?;
        if bytes.len() > MAX_BYTES {
            return Err("Snapshot archive exceeds the byte limit.".into());
        }
        let archive: Archive = serde_json::from_slice(&bytes)
            .map_err(|_| "Snapshot archive is corrupt or unsupported; original data retained.")?;
        if archive.version != 1 || archive.entries.len() > MAX_ENTRIES {
            return Err(
                "Snapshot archive version or entry limit is unsupported; original data retained.".into(),
            );
        }
        let mut ids = std::collections::HashSet::new();
        for entry in &archive.entries {
            entry.validate()?;
            if entry.id.is_empty() || entry.id.len() > 128 || !ids.insert(&entry.id) {
                return Err("Snapshot archive contains invalid identifiers; original data retained.".into());
            }
        }
        Ok(archive.entries)
    }

    pub fn save(&mut self, mut snapshot: Snapshot) -> Result<String, String> {
        snapshot.saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Clock unavailable.")?
            .as_secs();
        snapshot.validate()?;
        snapshot.id = format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "Clock unavailable.")?
                .as_nanos()
        );
        if self.open(&snapshot.id).is_some() {
            return Err("Snapshot identifier collision.".into());
        }
        let id = snapshot.id.clone();
        let mut entries = self.entries.clone();
        entries.insert(0, snapshot);
        self.commit(entries)?;
        Ok(id)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        if let Some(problem) = &self.problem {
            return Err(problem.clone());
        }
        if self.open(id).is_none() {
            return Ok(());
        }
        let entries = self.entries.iter().filter(|s| s.id != id).cloned().collect();
        self.commit(entries)
    }

    fn commit(&mut self, entries: Vec<Snapshot>) -> Result<(), String> {
        if let Some(problem) = &self.problem {
            return Err(problem.clone());
        }
        if self.read()? != self.entries {
            return Err(
                "Snapshot archive changed on disk. Reopen Dupin before editing; data retained.".into(),
            );
        }
        if entries.len() > MAX_ENTRIES {
            return Err("Snapshot limit reached. Delete an entry before saving.".into());
        }
        let bytes = serde_json::to_vec(&Archive {
            version: 1,
            entries: entries.clone(),
        })
        .map_err(|_| "Cannot encode archive.")?;
        if bytes.len() > MAX_BYTES {
            return Err("Snapshot storage is full. Delete an entry before saving.".into());
        }
        self.safe_directory()?;
        write_private(&self.directory, &bytes).map_err(|_| "Cannot write snapshot archive safely.")?;
        self.entries = entries;
        Ok(())
    }
}

fn write_private(directory: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let path = directory.join("archive.json");
    if let Ok(m) = std::fs::symlink_metadata(&path)
        && (!m.is_file() || m.file_type().is_symlink())
    {
        return Err(std::io::Error::other("unsafe archive"));
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp = directory.join(format!(
        ".archive-{}-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        std::fs::File::open(directory)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "dupin-snapshots-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> Snapshot {
        Snapshot {
            version: 1,
            id: String::new(),
            saved_at: 0,
            mode: "sample.search".into(),
            explicit: false,
            query: "query".into(),
            title: "Café 東京".into(),
            views: vec![
                View::Text {
                    title: "row".into(),
                    subtitle: "subtitle".into(),
                },
                View::Detail {
                    title: "Definition".into(),
                    body: "保存された本文".into(),
                },
                View::Pair {
                    source: "2 + 2".into(),
                    result: "4".into(),
                },
            ],
            state: Some(dupin_plugin_protocol::State {
                version: 1,
                data: serde_json::json!({"page": 2}),
            }),
        }
    }

    #[test]
    fn canonical_base_accepts_links_but_owned_paths_reject_them() {
        let temp = Temp::new();
        let linked = temp.0.join("base-link");
        std::os::unix::fs::symlink(&temp.0, &linked).unwrap();
        let mut store = Store::from_base(linked.clone(), Path::new("dupin/snapshots"));
        store.save(fixture()).unwrap();
        assert!(store.list()[0].saved_at > 0);
        assert_eq!(
            Store::from_base(linked, Path::new("dupin/snapshots")).list(),
            store.list()
        );
        std::os::unix::fs::symlink(temp.0.join("dupin"), temp.0.join("owned-link")).unwrap();
        assert!(
            Store::from_base(temp.0.clone(), Path::new("owned-link/snapshots"))
                .problem()
                .is_some()
        );
        assert!(
            Store::from_base(PathBuf::new(), Path::new("dupin/snapshots"))
                .problem()
                .is_some()
        );
    }
    #[test]
    fn persistence_search_and_explicit_delete() {
        let temp = Temp::new();
        let mut store = Store::load(temp.0.clone());
        let id = store.save(fixture()).unwrap();
        let mut reopened = Store::load(temp.0.clone());
        assert_eq!(store.list(), reopened.list());
        let saved = reopened.open(&id).unwrap();
        assert!(saved.matches("CAFE\u{301}"));
        assert!(saved.matches("本文"));
        assert!(saved.matches("2 + 2"));
        assert!(!saved.matches("missing"));
        reopened.delete(&id).unwrap();
        assert!(Store::load(temp.0.clone()).list().is_empty());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(temp.0.join("archive.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&temp.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    #[test]
    fn invalid_data_is_retained_and_blocks_mutation() {
        for bytes in [
            b"broken".as_slice(),
            br#"{"version":2,"entries":[]}"#,
            br#"{"version":1,"entries":[],"extra":true}"#,
        ] {
            let temp = Temp::new();
            let path = temp.0.join("archive.json");
            std::fs::write(&path, bytes).unwrap();
            let mut store = Store::load(temp.0.clone());
            assert!(store.problem().is_some());
            assert!(store.save(fixture()).is_err());
            assert!(store.delete("anything").is_err());
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
    #[test]
    fn bounds_fail_without_changing_saved_data() {
        let temp = Temp::new();
        let mut store = Store::load(temp.0.clone());
        let mut oversized = fixture();
        oversized.views.push(View::Detail {
            title: "large".into(),
            body: "x".repeat(MAX_ENTRY_BYTES),
        });
        assert!(store.save(oversized).is_err());
        for _ in 0..MAX_ENTRIES {
            store.save(fixture()).unwrap();
        }
        assert!(store.save(fixture()).is_err());
        assert_eq!(Store::load(temp.0.clone()).list().len(), MAX_ENTRIES);
        let path = temp.0.join("archive.json");
        std::fs::write(path, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert!(Store::load(temp.0.clone()).problem().is_some());
    }
    #[test]
    fn symlinks_are_never_overwritten() {
        let temp = Temp::new();
        let target = temp.0.join("private");
        std::fs::write(&target, b"untouched").unwrap();
        std::os::unix::fs::symlink(&target, temp.0.join("archive.json")).unwrap();
        let mut store = Store::load(temp.0.clone());
        assert!(store.problem().is_some());
        assert!(store.save(fixture()).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"untouched");
        let linked = temp.0.join("linked");
        std::os::unix::fs::symlink(&temp.0, &linked).unwrap();
        assert!(Store::load(linked).problem().is_some());
    }

    #[test]
    fn dto_versions_fields_and_external_corruption_are_rejected() {
        let saved = fixture();
        let bytes = serde_json::to_vec(&saved).unwrap();
        assert_eq!(serde_json::from_slice::<Snapshot>(&bytes).unwrap(), saved);
        let mut value = serde_json::to_value(&saved).unwrap();
        value["actions"] = serde_json::json!(["execute"]);
        assert!(serde_json::from_value::<Snapshot>(value).is_err());
        let mut unsupported = saved.clone();
        unsupported.version = 2;
        assert!(unsupported.validate().is_err());
        unsupported = saved.clone();
        unsupported.state.as_mut().unwrap().version = 0;
        assert!(unsupported.validate().is_err());
        let temp = Temp::new();
        let mut store = Store::load(temp.0.clone());
        store.save(saved.clone()).unwrap();
        let path = temp.0.join("archive.json");
        std::fs::write(&path, b"corrupted externally").unwrap();
        assert!(store.save(saved).is_err());
        assert!(store.delete(&store.list()[0].id.clone()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"corrupted externally");
    }

    #[test]
    fn total_byte_limit_is_transactional() {
        let temp = Temp::new();
        let mut store = Store::load(temp.0.clone());
        let mut large = fixture();
        large.views.push(View::Detail {
            title: "large".into(),
            body: "x".repeat(400 * 1024),
        });
        for _ in 0..20 {
            store.save(large.clone()).unwrap();
        }
        let before = std::fs::read(temp.0.join("archive.json")).unwrap();
        assert!(store.save(large).is_err());
        assert_eq!(std::fs::read(temp.0.join("archive.json")).unwrap(), before);
        assert_eq!(Store::load(temp.0.clone()).list().len(), 20);
    }
}
