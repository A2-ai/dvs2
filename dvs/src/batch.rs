use serde::{Deserialize, Serialize};

/// Result of a batch operation that may partially succeed.
///
/// `S` is the success-row type, `E` the error-row type. Both are expected
/// to carry their own identifying field (e.g. `path`) in their payload; the
/// outcome itself is intentionally generic so it can be reused across commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOutcome<S, E> {
    pub ok: Vec<S>,
    pub err: Vec<E>,
}

impl<S, E> BatchOutcome<S, E> {
    pub fn new() -> Self {
        Self {
            ok: Vec::new(),
            err: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.err.is_empty()
    }

    pub fn split(self) -> (Vec<S>, Vec<E>) {
        (self.ok, self.err)
    }
}

impl<S, E> Default for BatchOutcome<S, E> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_clean_true_when_no_errors() {
        let b: BatchOutcome<u32, String> = BatchOutcome {
            ok: vec![1, 2],
            err: vec![],
        };
        assert!(b.is_clean());
    }

    #[test]
    fn is_clean_false_when_any_error() {
        let b: BatchOutcome<u32, String> = BatchOutcome {
            ok: vec![1],
            err: vec!["oops".into()],
        };
        assert!(!b.is_clean());
    }

    #[test]
    fn split_returns_both_vecs() {
        let b: BatchOutcome<u32, String> = BatchOutcome {
            ok: vec![1, 2],
            err: vec!["e".into()],
        };
        let (ok, err) = b.split();
        assert_eq!(ok, vec![1, 2]);
        assert_eq!(err, vec!["e".to_string()]);
    }

    #[test]
    fn default_is_empty() {
        let b: BatchOutcome<u32, String> = BatchOutcome::default();
        assert!(b.ok.is_empty());
        assert!(b.err.is_empty());
        assert!(b.is_clean());
    }
}
