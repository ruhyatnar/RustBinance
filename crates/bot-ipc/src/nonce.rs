//! Single-use 10-second confirmation nonces for destructive IPC commands (§10.1).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngExt;
use time::OffsetDateTime;

use crate::error::IpcError;
use crate::protocol::ConfirmAction;

/// Default TTL for destructive command confirmation nonces (10 seconds, §10.1).
pub const DEFAULT_CONFIRM_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
struct NonceEntry {
    action: ConfirmAction,
    principal: String,
    expires_at: Instant,
}

/// Issued confirmation nonce and its UTC expiry timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedNonce {
    /// Cryptographically random URL-safe nonce string.
    pub nonce: String,
    /// UTC epoch milliseconds when this nonce expires.
    pub expires_at_ms: i64,
}

/// Thread-safe, single-use confirmation nonce store with automatic expiry.
#[derive(Debug)]
pub struct ConfirmNonceStore {
    ttl: Duration,
    entries: Mutex<HashMap<String, NonceEntry>>,
}

impl Default for ConfirmNonceStore {
    fn default() -> Self {
        Self::new(DEFAULT_CONFIRM_TTL)
    }
}

impl ConfirmNonceStore {
    /// Create a nonce store with a custom TTL (production default: 10 seconds).
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Issue a fresh single-use confirmation nonce bound to `(action, principal)`.
    #[must_use]
    pub fn issue(&self, action: ConfirmAction, principal: &str) -> IssuedNonce {
        let mut bytes = [0_u8; 24];
        rand::rng().fill(&mut bytes);
        let nonce = URL_SAFE_NO_PAD.encode(bytes);

        let now = Instant::now();
        let expires_at = now + self.ttl;
        let ttl_ms = i64::try_from(self.ttl.as_millis()).unwrap_or(10_000);
        let now_ms = unix_now_ms();
        let expires_at_ms = now_ms.saturating_add(ttl_ms);

        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.retain(|_, e| e.expires_at > now);
        guard.insert(
            nonce.clone(),
            NonceEntry {
                action,
                principal: principal.to_owned(),
                expires_at,
            },
        );

        IssuedNonce {
            nonce,
            expires_at_ms,
        }
    }

    /// Verify and atomically consume a confirmation nonce for `(action, principal)`.
    ///
    /// # Errors
    /// Returns [`IpcError::InvalidConfirmNonce`] if the nonce does not exist, has expired,
    /// was issued for a different action, or belongs to a different principal.
    pub fn consume(
        &self,
        nonce: &str,
        expected_action: ConfirmAction,
        principal: &str,
    ) -> Result<(), IpcError> {
        if nonce.is_empty() {
            return Err(IpcError::InvalidConfirmNonce(
                "confirmation nonce is empty".to_owned(),
            ));
        }

        let now = Instant::now();
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.retain(|_, e| e.expires_at > now);

        let Some(entry) = guard.remove(nonce) else {
            return Err(IpcError::InvalidConfirmNonce(
                "confirmation nonce is unknown, expired, or already used".to_owned(),
            ));
        };

        if entry.action != expected_action {
            return Err(IpcError::InvalidConfirmNonce(format!(
                "confirmation nonce was issued for '{}', not '{}'",
                entry.action.as_str(),
                expected_action.as_str()
            )));
        }

        if entry.principal != principal {
            return Err(IpcError::InvalidConfirmNonce(
                "confirmation nonce belongs to a different caller".to_owned(),
            ));
        }

        Ok(())
    }
}

/// Current UTC epoch time in milliseconds.
#[must_use]
pub fn unix_now_ms() -> i64 {
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000).unwrap_or(0)
}
