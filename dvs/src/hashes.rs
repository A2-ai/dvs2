use serde::{Deserialize, Serialize};
use std::fmt::Display;

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

impl From<Vec<u8>> for Hashes {
    fn from(bytes: Vec<u8>) -> Self {
        let blake3_hash = format!("{}", blake3::hash(&bytes));
        let md5_hash = format!("{:x}", md5::compute(&bytes));

        Self {
            blake3: blake3_hash,
            md5: md5_hash,
        }
    }
}

impl Hashes {
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
