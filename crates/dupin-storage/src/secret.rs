use std::io;

use zeroize::Zeroizing;

/// A growable plaintext buffer that zeroizes every allocation it outgrows, where `Vec`'s own
/// growth would free the old one with the plaintext still in it.
pub struct SecretBuf(Zeroizing<Vec<u8>>);

impl SecretBuf {
    pub fn new() -> Self {
        Self(Zeroizing::new(Vec::with_capacity(256)))
    }

    pub fn into_inner(self) -> Zeroizing<Vec<u8>> {
        self.0
    }
}

impl io::Write for SecretBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let needed = self.0.len() + buf.len();
        if needed > self.0.capacity() {
            let mut grown = Zeroizing::new(Vec::with_capacity(needed.max(self.0.capacity() * 2)));
            grown.extend_from_slice(&self.0);
            self.0 = grown;
        }
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn to_json(value: &impl serde::Serialize) -> serde_json::Result<Zeroizing<Vec<u8>>> {
    let mut buf = SecretBuf::new();
    serde_json::to_writer(&mut buf, value)?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_without_losing_bytes() {
        let value: Vec<u32> = (0..10_000).collect();
        assert_eq!(*to_json(&value).unwrap(), serde_json::to_vec(&value).unwrap());
    }
}
