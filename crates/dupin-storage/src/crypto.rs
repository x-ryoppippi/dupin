//! Sealing: the keys each namespace derives from the key that seals it, the key tag every
//! sealed file starts with, and the associated data that binds every sealed piece to its
//! identity rather than to its path.

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
pub const ID_LEN: usize = 16;
/// The format version, written into every file and authenticated with it.
pub const VERSION: u8 = 2;
pub const KEY_TAG_LEN: usize = 1 + ID_LEN + 4;

const SALT: &[u8] = b"dupin-storage v2";

/// The key a file is sealed under: this device's data key, or one epoch of a sync vault key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeySource {
    Device,
    Vault { id: [u8; ID_LEN], epoch: u32 },
}

impl KeySource {
    /// The plaintext tag a sealed file starts with: a kind byte, then the vault ID and epoch,
    /// which are zero for the device key.
    pub fn tag(&self) -> [u8; KEY_TAG_LEN] {
        let mut tag = [0u8; KEY_TAG_LEN];
        if let KeySource::Vault { id, epoch } = self {
            tag[0] = 1;
            tag[1..1 + ID_LEN].copy_from_slice(id);
            tag[1 + ID_LEN..].copy_from_slice(&epoch.to_le_bytes());
        }
        tag
    }

    pub fn from_tag(tag: &[u8; KEY_TAG_LEN]) -> Option<Self> {
        let id: [u8; ID_LEN] = tag[1..1 + ID_LEN].try_into().expect("tag layout");
        let epoch = u32::from_le_bytes(tag[1 + ID_LEN..].try_into().expect("tag layout"));
        match tag[0] {
            0 if id == [0; ID_LEN] && epoch == 0 => Some(KeySource::Device),
            1 if epoch >= 1 => Some(KeySource::Vault { id, epoch }),
            _ => None,
        }
    }
}

fn hkdf(key: &DataKey) -> Hkdf<Sha256> {
    Hkdf::<Sha256>::new(Some(SALT), key.as_bytes())
}

/// HKDF info that names the purpose, the key source and, for namespace keys, the namespace,
/// each length-prefixed so no two lists collide.
fn info(purpose: &str, source: &KeySource, namespace: Option<&str>) -> Vec<u8> {
    let tag = source.tag();
    let mut parts: Vec<&[u8]> = vec![purpose.as_bytes(), &tag];
    if let Some(namespace) = namespace {
        parts.push(namespace.as_bytes());
    }
    length_prefixed(&parts)
}

fn length_prefixed(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    for part in parts {
        out.extend_from_slice(&(part.len() as u32).to_le_bytes());
        out.extend_from_slice(part);
    }
    out
}

fn expand<const N: usize>(hkdf: &Hkdf<Sha256>, info: &[u8]) -> Zeroizing<[u8; N]> {
    let mut out = Zeroizing::new([0u8; N]);
    hkdf.expand(info, out.as_mut())
        .expect("16 and 32 bytes are valid HKDF-SHA256 output lengths");
    out
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(&length_prefixed(parts));
    mac.finalize().into_bytes().into()
}

/// One XChaCha20-Poly1305 cipher and the associated data that opens with it.
struct Sealer {
    cipher: XChaCha20Poly1305,
    /// Everything every piece sealed with this cipher authenticates: format, key tag and,
    /// for a namespace, its ID.
    common: Vec<Vec<u8>>,
}

impl Sealer {
    fn aad(&self, kind: &str, parts: &[&[u8]]) -> Vec<u8> {
        let mut all: Vec<&[u8]> = vec![b"dupin-storage", &[VERSION], kind.as_bytes()];
        all.extend(self.common.iter().map(Vec::as_slice));
        all.extend_from_slice(parts);
        length_prefixed(&all)
    }

    fn seal_with(&self, nonce: [u8; NONCE_LEN], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, Error> {
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
        Ok(std::mem::take(&mut *buffer))
    }

    fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>), Error> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|error| Error::Io(std::io::Error::other(error)))?;
        Ok((nonce, self.seal_with(nonce, aad, plaintext)?))
    }

    fn open(&self, nonce: &[u8; NONCE_LEN], aad: &[u8], ciphertext: Vec<u8>) -> Option<Zeroizing<Vec<u8>>> {
        let mut buffer = Zeroizing::new(ciphertext);
        self.cipher
            .decrypt_in_place(&XNonce::from(*nonce), aad, &mut *buffer)
            .ok()?;
        Some(buffer)
    }
}

fn cipher(key: &[u8; KEY_LEN]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new_from_slice(key).expect("the key is 32 bytes")
}

/// The keys of one namespace under one key source.
pub struct NamespaceKeys {
    source: KeySource,
    sealer: Sealer,
    blob_mac: Zeroizing<[u8; KEY_LEN]>,
    blob_nonce: Zeroizing<[u8; KEY_LEN]>,
    name: Zeroizing<[u8; KEY_LEN]>,
}

impl NamespaceKeys {
    /// `key` must be the key `source` names: the data key for [`KeySource::Device`].
    pub fn derive(key: &DataKey, source: KeySource, namespace: &str) -> Self {
        let hkdf = hkdf(key);
        let derive = |purpose| info(purpose, &source, Some(namespace));
        // The name a synced folder knows the namespace by, so it never shows the plain name.
        let namespace_id = expand::<ID_LEN>(&hkdf, &derive("namespace-id"));
        Self {
            source,
            sealer: Sealer {
                cipher: cipher(&expand::<KEY_LEN>(&hkdf, &derive("seal"))),
                common: vec![source.tag().to_vec(), namespace_id.to_vec()],
            },
            blob_mac: expand(&hkdf, &derive("blob-name")),
            blob_nonce: expand(&hkdf, &derive("blob-nonce")),
            name: expand(&hkdf, &derive("name")),
        }
    }

    pub fn source(&self) -> KeySource {
        self.source
    }

    pub fn aad(&self, kind: &str, parts: &[&[u8]]) -> Vec<u8> {
        self.sealer.aad(kind, parts)
    }

    /// A fresh random nonce and the ciphertext with its tag appended.
    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>), Error> {
        self.sealer.seal(aad, plaintext)
    }

    /// Seals a blob under a nonce derived from its handle, so every device that stores the
    /// same content writes the same bytes.
    pub fn seal_blob(
        &self,
        handle: &[u8; 32],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<([u8; NONCE_LEN], Vec<u8>), Error> {
        let nonce = self.blob_nonce(handle);
        Ok((nonce, self.sealer.seal_with(nonce, aad, plaintext)?))
    }

    pub fn blob_nonce(&self, handle: &[u8; 32]) -> [u8; NONCE_LEN] {
        let mac = hmac(self.blob_nonce.as_ref(), &[handle]);
        mac[..NONCE_LEN].try_into().expect("an HMAC-SHA256 is 32 bytes")
    }

    pub fn open(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        ciphertext: Vec<u8>,
    ) -> Option<Zeroizing<Vec<u8>>> {
        self.sealer.open(nonce, aad, ciphertext)
    }

    /// A blob's name: a keyed hash, so equal content shares one file without the name
    /// revealing to anyone without the key whether a guessed file is stored.
    pub fn blob_handle(&self, content: &[u8]) -> [u8; 32] {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(self.blob_mac.as_ref())
            .expect("HMAC takes any key length");
        mac.update(content);
        mac.finalize().into_bytes().into()
    }

    /// The record ID an owner's logical key maps to, so the key itself is never written.
    pub fn key_id(&self, key: &str) -> [u8; ID_LEN] {
        let mac = hmac(self.name.as_ref(), &[b"record-key", key.as_bytes()]);
        mac[..ID_LEN].try_into().expect("an HMAC-SHA256 is 32 bytes")
    }

    /// A tag the host vouches for `value` with, apart from every record ID, which an owner can
    /// name by any key it likes.
    pub fn host_tag(&self, purpose: &str, value: &str) -> [u8; 32] {
        hmac(
            self.name.as_ref(),
            &[b"host-tag", purpose.as_bytes(), value.as_bytes()],
        )
    }
}

/// The key that seals `storage.state`, which belongs to the device, not to a namespace.
pub struct StateKeys {
    sealer: Sealer,
}

impl StateKeys {
    pub fn derive(data_key: &DataKey) -> Self {
        let source = KeySource::Device;
        Self {
            sealer: Sealer {
                cipher: cipher(&expand::<KEY_LEN>(&hkdf(data_key), &info("state", &source, None))),
                common: vec![source.tag().to_vec()],
            },
        }
    }

    pub fn aad(&self) -> Vec<u8> {
        self.sealer.aad("state", &[])
    }

    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>), Error> {
        self.sealer.seal(aad, plaintext)
    }

    pub fn open(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        ciphertext: Vec<u8>,
    ) -> Option<Zeroizing<Vec<u8>>> {
        self.sealer.open(nonce, aad, ciphertext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VAULT: KeySource = KeySource::Vault {
        id: [3; ID_LEN],
        epoch: 1,
    };

    fn key() -> DataKey {
        DataKey::from_slice(&[7; 32]).unwrap()
    }

    fn keys(namespace: &str) -> NamespaceKeys {
        NamespaceKeys::derive(&key(), KeySource::Device, namespace)
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
    fn record_nonces_are_fresh() {
        let keys = keys("clipboard");
        let (first, a) = keys.seal(b"", b"same").unwrap();
        let (second, b) = keys.seal(b"", b"same").unwrap();
        assert_ne!(first, second);
        assert_ne!(a, b);
    }

    #[test]
    fn blob_ciphertext_is_deterministic() {
        let keys = keys("clipboard");
        let handle = keys.blob_handle(b"image");
        let aad = keys.aad("blob", &[&handle]);
        let first = keys.seal_blob(&handle, &aad, b"image").unwrap();
        assert_eq!(first, keys.seal_blob(&handle, &aad, b"image").unwrap());
        assert_eq!(
            &first.0[..],
            &hmac(keys.blob_nonce.as_ref(), &[&handle])[..NONCE_LEN]
        );
        let other = keys.blob_handle(b"other");
        assert_ne!(keys.blob_nonce(&handle), keys.blob_nonce(&other));
    }

    #[test]
    fn rejects_other_places_namespaces_and_key_sources() {
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
                .open(&nonce, &keys_b.aad("record", &[b"one"]), ciphertext.clone())
                .is_none()
        );
        let vault = NamespaceKeys::derive(&key(), VAULT, "a");
        assert!(
            vault
                .open(&nonce, &vault.aad("record", &[b"one"]), ciphertext)
                .is_none()
        );
    }

    #[test]
    fn length_prefixes_keep_parts_apart() {
        let keys = keys("n");
        assert_ne!(keys.aad("k", &[b"ab", b"c"]), keys.aad("k", &[b"a", b"bc"]));
    }

    #[test]
    fn names_depend_on_the_namespace_and_the_key_source() {
        assert_eq!(keys("a").blob_handle(b"x"), keys("a").blob_handle(b"x"));
        assert_ne!(keys("a").blob_handle(b"x"), keys("b").blob_handle(b"x"));
        assert_ne!(keys("a").blob_handle(b"x"), keys("a").blob_handle(b"y"));
        assert_eq!(keys("a").key_id("k"), keys("a").key_id("k"));
        assert_ne!(keys("a").key_id("k"), keys("b").key_id("k"));
        assert_ne!(keys("a").key_id("k"), keys("a").key_id("l"));
        let namespace_id = |keys: &NamespaceKeys| keys.sealer.common[1].clone();
        assert_ne!(namespace_id(&keys("a")), namespace_id(&keys("b")));
        let vault = NamespaceKeys::derive(&key(), VAULT, "a");
        assert_ne!(namespace_id(&vault), namespace_id(&keys("a")));
        assert_ne!(vault.key_id("k"), keys("a").key_id("k"));
        assert_ne!(vault.blob_handle(b"x"), keys("a").blob_handle(b"x"));
    }

    #[test]
    fn no_record_key_names_a_host_tag() {
        let a = keys("a");
        let tag = a.host_tag("file", "/x");
        assert_eq!(tag, a.host_tag("file", "/x"));
        assert_ne!(tag, a.host_tag("file", "/y"));
        assert_ne!(tag, a.host_tag("other", "/x"));
        assert_ne!(tag, keys("b").host_tag("file", "/x"));
        for key in ["file/x", "file\0/x", "host-tagfile/x"] {
            assert_ne!(tag[..ID_LEN], a.key_id(key));
        }
    }

    #[test]
    fn key_tags_round_trip_and_reject_malformed_ones() {
        for source in [KeySource::Device, VAULT] {
            assert_eq!(KeySource::from_tag(&source.tag()), Some(source));
        }
        let mut device_with_epoch = KeySource::Device.tag();
        device_with_epoch[KEY_TAG_LEN - 1] = 1;
        assert_eq!(KeySource::from_tag(&device_with_epoch), None);
        let mut vault_epoch_zero = VAULT.tag();
        vault_epoch_zero[1 + ID_LEN..].fill(0);
        assert_eq!(KeySource::from_tag(&vault_epoch_zero), None);
        let mut unknown = KeySource::Device.tag();
        unknown[0] = 9;
        assert_eq!(KeySource::from_tag(&unknown), None);
    }
}
