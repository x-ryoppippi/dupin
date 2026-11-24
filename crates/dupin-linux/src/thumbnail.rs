//! Thumbnails of the images extensions keep, which Dupin doesn't make on Linux yet.

use std::path::Path;

pub fn from_bytes(_data: &[u8], _fit: (u32, u32)) -> Option<Vec<u8>> {
    None
}

pub fn from_file(_path: &Path, _fit: (u32, u32)) -> Option<Vec<u8>> {
    None
}
