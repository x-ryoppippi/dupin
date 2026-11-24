//! Reading the gzip-compressed tar GitHub serves for a commit, checking every entry against the
//! commit's tree before it is written, so the staged files are exactly the commit's blobs.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read};

use crate::Error;
use crate::tree::{MAX_TOTAL_BYTES, Stage, check_path};

const BLOCK: usize = 512;
/// Room for the headers, padding and folders around the files, which the tree already capped.
const MAX_ARCHIVE_BYTES: u64 = MAX_TOTAL_BYTES + 8 * 1024 * 1024;
const MAX_PAX_BYTES: u64 = 64 * 1024;

/// A file of the commit, as its tree lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Blob {
    /// The Git blob id, SHA-1 in hex.
    pub id: String,
    pub size: u64,
    pub executable: bool,
}

/// Writes the files of `archive` into `stage`, below its one top-level folder. Each must be a
/// regular file of `expected` with the same size, mode and blob id, and every file of
/// `expected` must be there.
pub(crate) fn extract(
    archive: impl Read,
    expected: &BTreeMap<String, Blob>,
    stage: &Stage,
) -> Result<(), Error> {
    let mut reader = Capped {
        inner: flate2::read::GzDecoder::new(archive),
        left: MAX_ARCHIVE_BYTES,
    };
    let mut top: Option<String> = None;
    let mut pax = Pax::default();
    let mut seen = BTreeSet::new();
    loop {
        let mut header = [0u8; BLOCK];
        reader.fill(&mut header)?;
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        verify_checksum(&header)?;
        let kind = header[156];
        let size = octal(&header[124..136])?;
        if matches!(kind, b'x' | b'g') {
            if size > MAX_PAX_BYTES {
                return Err(invalid("has an extended header that is too large"));
            }
            let records = reader.take_bytes(size)?;
            if kind == b'x' {
                pax = Pax::parse(&records)?;
            }
            continue;
        }
        let name = match pax.path.take() {
            Some(path) => path,
            None => ustar_name(&header)?,
        };
        let size = pax.size.take().unwrap_or(size);
        let Some(path) = strip_top(&name, &mut top)? else {
            reader.skip(size)?;
            continue;
        };
        match kind {
            b'0' | 0 => {}
            b'5' => {
                check_path(&path)?;
                reader.skip(size)?;
                continue;
            }
            b'2' => {
                return Err(Error::new("invalid-package-path", "is a symbolic link")
                    .file(path)
                    .hint("Replace the link with the file it points to."));
            }
            b'1' => return Err(Error::new("invalid-package-path", "is a hard link").file(path)),
            _ => {
                return Err(
                    Error::new("invalid-package-path", "is a device, FIFO or other special file").file(path),
                );
            }
        }
        let Some(blob) = expected.get(&path) else {
            return Err(
                Error::new("invalid-archive", "is in GitHub's archive but not in the commit").file(path),
            );
        };
        let executable = octal(&header[100..108])? & 0o111 != 0;
        let differs = || {
            Error::new("invalid-archive", "differs from the commit in GitHub's archive").file(path.clone())
        };
        if blob.executable != executable || !seen.insert(path.clone()) {
            return Err(differs());
        }
        let rewritten = || {
            differs().hint("GitHub's archive applies `export-subst` and line-ending attributes from .gitattributes; drop them for the package's files.")
        };
        if blob.size != size {
            return Err(rewritten());
        }
        let bytes = reader.take_bytes(size)?;
        if blob_id(&bytes) != blob.id {
            return Err(rewritten());
        }
        stage.write(&path, &bytes, executable)?;
    }
    if let Some(path) = expected.keys().find(|path| !seen.contains(*path)) {
        return Err(Error::new("invalid-archive", "is missing from GitHub's archive")
            .file(path.clone())
            .hint("It is marked `export-ignore` in .gitattributes; Dupin installs every file of the commit, so drop the attribute or the file."));
    }
    Ok(())
}

/// The id Git gives a blob with these contents.
pub(crate) fn blob_id(bytes: &[u8]) -> String {
    let mut hasher = sha1_smol::Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hasher.digest().to_string()
}

fn invalid(message: &str) -> Error {
    Error::new("invalid-archive", format!("GitHub's archive {message}"))
}

/// The path below the archive's top-level folder, or `None` for that folder itself. Every
/// entry must sit in the same one.
fn strip_top(name: &str, top: &mut Option<String>) -> Result<Option<String>, Error> {
    let name = name.trim_end_matches('/');
    let (first, rest) = name.split_once('/').unwrap_or((name, ""));
    if first.is_empty() || first == "." || first == ".." {
        return Err(Error::new(
            "invalid-package-path",
            "must be a relative path inside the package",
        )
        .file(name));
    }
    match top {
        Some(top) if top != first => {
            return Err(invalid("has more than one top-level folder"));
        }
        Some(_) => {}
        None => *top = Some(first.to_owned()),
    }
    Ok((!rest.is_empty()).then(|| rest.to_owned()))
}

fn ustar_name(header: &[u8; BLOCK]) -> Result<String, Error> {
    let field = |bytes: &[u8]| {
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
        std::str::from_utf8(&bytes[..end])
            .map(str::to_owned)
            .map_err(|_| Error::new("invalid-package-path", "a file name isn't UTF-8"))
    };
    let name = field(&header[..100])?;
    let prefix = if &header[257..262] == b"ustar" {
        field(&header[345..500])?
    } else {
        String::new()
    };
    Ok(if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    })
}

fn octal(field: &[u8]) -> Result<u64, Error> {
    let text = std::str::from_utf8(field).map_err(|_| invalid("has a malformed header"))?;
    let text = text.trim_matches(|ch: char| ch == '\0' || ch == ' ');
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).map_err(|_| invalid("has a malformed header"))
}

fn verify_checksum(header: &[u8; BLOCK]) -> Result<(), Error> {
    let stored = octal(&header[148..156])?;
    let sum: u64 = header
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum();
    if sum == stored {
        Ok(())
    } else {
        Err(invalid("has a corrupt header"))
    }
}

/// The `path` and `size` an extended header sets for the entry after it.
#[derive(Default)]
struct Pax {
    path: Option<String>,
    size: Option<u64>,
}

impl Pax {
    fn parse(mut records: &[u8]) -> Result<Self, Error> {
        let malformed = || invalid("has a malformed extended header");
        let mut pax = Self::default();
        while !records.is_empty() {
            let space = records
                .iter()
                .position(|byte| *byte == b' ')
                .ok_or_else(malformed)?;
            let length: usize = std::str::from_utf8(&records[..space])
                .ok()
                .and_then(|length| length.parse().ok())
                .filter(|length| *length > space + 1 && *length <= records.len())
                .ok_or_else(malformed)?;
            let record = records[space + 1..length]
                .strip_suffix(b"\n")
                .ok_or_else(malformed)?;
            let equals = record
                .iter()
                .position(|byte| *byte == b'=')
                .ok_or_else(malformed)?;
            let value = &record[equals + 1..];
            match &record[..equals] {
                b"path" => {
                    pax.path = Some(
                        String::from_utf8(value.to_vec())
                            .map_err(|_| Error::new("invalid-package-path", "a file name isn't UTF-8"))?,
                    );
                }
                b"size" => {
                    pax.size = Some(
                        std::str::from_utf8(value)
                            .ok()
                            .and_then(|size| size.parse().ok())
                            .ok_or_else(malformed)?,
                    );
                }
                _ => {}
            }
            records = &records[length..];
        }
        Ok(pax)
    }
}

/// The decompressed stream, cut off past `MAX_ARCHIVE_BYTES` so a small download can't expand
/// into an unbounded amount of work.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Capped<R> {
    fn fill(&mut self, buffer: &mut [u8]) -> Result<(), Error> {
        let wanted = buffer.len() as u64;
        if wanted > self.left {
            return Err(Error::new(
                "package-too-large",
                format!(
                    "GitHub's archive unpacks to more than {} MiB",
                    MAX_ARCHIVE_BYTES / 1024 / 1024
                ),
            ));
        }
        self.left -= wanted;
        self.inner.read_exact(buffer).map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => invalid("ends early"),
            _ => invalid(&format!("can't be decompressed: {error}")),
        })
    }

    /// The `size` bytes of an entry, and the padding after them.
    fn take_bytes(&mut self, size: u64) -> Result<Vec<u8>, Error> {
        let padded = size.div_ceil(BLOCK as u64) * BLOCK as u64;
        if padded > self.left {
            return Err(Error::new(
                "package-too-large",
                format!(
                    "GitHub's archive unpacks to more than {} MiB",
                    MAX_ARCHIVE_BYTES / 1024 / 1024
                ),
            ));
        }
        let mut bytes = vec![0; usize::try_from(padded).map_err(|_| invalid("has a malformed header"))?];
        self.fill(&mut bytes)?;
        bytes.truncate(size as usize);
        Ok(bytes)
    }

    fn skip(&mut self, size: u64) -> Result<(), Error> {
        let mut left = size.div_ceil(BLOCK as u64);
        let mut block = [0u8; BLOCK];
        while left > 0 {
            self.fill(&mut block)?;
            left -= 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_ids_match_git() {
        assert_eq!(blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(blob_id(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[test]
    fn pax_records_set_the_next_path_and_size() {
        let pax = Pax::parse(b"20 path=a/long/name\n11 size=42\n").unwrap();
        assert_eq!(pax.path.as_deref(), Some("a/long/name"));
        assert_eq!(pax.size, Some(42));
        assert!(Pax::parse(b"99 path=x\n").is_err());
        assert!(Pax::parse(b"5 x\n").is_err());
    }

    #[test]
    fn one_top_level_folder_is_stripped() {
        let mut top = None;
        assert_eq!(strip_top("o-r-abc/", &mut top).unwrap(), None);
        assert_eq!(
            strip_top("o-r-abc/lib/a.js", &mut top).unwrap().as_deref(),
            Some("lib/a.js")
        );
        assert!(strip_top("other/a.js", &mut top).is_err());
        assert!(strip_top("/etc/passwd", &mut None).is_err());
        assert!(strip_top("../x", &mut None).is_err());
    }
}
