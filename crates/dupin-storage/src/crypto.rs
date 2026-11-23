//! Sealing: one XChaCha20-Poly1305 key and one blob-naming key per namespace, both derived
//! from the data key, and the associated data that binds every sealed piece to its place.

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::Error;
use crate::key::{DataKey, KEY_LEN};

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
/// The format version, written into every file and authenticated with it.
pub const VERSION: u8 = 1;

const SALT: &[u8] = b"dupin-storage v1";

pub struct NamespaceKeys {
    namespace: String,
    cipher: XChaCha20Poly1305,
    blob_mac: Zeroizing<[u8; KEY_LEN]>,
}

impl NamespaceKeys {
    pub fn derive(data_key: &DataKey, namespace: &str) -> Self {
        let hkdf = Hkdf::<Sha256>::new(Some(SALT), data_key.as_bytes());
        let mut seal = Zeroizing::new([0u8; KEY_LEN]);
        let mut blob_mac = Zeroizing::new([0u8; KEY_LEN]);
        hkdf.expand(&info("seal", namespace), seal.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        hkdf.expand(&info("blob-name", namespace), blob_mac.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        let cipher = XChaCha20Poly1305::new_from_slice(seal.as_ref()).expect("the key is 32 bytes");
        Self {
            namespace: namespace.to_owned(),
            cipher,
            blob_mac,
        }
    }

    /// The associated data for one sealed piece: its kind, the namespace and the names that
    /// place it (record ID, blob handle), each length-prefixed so no two lists collide.
    pub fn aad(&self, kind: &str, parts: &[&[u8]]) -> Vec<u8> {
        let mut aad = Vec::with_capacity(64);
        for part in [
            b"dupin-storage".as_slice(),
            &[VERSION],
            kind.as_bytes(),
            self.namespace.as_bytes(),
        ]
        .into_iter()
        .chain(parts.iter().copied())
        {
            aad.extend_from_slice(&(part.len() as u32).to_le_bytes());
            aad.extend_from_slice(part);
        }
        aad
    }

    /// A fresh random nonce and the ciphertext with its tag appended.
    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>), Error> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|error| Error::Io(std::io::Error::other(error)))?;
        // Room for the tag up front, so encrypting in place never reallocates and frees a
        // buffer that still holds plaintext.
        let mut buffer = Zeroizing::new(Vec::with_capacity(plaintext.len() + TAG_LEN));
        buffer.extend_from_slice(plaintext);
        self.cipher
            .encrypt_in_place(&XNonce::from(nonce), aad, &mut *buffer)
            .map_err(|_| Error::TooLarge {
                bytes: plaintext.len() as u64,
                limit: u64::MAX,
            })?;
        Ok((nonce, std::mem::take(&mut *buffer)))
    }

    pub fn open(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        ciphertext: Vec<u8>,
    ) -> Option<Zeroizing<Vec<u8>>> {
        let mut buffer = Zeroizing::new(ciphertext);
        self.cipher
            .decrypt_in_place(&XNonce::from(*nonce), aad, &mut *buffer)
            .ok()?;
        Some(buffer)
    }

    /// A blob's name: a keyed hash, so equal content shares one file without the name
    /// revealing to anyone without the key whether a guessed file is stored.
    pub fn blob_handle(&self, content: &[u8]) -> [u8; 32] {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(self.blob_mac.as_ref())
            .expect("HMAC takes any key length");
        mac.update(content);
        mac.finalize().into_bytes().into()
    }
}

fn info(purpose: &str, namespace: &str) -> Vec<u8> {
    let mut info = Vec::with_capacity(purpose.len() + namespace.len() + 1);
    info.extend_from_slice(purpose.as_bytes());
    info.push(0);
    info.extend_from_slice(namespace.as_bytes());
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(namespace: &str) -> NamespaceKeys {
        NamespaceKeys::derive(&DataKey::from_slice(&[7; 32]).unwrap(), namespace)
    }

    #[test]
    fn seals_and_opens() {
        let keys = keys("clipboard");
        let aad = keys.aad("record", &[b"id"]);
        let (nonce, ciphertext) = keys.seal(&aad, b"hello").unwrap();
        assert_ne!(&ciphertext[..5], b"hello");
        assert_eq!(&**keys.open(&nonce, &aad, ciphertext).unwrap(), b"hello");
    }

    #[test]
    fn nonces_are_fresh() {
        let keys = keys("clipboard");
        let (first, a) = keys.seal(b"", b"same").unwrap();
        let (second, b) = keys.seal(b"", b"same").unwrap();
        assert_ne!(first, second);
        assert_ne!(a, b);
    }

    #[test]
    fn rejects_other_places_and_namespaces() {
        let keys_a = keys("a");
        let aad = keys_a.aad("record", &[b"one"]);
        let (nonce, ciphertext) = keys_a.seal(&aad, b"secret").unwrap();
        assert!(
            keys_a
                .open(&nonce, &keys_a.aad("record", &[b"two"]), ciphertext.clone())
                .is_none()
        );
        assert!(
            keys_a
                .open(&nonce, &keys_a.aad("blob", &[b"one"]), ciphertext.clone())
                .is_none()
        );
        let keys_b = keys("b");
        assert!(
            keys_b
                .open(&nonce, &keys_b.aad("record", &[b"one"]), ciphertext)
                .is_none()
        );
    }

    #[test]
    fn length_prefixes_keep_parts_apart() {
        let keys = keys("n");
        assert_ne!(keys.aad("k", &[b"ab", b"c"]), keys.aad("k", &[b"a", b"bc"]));
    }

    #[test]
    fn blob_handles_depend_on_the_namespace() {
        assert_eq!(keys("a").blob_handle(b"x"), keys("a").blob_handle(b"x"));
        assert_ne!(keys("a").blob_handle(b"x"), keys("b").blob_handle(b"x"));
        assert_ne!(keys("a").blob_handle(b"x"), keys("a").blob_handle(b"y"));
    }
}
