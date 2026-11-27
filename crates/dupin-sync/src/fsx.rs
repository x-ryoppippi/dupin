//! File operations inside the sync folder that never follow a symbolic link. Anyone with write
//! access to the folder can plant one, so every path is resolved one component at a time from
//! an open directory with `O_NOFOLLOW`, and a link is treated as a file that isn't there.

use std::ffi::{CStr, CString};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `SF_DATALESS`: an iCloud Drive file whose contents are not on this Mac yet. Reading it would
/// block on a download.
#[cfg(target_os = "macos")]
const SF_DATALESS: u32 = 0x4000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    /// A link, device, socket or anything else Dupin never reads through.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub len: u64,
    pub modified: SystemTime,
    /// The contents have not been downloaded.
    pub dataless: bool,
}

/// What reading a file found.
#[derive(Debug)]
pub enum Contents {
    Bytes(Vec<u8>),
    Missing,
    /// Not downloaded yet, or larger than the caller accepts.
    Unavailable,
}

/// An open directory inside the folder.
#[derive(Debug)]
pub struct Dir {
    fd: OwnedFd,
}

fn cstr(name: &str) -> io::Result<CString> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a plain file name",
        ));
    }
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL in a file name"))
}

fn check(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn is_link_or_not_dir(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR))
}

impl Dir {
    /// Opens the folder itself. Its own path may pass through links the user chose; the folder
    /// must be a directory, not a link to one.
    pub fn open_root(path: &Path) -> io::Result<Self> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL in the path"))?;
        // SAFETY: A valid NUL-terminated path; the returned descriptor is owned below.
        let fd = check(unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        // SAFETY: `open` returned a fresh descriptor that nothing else owns.
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    fn raw(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// A subdirectory, or `None` when it is missing, a link or not a directory.
    pub fn open_dir(&self, name: &str) -> io::Result<Option<Dir>> {
        let name = cstr(name)?;
        // SAFETY: A valid directory descriptor and NUL-terminated name.
        let fd = unsafe {
            libc::openat(
                self.raw(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound || is_link_or_not_dir(&error) {
                return Ok(None);
            }
            return Err(error);
        }
        // SAFETY: A fresh descriptor that nothing else owns.
        Ok(Some(Dir {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        }))
    }

    /// A subdirectory, created `0700` if missing. A link or a file in its place is an error,
    /// never followed or replaced.
    pub fn create_dir(&self, name: &str) -> io::Result<Dir> {
        let c_name = cstr(name)?;
        // SAFETY: A valid directory descriptor and NUL-terminated name.
        let made = unsafe { libc::mkdirat(self.raw(), c_name.as_ptr(), 0o700) };
        if made < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        } else {
            self.sync()?;
        }
        self.open_dir(name)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name} in the sync folder is not a directory"),
            )
        })
    }

    /// Every entry, without following links: a link is listed as [`Kind::Other`].
    pub fn entries(&self) -> io::Result<Vec<Entry>> {
        // A plain `dup` would clear close-on-exec, and a program another thread spawned while the
        // listing ran would keep the folder open after Dupin let it go.
        // SAFETY: duplicating a valid descriptor; `fdopendir` takes ownership of the duplicate.
        let dup = check(unsafe { libc::fcntl(self.raw(), libc::F_DUPFD_CLOEXEC, 0) })?;
        // SAFETY: As above.
        let stream = unsafe { libc::fdopendir(dup) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: `fdopendir` failed, so the duplicate is still ours to close.
            unsafe { libc::close(dup) };
            return Err(error);
        }
        // SAFETY: A directory stream at its start, which `rewinddir` keeps so after a `dup`.
        unsafe { libc::rewinddir(stream) };
        let mut names = Vec::new();
        loop {
            // SAFETY: A valid stream; the entry stays valid until the next `readdir`.
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            // SAFETY: `d_name` is NUL-terminated within the entry.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            let bytes = name.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if let Ok(name) = std::str::from_utf8(bytes) {
                names.push(name.to_owned());
            }
        }
        // SAFETY: Closes the stream and the duplicate it owns.
        unsafe { libc::closedir(stream) };
        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            if let Some(entry) = self.stat(&name)? {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    pub fn stat(&self, name: &str) -> io::Result<Option<Entry>> {
        let c_name = cstr(name)?;
        // SAFETY: A zeroed `stat` is a valid out value.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: A valid directory descriptor, name and out pointer.
        let result =
            unsafe { libc::fstatat(self.raw(), c_name.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error);
        }
        Ok(Some(entry_from(name.to_owned(), &stat)))
    }

    /// Reads a regular file of at most `max` bytes. A link, a file still being downloaded or
    /// one larger than `max` is not read.
    pub fn read(&self, name: &str, max: u64) -> io::Result<Contents> {
        let c_name = cstr(name)?;
        // SAFETY: A valid directory descriptor and name.
        let fd = unsafe {
            libc::openat(
                self.raw(),
                c_name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(Contents::Missing);
            }
            if is_link_or_not_dir(&error) {
                return Ok(Contents::Unavailable);
            }
            return Err(error);
        }
        // SAFETY: A fresh descriptor that nothing else owns.
        let mut file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        // SAFETY: A zeroed `stat` is a valid out value.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: A valid descriptor and out pointer.
        check(unsafe { libc::fstat(file.as_raw_fd(), &mut stat) })?;
        let entry = entry_from(name.to_owned(), &stat);
        if entry.kind != Kind::File || entry.dataless || entry.len > max {
            return Ok(Contents::Unavailable);
        }
        let mut bytes = Vec::with_capacity(entry.len as usize);
        (&mut file).take(max + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max {
            return Ok(Contents::Unavailable);
        }
        Ok(Contents::Bytes(bytes))
    }

    /// Writes `bytes` to `name` in one step: staged in `staging`, which must be on the same
    /// volume, flushed, then renamed over `name`. A link at `name` is replaced, never followed.
    pub fn write_atomic(&self, name: &str, bytes: &[u8], staging: &Dir) -> io::Result<()> {
        let target = cstr(name)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(io::Error::other)?;
        let temp_name = format!("{}.tmp", hex(&random));
        let temp = cstr(&temp_name)?;
        // SAFETY: A valid directory descriptor and name; `O_EXCL` never opens an existing file.
        let fd = check(unsafe {
            libc::openat(
                staging.raw(),
                temp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_uint,
            )
        })?;
        // SAFETY: A fresh descriptor that nothing else owns.
        let mut file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        let renamed = written.and_then(|()| {
            // SAFETY: Valid descriptors and names.
            check(unsafe { libc::renameat(staging.raw(), temp.as_ptr(), self.raw(), target.as_ptr()) })
                .map(drop)
        });
        if let Err(error) = renamed {
            let _ = staging.remove(&temp_name);
            return Err(error);
        }
        self.sync()
    }

    /// Creates `name` with `bytes` only if nothing is there: staged, then linked into place,
    /// which fails if the name exists.
    pub fn create_new(&self, name: &str, bytes: &[u8], staging: &Dir) -> io::Result<()> {
        let target = cstr(name)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(io::Error::other)?;
        let temp_name = format!("{}.tmp", hex(&random));
        let temp = cstr(&temp_name)?;
        // SAFETY: As in `write_atomic`.
        let fd = check(unsafe {
            libc::openat(
                staging.raw(),
                temp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_uint,
            )
        })?;
        // SAFETY: A fresh descriptor that nothing else owns.
        let mut file = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        let linked = written.and_then(|()| {
            // SAFETY: Valid descriptors and names; no flags, so a link is not followed.
            check(unsafe { libc::linkat(staging.raw(), temp.as_ptr(), self.raw(), target.as_ptr(), 0) })
                .map(drop)
        });
        let _ = staging.remove(&temp_name);
        linked?;
        self.sync()
    }

    /// Removes a file or a link (never what it points to). A missing name is not an error.
    pub fn remove(&self, name: &str) -> io::Result<()> {
        let c_name = cstr(name)?;
        // SAFETY: A valid directory descriptor and name.
        let result = unsafe { libc::unlinkat(self.raw(), c_name.as_ptr(), 0) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error);
            }
        }
        Ok(())
    }

    /// Removes a directory and everything in it, never following a link out of it.
    pub fn remove_tree(&self, name: &str) -> io::Result<()> {
        if let Some(dir) = self.open_dir(name)? {
            for entry in dir.entries()? {
                match entry.kind {
                    Kind::Dir => dir.remove_tree(&entry.name)?,
                    Kind::File | Kind::Other => dir.remove(&entry.name)?,
                }
            }
        } else {
            return self.remove(name);
        }
        let c_name = cstr(name)?;
        // SAFETY: A valid directory descriptor and name.
        let result = unsafe { libc::unlinkat(self.raw(), c_name.as_ptr(), libc::AT_REMOVEDIR) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error);
            }
        }
        self.sync()
    }

    pub fn sync(&self) -> io::Result<()> {
        // SAFETY: A valid descriptor.
        check(unsafe { libc::fsync(self.raw()) }).map(drop)
    }

    /// The file system's type name, such as `apfs`, `smbfs` or `nfs`.
    #[cfg(target_os = "macos")]
    pub fn fs_type(&self) -> io::Result<String> {
        // SAFETY: A zeroed `statfs` is a valid out value.
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: A valid descriptor and out pointer.
        check(unsafe { libc::fstatfs(self.raw(), &mut stat) })?;
        // SAFETY: `f_fstypename` is NUL-terminated.
        let name = unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) };
        Ok(name.to_string_lossy().into_owned())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn fs_type(&self) -> io::Result<String> {
        Ok(String::new())
    }
}

fn entry_from(name: String, stat: &libc::stat) -> Entry {
    let kind = match stat.st_mode & libc::S_IFMT {
        libc::S_IFREG => Kind::File,
        libc::S_IFDIR => Kind::Dir,
        _ => Kind::Other,
    };
    #[cfg(target_os = "macos")]
    let dataless = stat.st_flags & SF_DATALESS != 0;
    #[cfg(not(target_os = "macos"))]
    let dataless = false;
    let (secs, nanos) = (stat.st_mtime, stat.st_mtime_nsec);
    let modified = UNIX_EPOCH
        + Duration::from_secs(u64::try_from(secs).unwrap_or(0))
        + Duration::from_nanos(u64::try_from(nanos).unwrap_or(0));
    Entry {
        name,
        kind,
        len: u64::try_from(stat.st_size).unwrap_or(0),
        modified,
        dataless,
    }
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0xf)] as char);
    }
    out
}

pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    if text.len() != N * 2 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (index, pair) in text.as_chunks::<2>().0.iter().enumerate() {
        out[index] = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Some(out)
}

/// Whether `name` is exactly `<2N lowercase hex digits><suffix>`: the folder's name grammar.
pub fn id_name<const N: usize>(name: &str, suffix: &str) -> Option<[u8; N]> {
    unhex::<N>(name.strip_suffix(suffix)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_never_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked-dir")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("linked-file")).unwrap();
        let dir = Dir::open_root(root.path()).unwrap();
        assert!(dir.open_dir("linked-dir").unwrap().is_none());
        assert!(dir.create_dir("linked-dir").is_err());
        assert!(matches!(
            dir.read("linked-file", 1 << 20).unwrap(),
            Contents::Unavailable
        ));
        let staging = dir.create_dir("tmp").unwrap();
        dir.write_atomic("linked-file", b"new", &staging).unwrap();
        assert_eq!(std::fs::read(outside.path().join("secret")).unwrap(), b"outside");
        dir.remove_tree("linked-dir").unwrap();
        assert!(
            outside.path().join("secret").exists(),
            "removing a link removes only the link"
        );
        let kinds: Vec<_> = dir
            .entries()
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.kind))
            .collect();
        assert!(kinds.contains(&("tmp".to_owned(), Kind::Dir)));
        assert!(Dir::open_root(&root.path().join("tmp/../linked-dir")).is_err());
    }

    #[test]
    fn creates_new_files_once_and_reads_within_a_limit() {
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open_root(root.path()).unwrap();
        let staging = dir.create_dir("tmp").unwrap();
        dir.create_new("a", b"first", &staging).unwrap();
        assert_eq!(
            dir.create_new("a", b"second", &staging).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(matches!(dir.read("a", 100).unwrap(), Contents::Bytes(b) if b == b"first"));
        assert!(matches!(dir.read("a", 3).unwrap(), Contents::Unavailable));
        assert!(matches!(dir.read("b", 3).unwrap(), Contents::Missing));
        assert!(staging.entries().unwrap().is_empty());
        assert!(dir.read("../x", 3).is_err());
    }

    #[test]
    fn the_name_grammar_is_exact() {
        assert!(id_name::<16>(&format!("{}.rec", "a".repeat(32)), ".rec").is_some());
        for bad in [
            format!("{}.rec", "A".repeat(32)),
            format!("{} 2.rec", "a".repeat(32)),
            format!("{}.REC", "a".repeat(32)),
            format!(".{}.rec.icloud", "a".repeat(32)),
            format!("{}.rec", "a".repeat(31)),
        ] {
            assert!(id_name::<16>(&bad, ".rec").is_none(), "{bad}");
        }
    }
}
