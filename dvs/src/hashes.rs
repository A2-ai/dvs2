use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::io::BufRead;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum HashAlg {
    Blake3,
    Md5,
}

/// By default, blake3 is used locally but for example AWS/Azure automatically computes
/// MD5 so it makes sense to use MD5 for those.
/// We compute both so we can easily switch backends if needed.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Hashes {
    pub blake3: String,
    pub md5: String,
}

impl Hashes {
    pub fn from_reader<R: BufRead>(mut reader: R) -> std::io::Result<Self> {
        let mut blake3_hasher = blake3::Hasher::new();
        let mut md5_context = md5::Context::new();

        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            blake3_hasher.update(buf);
            md5_context.consume(buf);
            let len = buf.len();
            reader.consume(len);
        }

        Ok(Self {
            blake3: blake3_hasher.finalize().to_string(),
            md5: format!("{:x}", md5_context.finalize()),
        })
    }

    pub fn get_by_alg(&self, alg: HashAlg) -> &str {
        match alg {
            HashAlg::Blake3 => &self.blake3,
            HashAlg::Md5 => &self.md5,
        }
    }
}

impl Display for Hashes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Hashes(md5={}, blake3={})", self.md5, self.blake3)
    }
}
