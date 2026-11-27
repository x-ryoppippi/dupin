//! Recovery passphrases: generated from the EFF long word list, or chosen by the user and
//! checked against a bundled list of common passwords.

use std::fmt;

use zeroize::Zeroizing;

use crate::words::WordList;

/// Passwords of at least the minimum length from a list of the most used ones, lowercased.
const COMMON: &str = include_str!("../data/common_passwords.txt");
pub const GENERATED_WORDS: usize = 6;
pub const MIN_CHARS: usize = 12;

/// A passphrase, zeroized when dropped.
#[derive(Clone)]
pub struct Passphrase {
    text: Zeroizing<String>,
    /// Generated, or chosen and checked: fit to protect a new vault.
    checked: bool,
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weakness {
    TooShort,
    Common,
}

impl fmt::Display for Weakness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Weakness::TooShort => write!(f, "use at least {MIN_CHARS} characters"),
            Weakness::Common => f.write_str("this is a commonly used password; choose another"),
        }
    }
}

impl Passphrase {
    /// Six words from the EFF long list (7,776 words), about 77 bits.
    pub fn generate(words: &WordList) -> Result<Self, getrandom::Error> {
        let mut chosen = Zeroizing::new(String::with_capacity(64));
        for index in 0..GENERATED_WORDS {
            if index > 0 {
                chosen.push(' ');
            }
            chosen.push_str(words.get(uniform(words.len() as u32)? as usize));
        }
        Ok(Self {
            text: chosen,
            checked: true,
        })
    }

    /// A passphrase the user chose, refused when it is short or common.
    pub fn chosen(text: &str) -> Result<Self, Weakness> {
        if text.chars().count() < MIN_CHARS {
            return Err(Weakness::TooShort);
        }
        let lowered = Zeroizing::new(text.to_lowercase());
        if COMMON.lines().any(|common| common == lowered.as_str()) {
            return Err(Weakness::Common);
        }
        Ok(Self {
            text: Zeroizing::new(text.to_owned()),
            checked: true,
        })
    }

    /// A passphrase typed to unlock an existing vault: never judged, since it is already set.
    pub fn entered(text: &str) -> Self {
        Self {
            text: Zeroizing::new(text.to_owned()),
            checked: false,
        }
    }

    pub fn expose(&self) -> &str {
        &self.text
    }

    pub(crate) fn is_checked(&self) -> bool {
        self.checked
    }
}

/// A uniform value below `bound`, by rejection sampling.
fn uniform(bound: u32) -> Result<u32, getrandom::Error> {
    let zone = u32::MAX - (u32::MAX % bound);
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes)?;
        let value = u32::from_le_bytes(bytes);
        if value < zone {
            return Ok(value % bound);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_passphrases_are_six_listed_words() {
        let list = crate::words::shipped();
        let words: std::collections::HashSet<&str> = (0..list.len()).map(|i| list.get(i)).collect();
        let first = Passphrase::generate(&list).unwrap();
        let parts: Vec<&str> = first.expose().split(' ').collect();
        assert_eq!(parts.len(), GENERATED_WORDS);
        assert!(parts.iter().all(|word| words.contains(word)));
        assert_ne!(first.expose(), Passphrase::generate(&list).unwrap().expose());
        assert!(Passphrase::chosen(first.expose()).is_ok());
    }

    #[test]
    fn chosen_passphrases_must_be_long_and_uncommon() {
        assert_eq!(Passphrase::chosen("short").unwrap_err(), Weakness::TooShort);
        assert_eq!(Passphrase::chosen("ééééééééééé").unwrap_err(), Weakness::TooShort);
        assert_eq!(Passphrase::chosen("qwerty123456").unwrap_err(), Weakness::Common);
        assert_eq!(Passphrase::chosen("QWERTY123456").unwrap_err(), Weakness::Common);
        assert!(Passphrase::chosen("a sentence nobody else uses").is_ok());
        assert_eq!(format!("{:?}", Passphrase::entered("secret")), "Passphrase(..)");
    }
}
