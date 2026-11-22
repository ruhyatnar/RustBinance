//! Redacted secret wrapper with automatic zeroization on drop.

use std::fmt;
use zeroize::Zeroize;

/// Wrapper for sensitive values (API keys, private keys, tokens).
///
/// Redacts all [`fmt::Debug`] output, intentionally omits `Display`, `Serialize`,
/// `Deref`, and `Clone`, and zeroizes inner memory on [`Drop`].
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    /// Wrap a sensitive value.
    #[must_use]
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Explicitly borrow the underlying secret value.
    #[must_use]
    pub const fn expose(&self) -> &T {
        &self.0
    }
}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use zeroize::Zeroize;

    use super::Secret;

    #[test]
    fn secret_redacts_debug_pretty_and_nested() {
        #[derive(Debug)]
        struct Holder {
            #[allow(dead_code)]
            api_key: Secret<String>,
        }

        let s = Secret::new("rahasia-super-penting".to_owned());
        let dbg = format!("{s:?}");
        let pretty = format!("{s:#?}");
        assert_eq!(dbg, "Secret(***)");
        assert_eq!(pretty, "Secret(***)");
        assert!(!dbg.contains("rahasia"));

        let holder = Holder { api_key: s };
        let nested = format!("{holder:?}");
        assert!(nested.contains("Secret(***)"));
        assert!(!nested.contains("rahasia"));
    }

    #[test]
    fn secret_exposes_edge_values_and_zeroizes_on_drop() {
        struct ZeroizeProbe {
            cleared: Arc<AtomicBool>,
        }
        impl Zeroize for ZeroizeProbe {
            fn zeroize(&mut self) {
                self.cleared.store(true, Ordering::SeqCst);
            }
        }

        let empty = Secret::new(String::new());
        assert_eq!(empty.expose(), "");

        let unicode = Secret::new("rahasia-🔑-αβγ".to_owned());
        assert_eq!(unicode.expose(), "rahasia-🔑-αβγ");

        let cleared = Arc::new(AtomicBool::new(false));
        {
            let _probe = Secret::new(ZeroizeProbe {
                cleared: Arc::clone(&cleared),
            });
            assert!(!cleared.load(Ordering::SeqCst));
        }
        assert!(cleared.load(Ordering::SeqCst));
    }
}
