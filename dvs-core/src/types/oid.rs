//! Object ID (content hash with algorithm prefix).

use serde::{Deserialize, Serialize, Serializer, Deserializer};
use std::fmt;
use std::str::FromStr;

/// Hash algorithm for content identification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashAlgo {
    /// BLAKE3 hash (64 hex chars).
    Blake3,
    /// SHA-256 hash (64 hex chars).
    Sha256,
    /// XXH3 hash (16 hex chars) - fast non-cryptographic.
    Xxh3,
}

impl HashAlgo {
    /// Get the algorithm prefix string.
    pub fn prefix(&self) -> &'static str {
        match self {
            HashAlgo::Blake3 => "blake3",
            HashAlgo::Sha256 => "sha256",
            HashAlgo::Xxh3 => "xxh3",
        }
    }

    /// Parse algorithm from prefix string.
    pub fn from_prefix(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "blake3" => Some(HashAlgo::Blake3),
            "sha256" => Some(HashAlgo::Sha256),
            "xxh3" => Some(HashAlgo::Xxh3),
            _ => None,
        }
    }

    /// Expected hex length for this algorithm.
    pub fn hex_len(&self) -> usize {
        match self {
            HashAlgo::Blake3 => 64,
            HashAlgo::Sha256 => 64,
            HashAlgo::Xxh3 => 16,
        }
    }
}

impl fmt::Display for HashAlgo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.prefix())
    }
}

impl FromStr for HashAlgo {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        HashAlgo::from_prefix(s).ok_or_else(|| format!("Unknown hash algorithm: {}", s))
    }
}

/// Object ID with algorithm prefix.
///
/// Format: `{algo}:{hex}` (e.g., `blake3:abc123...`)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Oid {
    /// Hash algorithm.
    pub algo: HashAlgo,
    /// Hex-encoded hash value.
    pub hex: String,
}

impl Oid {
    /// Create a new OID.
    pub fn new(algo: HashAlgo, hex: String) -> Self {
        Self { algo, hex }
    }

    /// Create a BLAKE3 OID from a hex string.
    pub fn blake3(hex: String) -> Self {
        Self::new(HashAlgo::Blake3, hex)
    }

    /// Create a SHA-256 OID from a hex string.
    pub fn sha256(hex: String) -> Self {
        Self::new(HashAlgo::Sha256, hex)
    }

    /// Get the storage path components: (prefix, suffix).
    ///
    /// Returns first 2 chars as prefix and rest as suffix for content-addressable storage.
    pub fn storage_path_components(&self) -> (&str, &str) {
        let split_at = 2.min(self.hex.len());
        self.hex.split_at(split_at)
    }

    /// Get the full storage subpath: `{algo}/{prefix}/{suffix}`.
    pub fn storage_subpath(&self) -> String {
        let (prefix, suffix) = self.storage_path_components();
        format!("{}/{}/{}", self.algo.prefix(), prefix, suffix)
    }

    /// Parse an OID from string format `algo:hex`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid OID format (expected algo:hex): {}", s));
        }

        let algo = HashAlgo::from_prefix(parts[0])
            .ok_or_else(|| format!("Unknown hash algorithm: {}", parts[0]))?;
        let hex = parts[1].to_string();

        // Validate hex length
        if hex.len() != algo.hex_len() {
            return Err(format!(
                "Invalid hex length for {}: expected {}, got {}",
                algo,
                algo.hex_len(),
                hex.len()
            ));
        }

        // Validate hex characters
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("Invalid hex characters in OID: {}", hex));
        }

        Ok(Self { algo, hex })
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algo, self.hex)
    }
}

impl FromStr for Oid {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Oid::parse(s)
    }
}

// Custom serde implementation to serialize as "algo:hex" string
impl Serialize for Oid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Oid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Oid::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_algo_prefix() {
        assert_eq!(HashAlgo::Blake3.prefix(), "blake3");
        assert_eq!(HashAlgo::Sha256.prefix(), "sha256");
        assert_eq!(HashAlgo::Xxh3.prefix(), "xxh3");
    }

    #[test]
    fn test_hash_algo_from_prefix() {
        assert_eq!(HashAlgo::from_prefix("blake3"), Some(HashAlgo::Blake3));
        assert_eq!(HashAlgo::from_prefix("BLAKE3"), Some(HashAlgo::Blake3));
        assert_eq!(HashAlgo::from_prefix("unknown"), None);
    }

    #[test]
    fn test_oid_new() {
        let oid = Oid::blake3("a".repeat(64));
        assert_eq!(oid.algo, HashAlgo::Blake3);
        assert_eq!(oid.hex.len(), 64);
    }

    #[test]
    fn test_oid_display() {
        let oid = Oid::blake3("a".repeat(64));
        let s = oid.to_string();
        assert!(s.starts_with("blake3:"));
        assert_eq!(s.len(), 7 + 64); // "blake3:" + 64 hex chars
    }

    #[test]
    fn test_oid_parse() {
        let hex = "a".repeat(64);
        let s = format!("blake3:{}", hex);
        let oid = Oid::parse(&s).unwrap();
        assert_eq!(oid.algo, HashAlgo::Blake3);
        assert_eq!(oid.hex, hex);
    }

    #[test]
    fn test_oid_parse_invalid() {
        assert!(Oid::parse("nocolon").is_err());
        assert!(Oid::parse("unknown:abc").is_err());
        assert!(Oid::parse("blake3:short").is_err());
        assert!(Oid::parse(&format!("blake3:{}", "g".repeat(64))).is_err()); // invalid hex
    }

    #[test]
    fn test_oid_storage_path() {
        let oid = Oid::blake3("abcdef1234567890".to_string() + &"0".repeat(48));
        let (prefix, suffix) = oid.storage_path_components();
        assert_eq!(prefix, "ab");
        assert_eq!(suffix.len(), 62);
    }

    #[test]
    fn test_oid_serde_roundtrip() {
        let oid = Oid::blake3("a".repeat(64));
        let json = serde_json::to_string(&oid).unwrap();
        let parsed: Oid = serde_json::from_str(&json).unwrap();
        assert_eq!(oid, parsed);
    }
}
