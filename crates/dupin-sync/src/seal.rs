//! The sealing the folder's own files use: the vault header's wrapped key and check, and the
//! device descriptors. Records and blobs are sealed by `dupin-storage`.

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use dupin_storage::VaultKey;

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
const SALT: &[u8] = b"dupin-sync v1";

pub fn length_prefixed(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    for part in parts {
        out.extend_from_slice(&(part.len() as u32).to_le_bytes());
        out.extend_from_slice(part);
    }
    out
}

/// A key for one purpose of one vault epoch.
pub fn derive(vault: &VaultKey, purpose: &str) -> Zeroizing<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(Some(SALT), vault.key().as_bytes());
    let id = vault.id();
    let epoch = vault.epoch().to_le_bytes();
    let info = length_prefixed(&[purpose.as_bytes(), &id, &epoch]);
    let mut out = Zeroizing::new([0u8; 32]);
    hkdf.expand(&info, out.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    out
}

fn cipher(key: &[u8; 32]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new_from_slice(key).expect("the key is 32 bytes")
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the system random source is available");
    bytes
}

/// Seals under a fresh random nonce; the ciphertext has its tag appended.
pub fn seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> ([u8; NONCE_LEN], Vec<u8>) {
    let nonce = random::<NONCE_LEN>();
    let mut buffer = Zeroizing::new(Vec::with_capacity(plaintext.len() + TAG_LEN));
    buffer.extend_from_slice(plaintext);
    cipher(key)
        .encrypt_in_place(&XNonce::from(nonce), aad, &mut *buffer)
        .expect("small plaintexts always seal");
    (nonce, std::mem::take(&mut *buffer))
}

/// Opens with the AEAD's constant-time tag check; `None` when anything was altered.
pub fn open(
    key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Option<Zeroizing<Vec<u8>>> {
    let mut buffer = Zeroizing::new(ciphertext.to_vec());
    cipher(key)
        .decrypt_in_place(&XNonce::from(*nonce), aad, &mut *buffer)
        .ok()?;
    Some(buffer)
}
