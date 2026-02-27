use std::fmt::Display;
use std::io::Read;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy, Hash)]
#[serde(rename_all = "lowercase")]
pub enum HashAlg {
    Blake3,
    Md5,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Hashes {
    pub blake3: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
}

impl Hashes {
    /// Stream-hash a file in one pass. Blake3 is always computed;
    /// `extra` controls which additional algorithms to include.
    /// Returns `(Hashes, file_size)`.
    pub fn compute_from_path(path: &Path, extra: &[HashAlg]) -> Result<(Self, u64)> {
        let mut file = fs_err::File::open(path)?;
        let mut size: u64 = 0;
        let mut buf = [0u8; 64 * 1024];

        let mut blake3_hasher = blake3::Hasher::new();
        let mut md5_context = if extra.contains(&HashAlg::Md5) {
            Some(md5::Context::new())
        } else {
            None
        };

        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            blake3_hasher.update(&buf[..n]);
            if let Some(c) = &mut md5_context {
                c.consume(&buf[..n]);
            }
        }

        Ok((
            Hashes {
                blake3: blake3_hasher.finalize().to_string(),
                md5: md5_context.map(|c| format!("{:x}", c.finalize())),
            },
            size,
        ))
    }

    pub fn get_blake3(&self) -> &str {
        self.blake3.as_str()
    }

    pub fn get_by_alg(&self, alg: HashAlg) -> Option<&str> {
        match alg {
            HashAlg::Blake3 => Some(&self.blake3),
            HashAlg::Md5 => self.md5.as_deref(),
        }
    }
}

impl Display for Hashes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.md5 {
            Some(m) => write!(f, "Hashes(md5={}, blake3={})", m, self.blake3),
            None => write!(f, "Hashes(blake3={})", self.blake3),
        }
    }
}
