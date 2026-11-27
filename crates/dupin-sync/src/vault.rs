//! The vault: one random key that seals everything that syncs, and `vault.dupin`, the one
//! plaintext file in the folder. The header names the vault and its key epoch, and, when a
//! passphrase protects the key, holds the key wrapped under an Argon2id key-encryption key. A
//! value sealed under the vault key authenticates every field, so a Mac that holds the key
//! detects any change to the header.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use dupin_storage::{DataKey, VaultKey};

use crate::error::Error;
use crate::fsx::{hex, unhex};
use crate::keys::SharedKeyStore;
use crate::passphrase::Passphrase;
use crate::seal::{self, NONCE_LEN, TAG_LEN, length_prefixed};

const MAGIC: &[u8; 4] = b"DUPV";
pub const FORMAT: u8 = 1;
pub const HEADER_NAME: &str = "vault.dupin";
/// The most a header may be: it is small, and a tampered one must not make Dupin read much.
pub const MAX_HEADER_BYTES: u64 = 16 * 1024;
const CHECK_PLAINTEXT: &[u8] = b"dupin vault check";
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;

/// Argon2id cost, as stored in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

impl KdfParams {
    /// About half a second on an Apple silicon Mac, once per Mac.
    pub const DEFAULT: KdfParams = KdfParams {
        memory_kib: 256 * 1024,
        iterations: 3,
        parallelism: 4,
    };
    /// Joining refuses anything cheaper, so a tampered header can't downgrade the derivation.
    pub(crate) const FLOOR: KdfParams = KdfParams {
        memory_kib: 64 * 1024,
        iterations: 3,
        parallelism: 1,
    };
    /// And anything this expensive, so a tampered header can't exhaust memory or time.
    const CEILING: KdfParams = KdfParams {
        memory_kib: 1024 * 1024,
        iterations: 16,
        parallelism: 16,
    };

    fn check(&self) -> Result<(), Error> {
        let refuse = |why: String| Err(Error::UnacceptableKdf(why));
        if self.memory_kib < Self::FLOOR.memory_kib || self.iterations < Self::FLOOR.iterations {
            return refuse(format!(
                "m = {} KiB, t = {} is below m = {} KiB, t = {}",
                self.memory_kib,
                self.iterations,
                Self::FLOOR.memory_kib,
                Self::FLOOR.iterations
            ));
        }
        if self.memory_kib > Self::CEILING.memory_kib
            || self.iterations > Self::CEILING.iterations
            || !(Self::FLOOR.parallelism..=Self::CEILING.parallelism).contains(&self.parallelism)
        {
            return refuse(format!(
                "m = {} KiB, t = {}, p = {} is beyond what Dupin accepts",
                self.memory_kib, self.iterations, self.parallelism
            ));
        }
        Ok(())
    }

    fn derive(
        &self,
        passphrase: &Passphrase,
        salt: &[u8; SALT_LEN],
    ) -> Result<Zeroizing<[u8; KEY_LEN]>, Error> {
        self.check()?;
        let params = argon2::Params::new(self.memory_kib, self.iterations, self.parallelism, Some(KEY_LEN))
            .map_err(|error| Error::UnacceptableKdf(error.to_string()))?;
        let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut out = Zeroizing::new([0u8; KEY_LEN]);
        argon
            .hash_password_into(passphrase.expose().as_bytes(), salt, out.as_mut())
            .map_err(|error| Error::UnacceptableKdf(error.to_string()))?;
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyMode {
    /// The vault key is wrapped under a passphrase in the header.
    Passphrase,
    /// The vault key travels as a synchronizable iCloud Keychain item. Dupin always also
    /// writes a passphrase-wrapped copy, since it can't tell whether iCloud Keychain is on.
    Keychain,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KdfJson {
    algorithm: String,
    version: u32,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedJson {
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeaderJson {
    vault: String,
    epoch: u32,
    mode: KeyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kdf: Option<KdfJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wrapped: Option<SealedJson>,
    check: SealedJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Sealed {
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

impl Sealed {
    fn parse(json: &SealedJson, plaintext_len: usize) -> Option<Self> {
        let nonce = unhex::<NONCE_LEN>(&json.nonce)?;
        let ciphertext = parse_hex_vec(&json.ciphertext)?;
        (ciphertext.len() == plaintext_len + TAG_LEN).then_some(Self { nonce, ciphertext })
    }

    fn json(&self) -> SealedJson {
        SealedJson {
            nonce: hex(&self.nonce),
            ciphertext: hex(&self.ciphertext),
        }
    }
}

fn parse_hex_vec(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() > 2 * 1024 {
        return None;
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| unhex::<1>(std::str::from_utf8(pair).ok()?).map(|[byte]| byte))
        .collect()
}

/// A parsed `vault.dupin`. Parsing checks its shape; only [`VaultHeader::verify`] with the key,
/// or unwrapping with the passphrase, proves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultHeader {
    pub(crate) id: [u8; 16],
    pub(crate) epoch: u32,
    pub(crate) mode: KeyMode,
    kdf: Option<(KdfParams, [u8; SALT_LEN])>,
    wrapped: Option<Sealed>,
    check: Sealed,
}

impl VaultHeader {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let bad = |why: &str| Error::BadHeader(why.to_owned());
        if bytes.len() as u64 > MAX_HEADER_BYTES {
            return Err(bad("too large"));
        }
        let rest = bytes
            .strip_prefix(MAGIC.as_slice())
            .ok_or_else(|| bad("not a vault header"))?;
        let (&format, json) = rest.split_first().ok_or_else(|| bad("truncated"))?;
        if format > FORMAT {
            return Err(Error::NewerFormat);
        }
        if format != FORMAT {
            return Err(bad("unknown format"));
        }
        let json: HeaderJson = serde_json::from_slice(json).map_err(|_| bad("malformed"))?;
        let id = unhex::<16>(&json.vault).ok_or_else(|| bad("vault ID"))?;
        if json.epoch == 0 {
            return Err(bad("epoch 0"));
        }
        let kdf = match &json.kdf {
            None => None,
            Some(kdf) => {
                if kdf.algorithm != "argon2id" || kdf.version != 0x13 {
                    return Err(Error::UnacceptableKdf(format!(
                        "{} version {} is not Argon2id 1.3",
                        kdf.algorithm, kdf.version
                    )));
                }
                let salt = unhex::<SALT_LEN>(&kdf.salt).ok_or_else(|| bad("salt"))?;
                Some((
                    KdfParams {
                        memory_kib: kdf.memory_kib,
                        iterations: kdf.iterations,
                        parallelism: kdf.parallelism,
                    },
                    salt,
                ))
            }
        };
        let wrapped = match &json.wrapped {
            None => None,
            Some(sealed) => Some(Sealed::parse(sealed, KEY_LEN).ok_or_else(|| bad("wrapped key"))?),
        };
        if kdf.is_some() != wrapped.is_some() {
            return Err(bad(
                "a wrapped key needs its key derivation and the other way round",
            ));
        }
        if json.mode == KeyMode::Passphrase && wrapped.is_none() {
            return Err(bad("passphrase mode without a wrapped key"));
        }
        let check = Sealed::parse(&json.check, CHECK_PLAINTEXT.len()).ok_or_else(|| bad("check"))?;
        Ok(Self {
            id,
            epoch: json.epoch,
            mode: json.mode,
            kdf,
            wrapped,
            check,
        })
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let json = HeaderJson {
            vault: hex(&self.id),
            epoch: self.epoch,
            mode: self.mode,
            kdf: self.kdf.map(|(params, salt)| KdfJson {
                algorithm: "argon2id".into(),
                version: 0x13,
                memory_kib: params.memory_kib,
                iterations: params.iterations,
                parallelism: params.parallelism,
                salt: hex(&salt),
            }),
            wrapped: self.wrapped.as_ref().map(Sealed::json),
            check: self.check.json(),
        };
        let mut out = MAGIC.to_vec();
        out.push(FORMAT);
        out.extend_from_slice(&serde_json::to_vec_pretty(&json).expect("the header serializes"));
        out.push(b'\n');
        out
    }

    #[cfg(test)]
    pub(crate) fn has_passphrase(&self) -> bool {
        self.wrapped.is_some()
    }

    /// Every field but the sealed values themselves, length-prefixed.
    fn binding(&self, purpose: &str) -> Vec<u8> {
        let epoch = self.epoch.to_le_bytes();
        let mode: &[u8] = match self.mode {
            KeyMode::Passphrase => b"passphrase",
            KeyMode::Keychain => b"keychain",
        };
        let kdf = self.kdf.map(|(params, salt)| {
            let mut out = Vec::with_capacity(12 + SALT_LEN);
            out.extend_from_slice(&params.memory_kib.to_le_bytes());
            out.extend_from_slice(&params.iterations.to_le_bytes());
            out.extend_from_slice(&params.parallelism.to_le_bytes());
            out.extend_from_slice(&salt);
            out
        });
        let mut parts: Vec<&[u8]> = vec![
            b"dupin-vault",
            &[FORMAT],
            purpose.as_bytes(),
            &self.id,
            &epoch,
            mode,
            kdf.as_deref().unwrap_or_default(),
        ];
        let wrapped;
        if purpose == "check" {
            wrapped = self
                .wrapped
                .as_ref()
                .map(|sealed| [sealed.nonce.as_slice(), &sealed.ciphertext].concat());
            parts.push(wrapped.as_deref().unwrap_or_default());
        }
        length_prefixed(&parts)
    }

    /// Whether `key` is this vault's key: the sealed check, which authenticates every field of
    /// the header, opens under it.
    pub(crate) fn verify(&self, key: &VaultKey) -> bool {
        if key.id() != self.id || key.epoch() != self.epoch {
            return false;
        }
        let check_key = seal::derive(key, "vault-check");
        seal::open(
            &check_key,
            &self.check.nonce,
            &self.binding("check"),
            &self.check.ciphertext,
        )
        .is_some_and(|plain| plain.as_slice() == CHECK_PLAINTEXT)
    }

    /// Unwraps the vault key with the passphrase. The derivation's cost is checked against
    /// [`KdfParams::FLOOR`] and [`KdfParams::CEILING`] first, and the unwrapped key must also
    /// open the header's check.
    fn unwrap(&self, passphrase: &Passphrase) -> Result<VaultKey, Error> {
        let (Some((params, salt)), Some(wrapped)) = (self.kdf, &self.wrapped) else {
            return Err(Error::KeyUnavailable);
        };
        let kek = params.derive(passphrase, &salt)?;
        let plain = seal::open(&kek, &wrapped.nonce, &self.binding("wrap"), &wrapped.ciphertext)
            .ok_or(Error::WrongPassphrase)?;
        let key = VaultKey::new(self.id, self.epoch, DataKey::from_slice(&plain)?)?;
        if !self.verify(&key) {
            return Err(Error::BadHeader(
                "the check doesn't open under the wrapped key".into(),
            ));
        }
        Ok(key)
    }

    /// A new header for `key`, with the key wrapped under `passphrase` when one is given.
    pub(crate) fn new(
        key: &VaultKey,
        mode: KeyMode,
        passphrase: Option<(&Passphrase, KdfParams)>,
    ) -> Result<Self, Error> {
        if mode == KeyMode::Passphrase && passphrase.is_none() {
            return Err(Error::KeyUnavailable);
        }
        let mut header = Self {
            id: key.id(),
            epoch: key.epoch(),
            mode,
            kdf: None,
            wrapped: None,
            check: Sealed {
                nonce: [0; NONCE_LEN],
                ciphertext: Vec::new(),
            },
        };
        if let Some((passphrase, params)) = passphrase {
            let salt = seal::random::<SALT_LEN>();
            header.kdf = Some((params, salt));
            let kek = params.derive(passphrase, &salt)?;
            let (nonce, ciphertext) = seal::seal(&kek, &header.binding("wrap"), key.key().as_bytes());
            header.wrapped = Some(Sealed { nonce, ciphertext });
        }
        let check_key = seal::derive(key, "vault-check");
        let (nonce, ciphertext) = seal::seal(&check_key, &header.binding("check"), CHECK_PLAINTEXT);
        header.check = Sealed { nonce, ciphertext };
        Ok(header)
    }
}

/// A new random vault key for epoch 1.
pub fn new_vault_key() -> Result<VaultKey, Error> {
    Ok(VaultKey::new(seal::random::<16>(), 1, DataKey::generate()?)?)
}

/// Picks the header of the vault that wins among competing ones, the lowest vault ID, and
/// opens it with the shared Keychain item when this Mac has it, otherwise with `passphrase`.
pub fn open_winner(
    headers: &[VaultHeader],
    passphrase: Option<&Passphrase>,
    shared: Option<&dyn SharedKeyStore>,
) -> Result<(VaultKey, VaultHeader), Error> {
    let winner = headers
        .iter()
        .map(|header| header.id)
        .min()
        .ok_or(Error::NoVault)?;
    let mut candidates: Vec<&VaultHeader> = headers.iter().filter(|header| header.id == winner).collect();
    candidates.sort_by_key(|header| std::cmp::Reverse(header.epoch));
    let mut last = Error::KeyUnavailable;
    for header in candidates {
        if let Some(shared) = shared
            && let Some(key) = shared.load(header.id, header.epoch)?
        {
            let key = VaultKey::new(header.id, header.epoch, key)?;
            if header.verify(&key) {
                return Ok((key, header.clone()));
            }
        }
        if let Some(passphrase) = passphrase {
            match header.unwrap(passphrase) {
                Ok(key) => return Ok((key, header.clone())),
                Err(error) => last = error,
            }
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: KdfParams = KdfParams::FLOOR;

    fn passphrase() -> Passphrase {
        Passphrase::entered("correct horse battery staple")
    }

    #[test]
    fn a_header_round_trips_and_unwraps_with_the_passphrase() {
        let key = new_vault_key().unwrap();
        let header = VaultHeader::new(&key, KeyMode::Passphrase, Some((&passphrase(), FAST))).unwrap();
        let parsed = VaultHeader::parse(&header.encode()).unwrap();
        assert_eq!(parsed, header);
        assert!(parsed.verify(&key));
        let unwrapped = parsed.unwrap(&passphrase()).unwrap();
        assert_eq!(unwrapped.key().as_bytes(), key.key().as_bytes());
        assert!(matches!(
            parsed.unwrap(&Passphrase::entered("wrong horse battery staple")),
            Err(Error::WrongPassphrase)
        ));
    }

    #[test]
    fn the_default_cost_is_the_designed_one() {
        assert_eq!(
            KdfParams::DEFAULT,
            KdfParams {
                memory_kib: 262_144,
                iterations: 3,
                parallelism: 4
            }
        );
        assert!(KdfParams::DEFAULT.check().is_ok());
    }

    #[test]
    fn a_downgraded_or_inflated_kdf_is_refused_before_deriving() {
        let key = new_vault_key().unwrap();
        let header = VaultHeader::new(&key, KeyMode::Passphrase, Some((&passphrase(), FAST))).unwrap();
        let text = String::from_utf8(header.encode()[5..].to_vec()).unwrap();
        for (from, to) in [
            ("\"memory_kib\": 65536", "\"memory_kib\": 8"),
            ("\"iterations\": 3", "\"iterations\": 1"),
            ("\"memory_kib\": 65536", "\"memory_kib\": 4294967295"),
            ("\"parallelism\": 1", "\"parallelism\": 0"),
            ("\"argon2id\"", "\"argon2i\""),
        ] {
            assert!(text.contains(from), "{from}");
            let mut bytes = b"DUPV\x01".to_vec();
            bytes.extend_from_slice(text.replace(from, to).as_bytes());
            let result = VaultHeader::parse(&bytes).and_then(|parsed| parsed.unwrap(&passphrase()));
            assert!(
                matches!(result, Err(Error::UnacceptableKdf(_))),
                "{to}: {result:?}"
            );
        }
    }

    #[test]
    fn any_altered_field_fails_the_check() {
        let key = new_vault_key().unwrap();
        let header = VaultHeader::new(&key, KeyMode::Keychain, Some((&passphrase(), FAST))).unwrap();
        let text = String::from_utf8(header.encode()[5..].to_vec()).unwrap();
        for (from, to) in [
            ("\"keychain\"", "\"passphrase\""),
            ("\"epoch\": 1", "\"epoch\": 2"),
        ] {
            let mut bytes = b"DUPV\x01".to_vec();
            bytes.extend_from_slice(text.replace(from, to).as_bytes());
            let parsed = VaultHeader::parse(&bytes).unwrap();
            assert!(!parsed.verify(&key), "{to}");
        }
        let mut salted = header.clone();
        salted.kdf.as_mut().unwrap().1[0] ^= 1;
        assert!(!salted.verify(&key));
        assert!(salted.unwrap(&passphrase()).is_err());
    }

    #[test]
    fn malformed_headers_are_refused() {
        let key = new_vault_key().unwrap();
        let good = VaultHeader::new(&key, KeyMode::Passphrase, Some((&passphrase(), FAST)))
            .unwrap()
            .encode();
        let mut newer = good.clone();
        newer[4] = FORMAT + 1;
        assert!(matches!(VaultHeader::parse(&newer), Err(Error::NewerFormat)));
        for bad in [
            &b""[..],
            b"DUPV",
            b"DUPV\x01{}",
            b"DUPV\x01not json",
            &good[..good.len() / 2],
            &[b'X'; 20][..],
        ] {
            assert!(matches!(VaultHeader::parse(bad), Err(Error::BadHeader(_))));
        }
        let text = String::from_utf8(good[5..].to_vec()).unwrap();
        let extra = text.replacen('{', "{\"extra\": 1,", 1);
        let mut bytes = b"DUPV\x01".to_vec();
        bytes.extend_from_slice(extra.as_bytes());
        assert!(VaultHeader::parse(&bytes).is_err(), "unknown fields are refused");
        assert!(VaultHeader::parse(&vec![b' '; MAX_HEADER_BYTES as usize + 1]).is_err());
    }

    #[test]
    fn the_lowest_vault_id_wins_and_its_key_opens_it() {
        let low = VaultKey::new([1; 16], 1, DataKey::generate().unwrap()).unwrap();
        let high = VaultKey::new([2; 16], 1, DataKey::generate().unwrap()).unwrap();
        let headers = [
            VaultHeader::new(&high, KeyMode::Passphrase, Some((&passphrase(), FAST))).unwrap(),
            VaultHeader::new(&low, KeyMode::Passphrase, Some((&passphrase(), FAST))).unwrap(),
        ];
        let (key, header) = open_winner(&headers, Some(&passphrase()), None).unwrap();
        assert_eq!(key.id(), [1; 16]);
        assert_eq!(header.id, [1; 16]);
        assert!(matches!(open_winner(&[], None, None), Err(Error::NoVault)));
        assert!(matches!(
            open_winner(&headers, None, None),
            Err(Error::KeyUnavailable)
        ));
    }
}
