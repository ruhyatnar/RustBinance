//! Authentication, origin verification, UDS `SO_PEERCRED` policy, and rate limiting (§10.1).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bot_types::Secret;
use subtle::ConstantTimeEq;

use crate::error::IpcError;

/// Permission scope assigned to an authenticated caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthScope {
    /// Read-only access (`GET` endpoints and telemetry stream only).
    ReadOnly,
    /// Full control access (reads, streams, and state-mutating commands).
    Full,
}

impl AuthScope {
    /// Whether this scope permits state-mutating commands.
    #[must_use]
    pub const fn can_mutate(self) -> bool {
        matches!(self, Self::Full)
    }

    /// Stable label for audit records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "readonly",
            Self::Full => "full",
        }
    }
}

/// Bearer token authenticator supporting separate full-control and read-only tokens (§10.1).
///
/// Tokens are stored inside [`Secret<String>`] (shared via `Arc` without cloning secret bytes)
/// and compared in constant time.
#[derive(Clone)]
pub struct TokenAuth {
    full_token: Arc<Secret<String>>,
    readonly_token: Option<Arc<Secret<String>>>,
}

impl std::fmt::Debug for TokenAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenAuth")
            .field("full_token", &"[REDACTED]")
            .field(
                "readonly_token",
                &self.readonly_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl TokenAuth {
    /// Create a token authenticator from in-memory secrets.
    #[must_use]
    pub fn new(full_token: Secret<String>, readonly_token: Option<Secret<String>>) -> Self {
        Self {
            full_token: Arc::new(full_token),
            readonly_token: readonly_token.map(Arc::new),
        }
    }

    /// Load bearer tokens from files on disk (whitespace trimmed).
    ///
    /// # Errors
    /// Returns [`IpcError::Io`] if reading a token file fails, or [`IpcError::BadRequest`]
    /// if the full token file is empty.
    pub fn from_files(
        full_token_path: &Path,
        readonly_token_path: Option<&Path>,
    ) -> Result<Self, IpcError> {
        let full_raw = std::fs::read_to_string(full_token_path)?;
        let full_trimmed = full_raw.trim().to_owned();
        if full_trimmed.is_empty() {
            return Err(IpcError::BadRequest(
                "full bearer token file must not be empty".to_owned(),
            ));
        }
        let readonly_token = match readonly_token_path {
            Some(path) if !path.as_os_str().is_empty() => {
                let raw = std::fs::read_to_string(path)?;
                let trimmed = raw.trim().to_owned();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(Secret::new(trimmed))
                }
            }
            _ => None,
        };
        Ok(Self::new(Secret::new(full_trimmed), readonly_token))
    }

    /// Authenticate a raw bearer token string using constant-time byte comparison.
    ///
    /// # Errors
    /// Returns [`IpcError::Unauthorized`] if the token is empty or does not match.
    pub fn authenticate(&self, candidate: &str) -> Result<AuthScope, IpcError> {
        if candidate.is_empty() {
            return Err(IpcError::Unauthorized("empty bearer token".to_owned()));
        }
        if ct_eq_str(candidate, self.full_token.expose()) {
            return Ok(AuthScope::Full);
        }
        if let Some(ro) = &self.readonly_token
            && ct_eq_str(candidate, ro.expose())
        {
            return Ok(AuthScope::ReadOnly);
        }
        Err(IpcError::Unauthorized("invalid bearer token".to_owned()))
    }
}

/// Constant-time string comparison (length check + `subtle::ConstantTimeEq`).
fn ct_eq_str(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    if a_bytes.len() != b_bytes.len() {
        return false;
    }
    bool::from(a_bytes.ct_eq(b_bytes))
}

/// Strict `Origin` header validation policy (CORS default closed, §10.1).
#[derive(Debug, Clone)]
pub struct OriginPolicy {
    allowed_origins: HashSet<String>,
}

impl OriginPolicy {
    /// Create an allowlist of exact `Origin` header values.
    #[must_use]
    pub fn new(origins: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed_origins: origins.into_iter().collect(),
        }
    }

    /// Default loopback allowlist for a local HTTP/WS server on `port`.
    #[must_use]
    pub fn loopback_default(port: u16) -> Self {
        Self::new([
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
            format!("http://[::1]:{port}"),
        ])
    }

    /// Validate an optional `Origin` header.
    ///
    /// Non-browser clients (CLI, curl, reverse-proxy internal healthchecks) omit `Origin`;
    /// when `Origin` IS present, it MUST exactly match the allowlist.
    ///
    /// # Errors
    /// Returns [`IpcError::Forbidden`] if `Origin` is present and not allowed.
    pub fn validate(&self, origin: Option<&str>) -> Result<(), IpcError> {
        let Some(origin_val) = origin else {
            return Ok(());
        };
        if self.allowed_origins.contains(origin_val) {
            Ok(())
        } else {
            Err(IpcError::Forbidden(format!(
                "origin '{origin_val}' is not allowed"
            )))
        }
    }
}

/// Per-caller token-bucket rate limiter (§10.1).
#[derive(Debug)]
pub struct RateLimiter {
    capacity: u32,
    refill_per_sec: u32,
    buckets: Mutex<HashMap<String, BucketState>>,
}

#[derive(Debug, Clone)]
struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    /// Create a rate limiter with `capacity` burst tokens and `refill_per_sec` sustained rate.
    #[must_use]
    pub fn new(capacity: u32, refill_per_sec: u32) -> Self {
        Self {
            capacity: capacity.max(1),
            refill_per_sec: refill_per_sec.max(1),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Attempt to consume 1 request token for `key`.
    ///
    /// # Errors
    /// Returns [`IpcError::RateLimited`] when the caller has exhausted its bucket.
    pub fn check(&self, key: &str) -> Result<(), IpcError> {
        let now = Instant::now();
        let mut guard = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Opportunistically bound map size to prevent memory exhaustion under unique keys.
        if guard.len() > 1024 {
            let cutoff = Duration::from_secs(60);
            guard.retain(|_, state| now.saturating_duration_since(state.last_refill) < cutoff);
        }

        let cap = f64::from(self.capacity);
        let rate = f64::from(self.refill_per_sec);
        let entry = guard.entry(key.to_owned()).or_insert_with(|| BucketState {
            tokens: cap,
            last_refill: now,
        });

        let elapsed = now
            .saturating_duration_since(entry.last_refill)
            .as_secs_f64();
        entry.tokens = (entry.tokens + elapsed * rate).min(cap);
        entry.last_refill = now;

        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            Ok(())
        } else {
            Err(IpcError::RateLimited)
        }
    }
}

/// UDS `SO_PEERCRED` peer authorization policy (§10.1).
#[derive(Debug, Clone)]
pub struct PeerAuthPolicy {
    allowed_uids: HashSet<u32>,
    allowed_gids: HashSet<u32>,
}

impl PeerAuthPolicy {
    /// Construct a policy allowing peers matching any UID in `user_ids` or GID in `group_ids`.
    #[must_use]
    pub fn new(
        user_ids: impl IntoIterator<Item = u32>,
        group_ids: impl IntoIterator<Item = u32>,
    ) -> Self {
        Self {
            allowed_uids: user_ids.into_iter().collect(),
            allowed_gids: group_ids.into_iter().collect(),
        }
    }

    /// Policy that permits the current process's effective UID and GID (read from `/proc/self/status`
    /// without `unsafe`).
    #[must_use]
    pub fn allow_current_process() -> Self {
        let (user_id, group_id) = current_process_euid_egid();
        Self::new(user_id, group_id)
    }

    /// Check whether a UDS peer with `(uid, gid)` is authorized.
    #[must_use]
    pub fn is_allowed(&self, user_id: u32, group_id: u32) -> bool {
        self.is_allowed_with_groups(user_id, group_id, &[])
    }

    /// Check whether a UDS peer with `(uid, primary_gid, supplementary_gids)` is authorized.
    #[must_use]
    pub fn is_allowed_with_groups(
        &self,
        user_id: u32,
        primary_gid: u32,
        supplementary_gids: &[u32],
    ) -> bool {
        self.allowed_uids.contains(&user_id)
            || self.allowed_gids.contains(&primary_gid)
            || supplementary_gids
                .iter()
                .any(|gid| self.allowed_gids.contains(gid))
    }
}

/// Parse the `Groups:` line from Linux `/proc/<pid>/status` contents (zero `unsafe`).
#[must_use]
pub fn parse_proc_status_groups(status_contents: &str) -> Vec<u32> {
    for line in status_contents.lines() {
        if let Some(rest) = line.strip_prefix("Groups:") {
            return rest
                .split_whitespace()
                .filter_map(|tok| tok.parse::<u32>().ok())
                .collect();
        }
    }
    Vec::new()
}

/// Read supplementary group IDs of a Linux peer process via `/proc/<pid>/status` (zero `unsafe`).
///
/// Returns an empty `Vec` if `pid` is `None`, non-positive, or `/proc/<pid>/status` cannot be read.
#[must_use]
pub fn peer_supplementary_groups(pid: Option<i32>) -> Vec<u32> {
    let Some(raw_pid) = pid.filter(|&p| p > 0) else {
        return Vec::new();
    };
    let path = format!("/proc/{raw_pid}/status");
    let Ok(status) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_proc_status_groups(&status)
}

/// Read current process EUID and EGID on Linux via `/proc/self/status` (zero `unsafe`).
fn current_process_euid_egid() -> (Option<u32>, Option<u32>) {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    let mut eff_user = None;
    let mut eff_group = None;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            // Format: real effective saved fs
            eff_user = rest
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<u32>().ok());
        } else if let Some(rest) = line.strip_prefix("Gid:") {
            eff_group = rest
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<u32>().ok());
        }
    }
    (eff_user, eff_group)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proc_status_groups_extracts_all_gids() {
        let sample = "Name:\tbotctl\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\nGroups:\t10 998 1000 \n";
        assert_eq!(parse_proc_status_groups(sample), vec![10, 998, 1000]);
    }

    #[test]
    fn peer_auth_allows_matching_supplementary_group() {
        let policy = PeerAuthPolicy::new([999], [998]);
        assert!(!policy.is_allowed(1000, 1000));
        assert!(policy.is_allowed_with_groups(1000, 1000, &[10, 998]));
        assert!(!policy.is_allowed_with_groups(1000, 1000, &[10, 1000]));
    }

    #[test]
    fn peer_supplementary_groups_handles_missing_or_invalid_pid() {
        assert_eq!(peer_supplementary_groups(None), Vec::<u32>::new());
        assert_eq!(peer_supplementary_groups(Some(0)), Vec::<u32>::new());
        assert_eq!(peer_supplementary_groups(Some(-1)), Vec::<u32>::new());
    }
}
