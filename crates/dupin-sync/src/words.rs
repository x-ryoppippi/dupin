//! The word list generated passphrases are drawn from. It is data under CC BY 3.0 US, so it
//! ships as a file of its own beside the binary, which the host reads and hands in.

/// Words for passphrases, one per line, all distinct.
pub struct WordList(Vec<String>);

/// The EFF long list's size: six words from it give about 77 bits.
pub const WORDS: usize = 7776;

impl WordList {
    /// Checks a list the host read: exactly 7,776 distinct lowercase words.
    pub fn parse(text: &str) -> Result<Self, String> {
        let words: Vec<String> = text.lines().map(str::to_owned).collect();
        if words.len() != WORDS {
            return Err(format!(
                "the passphrase word list has {} words, not {WORDS}",
                words.len()
            ));
        }
        if words
            .iter()
            .any(|word| word.is_empty() || !word.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'))
        {
            return Err("the passphrase word list holds something other than words".into());
        }
        let mut distinct: Vec<&String> = words.iter().collect();
        distinct.sort();
        distinct.dedup();
        if distinct.len() != WORDS {
            return Err("the passphrase word list repeats a word".into());
        }
        Ok(Self(words))
    }

    pub(crate) fn get(&self, index: usize) -> &str {
        &self.0[index]
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

#[cfg(test)]
pub(crate) fn shipped() -> WordList {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/eff_large_wordlist.txt");
    WordList::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_list_is_the_eff_long_list() {
        let list = shipped();
        assert_eq!(list.len(), WORDS);
        assert_eq!(list.get(0), "abacus");
        assert_eq!(list.get(WORDS - 1), "zoom");
    }

    #[test]
    fn a_damaged_list_is_refused() {
        assert!(WordList::parse("one\ntwo").is_err());
        let repeated = "same\n".repeat(WORDS);
        assert!(WordList::parse(&repeated).is_err());
        let shouting: String = (0..WORDS).map(|i| format!("W{i}\n")).collect();
        assert!(WordList::parse(&shouting).is_err());
    }
}
