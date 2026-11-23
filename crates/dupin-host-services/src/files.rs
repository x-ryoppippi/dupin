//! The `files` capability: named, user-picked file or directory scopes with read and list.
//!
//! Every operation opens the scope's anchor directory, checks it is still the one the user
//! picked, then opens one path component at a time relative to the previous descriptor with
//! `O_NOFOLLOW`. `..` is refused before any I/O and symbolic links are never followed, so a
//! path cannot name anything outside the scope. Before data is read, each opened descriptor
//! is checked to still be the entry of that name in its parent, which catches a directory moved
//! out of the scope while the path was being opened. Results carry [`Handle`]s rather than raw
//! paths, so actions that later act on a result name an entry this table already validated.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString, OsStr};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const DEFAULT_READ_BYTES: usize = 256 * 1024;
pub const MAX_READ_BYTES: usize = 4 * 1024 * 1024;
pub const DEFAULT_LIST_ENTRIES: usize = 200;
pub const MAX_LIST_ENTRIES: usize = 1000;
const MAX_DEPTH: usize = 64;
const MAX_PATH_BYTES: usize = 4096;
const MAX_COMPONENT_BYTES: usize = 255;
const MAX_SCOPE_NAME_BYTES: usize = 64;
const MAX_HANDLES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug)]
pub enum FileError {
    InvalidScopeName,
    UnknownScope(String),
    InvalidPath(&'static str),
    NotFound,
    Symlink,
    NotADirectory,
    NotAFile,
    ScopeChanged,
    Escaped,
    UnknownHandle,
    StaleHandle,
    Io(io::Error),
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScopeName => {
                f.write_str("Scope names use 1–64 lowercase letters, digits, hyphens or underscores.")
            }
            Self::UnknownScope(name) => write!(f, "No file access was granted for {name:?}."),
            Self::InvalidPath(problem) => write!(f, "Invalid path: {problem}."),
            Self::NotFound => f.write_str("Not found."),
            Self::Symlink => f.write_str("Symbolic links aren't followed inside a file scope."),
            Self::NotADirectory => f.write_str("Not a folder."),
            Self::NotAFile => f.write_str("Not a regular file."),
            Self::ScopeChanged => {
                f.write_str("The granted file or folder was moved or replaced. Choose it again in Settings.")
            }
            Self::Escaped => f.write_str("The entry moved outside the granted scope."),
            Self::UnknownHandle => f.write_str("Unknown file reference."),
            Self::StaleHandle => f.write_str("The referenced file was moved or replaced."),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for FileError {}

impl From<io::Error> for FileError {
    fn from(error: io::Error) -> Self {
        match error.raw_os_error() {
            Some(libc::ENOENT) => Self::NotFound,
            Some(libc::ELOOP) => Self::Symlink,
            Some(libc::ENOTDIR) => Self::NotADirectory,
            _ => Self::Io(error),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Identity {
    device: libc::dev_t,
    inode: libc::ino_t,
}

impl Identity {
    fn of(stat: &libc::stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        }
    }
}

fn kind(stat: &libc::stat) -> EntryKind {
    match stat.st_mode & libc::S_IFMT {
        libc::S_IFREG => EntryKind::File,
        libc::S_IFDIR => EntryKind::Directory,
        libc::S_IFLNK => EntryKind::Symlink,
        _ => EntryKind::Other,
    }
}

fn stat_fd(fd: RawFd) -> io::Result<libc::stat> {
    let mut stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(stat)
}

fn stat_at(directory: RawFd, name: &CStr) -> io::Result<libc::stat> {
    let mut stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(directory, name.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(stat)
}

fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

// O_NONBLOCK keeps a FIFO planted in the scope from blocking the open; the kind check then refuses it.
const OPEN_FLAGS: libc::c_int = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;

fn open_directory(path: &Path) -> Result<OwnedFd, FileError> {
    let path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| FileError::InvalidPath("contains NUL"))?;
    Ok(owned(unsafe {
        libc::open(path.as_ptr(), OPEN_FLAGS | libc::O_DIRECTORY)
    })?)
}

fn open_at(directory: RawFd, name: &CStr, flags: libc::c_int) -> Result<OwnedFd, FileError> {
    owned(unsafe { libc::openat(directory, name.as_ptr(), OPEN_FLAGS | flags) }).map_err(
        |error| match stat_at(directory, name) {
            Ok(stat) if kind(&stat) == EntryKind::Symlink => FileError::Symlink,
            _ => error.into(),
        },
    )
}

fn components(path: &str) -> Result<Vec<CString>, FileError> {
    if path.len() > MAX_PATH_BYTES {
        return Err(FileError::InvalidPath("longer than 4 KiB"));
    }
    if path.starts_with('/') {
        return Err(FileError::InvalidPath("must be relative to the scope"));
    }
    let mut names = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(FileError::InvalidPath("cannot contain ..")),
            _ if part.len() > MAX_COMPONENT_BYTES => {
                return Err(FileError::InvalidPath("a name is longer than 255 bytes"));
            }
            _ => names.push(CString::new(part).map_err(|_| FileError::InvalidPath("contains NUL"))?),
        }
    }
    if names.len() > MAX_DEPTH {
        return Err(FileError::InvalidPath("nested more than 64 levels"));
    }
    Ok(names)
}

/// A file or folder the user picked for one named scope of an installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    name: String,
    kind: ScopeKind,
    path: PathBuf,
    anchor_path: PathBuf,
    anchor: Identity,
}

impl Scope {
    /// Binds `name` to what the user picked. The picked path's symbolic links are resolved now,
    /// because the user chose their target; later operations follow none.
    pub fn pick(name: &str, kind: ScopeKind, picked: &Path) -> Result<Self, FileError> {
        let valid = (1..=MAX_SCOPE_NAME_BYTES).contains(&name.len())
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            });
        if !valid {
            return Err(FileError::InvalidScopeName);
        }
        let path = std::fs::canonicalize(picked)?;
        let metadata = std::fs::metadata(&path)?;
        let anchor_path = match kind {
            ScopeKind::Directory if !metadata.is_dir() => return Err(FileError::NotADirectory),
            ScopeKind::File if !metadata.is_file() => return Err(FileError::NotAFile),
            ScopeKind::Directory => path.clone(),
            ScopeKind::File => path.parent().ok_or(FileError::NotAFile)?.to_path_buf(),
        };
        let anchor = Identity::of(&stat_fd(open_directory(&anchor_path)?.as_raw_fd())?);
        Ok(Self {
            name: name.into(),
            kind,
            path,
            anchor_path,
            anchor,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> ScopeKind {
        self.kind
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What a grant allows, for the consent UI.
    pub fn describe(&self) -> String {
        let path = crate::display_path(&self.path);
        match self.kind {
            ScopeKind::File => format!("Read the file {path}."),
            ScopeKind::Directory => {
                format!(
                    "Read and list everything inside the folder {path}, without following symbolic links."
                )
            }
        }
    }

    fn open_anchor(&self) -> Result<OwnedFd, FileError> {
        let anchor = open_directory(&self.anchor_path).map_err(|error| match error {
            FileError::NotFound | FileError::Symlink | FileError::NotADirectory => FileError::ScopeChanged,
            error => error,
        })?;
        if Identity::of(&stat_fd(anchor.as_raw_fd())?) != self.anchor {
            return Err(FileError::ScopeChanged);
        }
        Ok(anchor)
    }

    fn names(&self, relative: Vec<CString>) -> Result<Vec<CString>, FileError> {
        match self.kind {
            ScopeKind::Directory => Ok(relative),
            ScopeKind::File if relative.is_empty() => Ok(vec![
                CString::new(self.path.file_name().unwrap_or_default().as_bytes())
                    .map_err(|_| FileError::InvalidPath("contains NUL"))?,
            ]),
            ScopeKind::File => Err(FileError::NotADirectory),
        }
    }

    fn walk(&self, names: &[CString], directory: bool) -> Result<Walk, FileError> {
        let mut chain = vec![self.open_anchor()?];
        for (index, name) in names.iter().enumerate() {
            let last = index + 1 == names.len();
            let flags = if !last || directory { libc::O_DIRECTORY } else { 0 };
            let next = open_at(chain[index].as_raw_fd(), name, flags)?;
            chain.push(next);
        }
        Ok(Walk { chain })
    }
}

struct Walk {
    chain: Vec<OwnedFd>,
}

impl Walk {
    fn target(&self) -> &OwnedFd {
        self.chain.last().unwrap()
    }

    fn verify(&self, names: &[CString]) -> Result<libc::stat, FileError> {
        for (index, name) in names.iter().enumerate() {
            let named = stat_at(self.chain[index].as_raw_fd(), name).map_err(|_| FileError::Escaped)?;
            let opened = stat_fd(self.chain[index + 1].as_raw_fd())?;
            if Identity::of(&named) != Identity::of(&opened) {
                return Err(FileError::Escaped);
            }
        }
        Ok(stat_fd(self.target().as_raw_fd())?)
    }
}

/// An opaque, host-validated reference to an entry a previous result returned. It is valid only
/// in the [`Files`] table that issued it and only while the entry keeps its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle {
    table: u64,
    id: u64,
}

impl Handle {
    pub fn parse(token: &str) -> Option<Self> {
        let (table, id) = token.split_once('.')?;
        let number = |text: &str| {
            (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .then(|| u64::from_str_radix(text, 16).ok())
                .flatten()
        };
        Some(Self {
            table: number(table)?,
            id: number(id)?,
        })
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}.{:x}", self.table, self.id)
    }
}

struct Record {
    scope: String,
    names: Vec<CString>,
    identity: Identity,
}

pub enum Target<'a> {
    Path { scope: &'a str, path: &'a str },
    Handle(Handle),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub handle: Option<Handle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub handle: Handle,
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contents {
    pub handle: Handle,
    pub bytes: Vec<u8>,
    pub size: u64,
    pub truncated: bool,
}

/// A handle resolved for a host action: the verified descriptor and the path it had when opened.
#[derive(Debug)]
pub struct Opened {
    pub descriptor: OwnedFd,
    pub kind: EntryKind,
    pub path: PathBuf,
}

/// One installation's granted scopes and the handles its results have carried.
pub struct Files {
    table: u64,
    next: u64,
    scopes: BTreeMap<String, Scope>,
    records: BTreeMap<u64, Record>,
    index: HashMap<(String, Vec<CString>), u64>,
}

impl Default for Files {
    fn default() -> Self {
        Self::new()
    }
}

impl Files {
    pub fn new() -> Self {
        static TABLES: AtomicU64 = AtomicU64::new(1);
        Self {
            table: TABLES.fetch_add(1, Ordering::Relaxed),
            next: 1,
            scopes: BTreeMap::new(),
            records: BTreeMap::new(),
            index: HashMap::new(),
        }
    }

    /// Grants `scope`, replacing a scope of the same name and invalidating its handles.
    pub fn grant(&mut self, scope: Scope) {
        self.forget(&scope.name);
        self.scopes.insert(scope.name.clone(), scope);
    }

    pub fn revoke(&mut self, name: &str) {
        self.forget(name);
        self.scopes.remove(name);
    }

    pub fn scopes(&self) -> impl Iterator<Item = &Scope> {
        self.scopes.values()
    }

    /// Lists at most `limit` entries (capped at [`MAX_LIST_ENTRIES`]) of a folder, sorted by name.
    /// Entries are taken in the order the file system returns them before sorting.
    pub fn list(&mut self, target: Target<'_>, limit: usize) -> Result<Listing, FileError> {
        let limit = limit.min(MAX_LIST_ENTRIES);
        let (scope, names, expected) = self.locate(target)?;
        let walk = scope.walk(&names, true)?;
        let stat = walk.verify(&names)?;
        if expected.is_some_and(|identity| identity != Identity::of(&stat)) {
            return Err(FileError::StaleHandle);
        }
        if kind(&stat) != EntryKind::Directory {
            return Err(FileError::NotADirectory);
        }
        let directory = walk.target();
        let (children, truncated) = read_directory(directory, limit)?;
        let scope = scope.name.clone();
        let handle = self.issue(&scope, names.clone(), Identity::of(&stat));
        let mut entries = Vec::with_capacity(children.len());
        for child in children {
            let Ok(stat) = stat_at(directory.as_raw_fd(), &child) else {
                continue;
            };
            let kind = kind(&stat);
            let handle = matches!(kind, EntryKind::File | EntryKind::Directory).then(|| {
                let mut path = names.clone();
                path.push(child.clone());
                self.issue(&scope, path, Identity::of(&stat))
            });
            entries.push(Entry {
                name: String::from_utf8_lossy(child.as_bytes()).into_owned(),
                kind,
                size: (kind == EntryKind::File).then(|| u64::try_from(stat.st_size).unwrap_or(0)),
                handle,
            });
        }
        Ok(Listing {
            handle,
            entries,
            truncated,
        })
    }

    /// Reads at most `limit` bytes (capped at [`MAX_READ_BYTES`]) from the start of a regular file.
    pub fn read(&mut self, target: Target<'_>, limit: usize) -> Result<Contents, FileError> {
        let limit = limit.min(MAX_READ_BYTES);
        let (scope, names, expected) = self.locate(target)?;
        let walk = scope.walk(&names, false)?;
        let stat = walk.verify(&names)?;
        if expected.is_some_and(|identity| identity != Identity::of(&stat)) {
            return Err(FileError::StaleHandle);
        }
        if kind(&stat) != EntryKind::File {
            return Err(FileError::NotAFile);
        }
        let size = u64::try_from(stat.st_size).unwrap_or(0);
        let mut file = File::from(walk.target().try_clone()?);
        let mut bytes = Vec::with_capacity(limit.min(usize::try_from(size).unwrap_or(usize::MAX)));
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        let truncated = bytes.len() > limit;
        bytes.truncate(limit);
        let scope = scope.name.clone();
        let handle = self.issue(&scope, names, Identity::of(&stat));
        Ok(Contents {
            handle,
            bytes,
            size,
            truncated,
        })
    }

    /// Re-opens a handle for a host action such as Open or Quick Look, verifying it still names
    /// the same entry inside its scope. Host actions should use the descriptor where they can.
    pub fn open(&self, handle: Handle) -> Result<Opened, FileError> {
        let (scope, names, expected) = self.locate(Target::Handle(handle))?;
        let walk = scope.walk(&names, false)?;
        let stat = walk.verify(&names)?;
        if expected.is_some_and(|identity| identity != Identity::of(&stat)) {
            return Err(FileError::StaleHandle);
        }
        let mut path = scope.anchor_path.clone();
        for name in &names {
            path.push(OsStr::from_bytes(name.as_bytes()));
        }
        let mut chain = walk.chain;
        Ok(Opened {
            descriptor: chain.pop().unwrap(),
            kind: kind(&stat),
            path,
        })
    }

    fn locate(&self, target: Target<'_>) -> Result<(&Scope, Vec<CString>, Option<Identity>), FileError> {
        match target {
            Target::Path { scope, path } => {
                let found = self
                    .scopes
                    .get(scope)
                    .ok_or_else(|| FileError::UnknownScope(scope.into()))?;
                Ok((found, found.names(components(path)?)?, None))
            }
            Target::Handle(handle) => {
                let record = (handle.table == self.table)
                    .then(|| self.records.get(&handle.id))
                    .flatten()
                    .ok_or(FileError::UnknownHandle)?;
                let scope = self.scopes.get(&record.scope).ok_or(FileError::UnknownHandle)?;
                Ok((scope, record.names.clone(), Some(record.identity)))
            }
        }
    }

    fn issue(&mut self, scope: &str, names: Vec<CString>, identity: Identity) -> Handle {
        let key = (scope.to_owned(), names);
        if let Some(&id) = self.index.get(&key) {
            if self.records[&id].identity == identity {
                return Handle {
                    table: self.table,
                    id,
                };
            }
            self.records.remove(&id);
        }
        while self.records.len() >= MAX_HANDLES {
            let (_, oldest) = self.records.pop_first().unwrap();
            self.index.remove(&(oldest.scope, oldest.names));
        }
        let id = self.next;
        self.next += 1;
        self.records.insert(
            id,
            Record {
                scope: key.0.clone(),
                names: key.1.clone(),
                identity,
            },
        );
        self.index.insert(key, id);
        Handle {
            table: self.table,
            id,
        }
    }

    fn forget(&mut self, scope: &str) {
        self.records.retain(|_, record| record.scope != scope);
        self.index.retain(|(name, _), _| name != scope);
    }
}

fn read_directory(directory: &OwnedFd, limit: usize) -> io::Result<(Vec<CString>, bool)> {
    let copy = owned(unsafe { libc::dup(directory.as_raw_fd()) })?;
    let stream = unsafe { libc::fdopendir(copy.as_raw_fd()) };
    if stream.is_null() {
        return Err(io::Error::last_os_error());
    }
    // The stream now owns the duplicate and closes it.
    std::mem::forget(copy);
    let mut names = Vec::new();
    let mut truncated = false;
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name == c"." || name == c".." {
            continue;
        }
        if names.len() == limit {
            truncated = true;
            break;
        }
        names.push(name.to_owned());
    }
    unsafe { libc::closedir(stream) };
    names.sort();
    Ok((names, truncated))
}

#[cfg(test)]
mod tests;
