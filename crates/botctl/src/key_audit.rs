//! Local Ed25519 key audit: SHA-256 fingerprint, PEM validity, Unix permission checks (M19).
//!
//! Never prints private key material: PEM text stays in `Zeroizing` buffers and parse
//! errors are replaced by fixed messages.

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::pkcs8::spki::DecodePublicKey;
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Required mode for private key files.
pub const PRIVATE_KEY_MODE: u32 = 0o600;
/// Required mode for directories containing private keys.
pub const PRIVATE_DIR_MODE: u32 = 0o700;

/// Kind of PEM key file detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyFileKind {
    /// SPKI `PUBLIC KEY` PEM.
    PublicSpki,
    /// PKCS#8 `PRIVATE KEY` PEM.
    PrivatePkcs8,
    /// Could not be decoded as either.
    Unknown,
}

/// Result of inspecting a key file. Contains no secret material.
#[derive(Debug, Clone, Serialize)]
pub struct KeyInspection {
    /// Inspected file path.
    pub path: PathBuf,
    /// Detected key kind.
    pub kind: KeyFileKind,
    /// Whether the PEM decoded as a valid Ed25519 key.
    pub pem_valid: bool,
    /// `SHA256:<hex>` fingerprint of the 32-byte public key (empty when invalid).
    pub sha256_fingerprint: String,
    /// File permission bits in octal (e.g. `0600`).
    pub file_mode_octal: String,
    /// Whether file mode satisfies policy (`0600` for private keys).
    pub file_mode_ok: bool,
    /// Parent directory permission bits in octal.
    pub dir_mode_octal: String,
    /// Whether directory mode satisfies policy (`0700` for private keys).
    pub dir_mode_ok: bool,
    /// Human-readable issues found.
    pub issues: Vec<String>,
}

impl KeyInspection {
    /// `true` when the PEM is valid and all permission checks pass.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.pem_valid && self.file_mode_ok && self.dir_mode_ok && self.issues.is_empty()
    }
}

fn decode_public_key(pem: &str) -> Option<(KeyFileKind, [u8; 32])> {
    if let Ok(vk) = VerifyingKey::from_public_key_pem(pem) {
        return Some((KeyFileKind::PublicSpki, vk.to_bytes()));
    }
    SigningKey::from_pkcs8_pem(pem)
        .ok()
        .map(|sk| (KeyFileKind::PrivatePkcs8, sk.verifying_key().to_bytes()))
}

fn format_fingerprint(public_key: &[u8; 32]) -> String {
    let digest = Sha256::digest(public_key);
    let mut out = String::from("SHA256:");
    for b in &digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn read_pem(path: &Path) -> Result<Zeroizing<String>> {
    std::fs::read_to_string(path)
        .map(Zeroizing::new)
        .with_context(|| format!("failed to read key file {}", path.display()))
}

/// Compute `SHA256:<hex>` fingerprint of the Ed25519 public key in a public or private PEM.
///
/// # Errors
/// Returns an error if the file cannot be read or is not a valid Ed25519 PEM.
pub fn fingerprint_key_file(path: &Path) -> Result<String> {
    let pem = read_pem(path)?;
    let Some((_, pk)) = decode_public_key(&pem) else {
        bail!(
            "{} is not a valid Ed25519 SPKI public or PKCS#8 private PEM",
            path.display()
        );
    };
    Ok(format_fingerprint(&pk))
}

/// Inspect a key file: PEM validity, fingerprint, and Unix permissions.
///
/// # Errors
/// Returns an error only if file metadata or contents cannot be read.
pub fn inspect_key_file(path: &Path) -> Result<KeyInspection> {
    let pem = read_pem(path)?;
    let file_mode = std::fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .permissions()
        .mode()
        & 0o777;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let dir_mode = std::fs::metadata(dir)
        .with_context(|| format!("failed to stat {}", dir.display()))?
        .permissions()
        .mode()
        & 0o777;

    let mut issues = Vec::new();
    let (kind, fingerprint, pem_valid) = if let Some((kind, pk)) = decode_public_key(&pem) {
        (kind, format_fingerprint(&pk), true)
    } else {
        issues.push("file is not a valid Ed25519 SPKI/PKCS#8 PEM".to_owned());
        (KeyFileKind::Unknown, String::new(), false)
    };
    // Unknown files are treated as potentially secret: apply private-key policy.
    let strict = kind != KeyFileKind::PublicSpki;
    let file_mode_ok = !strict || file_mode == PRIVATE_KEY_MODE;
    // Directory must grant no group/other bits (low 6 bits zero, i.e. `0700` or stricter).
    let dir_mode_ok = !strict || dir_mode.trailing_zeros() >= 6;
    if !file_mode_ok {
        issues.push(format!(
            "private key mode is {file_mode:04o}, expected 0600 (chmod 600 {})",
            path.display()
        ));
    }
    if !dir_mode_ok {
        issues.push(format!(
            "key directory mode is {dir_mode:04o}, expected 0700 (chmod 700 {})",
            dir.display()
        ));
    }

    Ok(KeyInspection {
        path: path.to_path_buf(),
        kind,
        pem_valid,
        sha256_fingerprint: fingerprint,
        file_mode_octal: format!("{file_mode:04o}"),
        file_mode_ok,
        dir_mode_octal: format!("{dir_mode:04o}"),
        dir_mode_ok,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::{KeyFileKind, fingerprint_key_file, inspect_key_file};
    use crate::keygen::generate_ed25519_keypair;

    fn secure_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn fingerprint_matches_between_private_and_public_pem() {
        let dir = secure_dir();
        let pair = generate_ed25519_keypair(dir.path(), "k").unwrap();
        let a = fingerprint_key_file(&pair.private_path).unwrap();
        let b = fingerprint_key_file(&pair.public_path).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("SHA256:"));
        assert_eq!(a.len(), 71);
    }

    #[test]
    fn inspect_accepts_0600_private_key_in_0700_dir() {
        let dir = secure_dir();
        let pair = generate_ed25519_keypair(dir.path(), "k").unwrap();
        let rep = inspect_key_file(&pair.private_path).unwrap();
        assert_eq!(rep.kind, KeyFileKind::PrivatePkcs8);
        assert!(rep.pem_valid);
        assert!(rep.file_mode_ok);
        assert!(rep.dir_mode_ok);
        assert!(rep.issues.is_empty(), "{:?}", rep.issues);
    }

    #[test]
    fn inspect_flags_loose_private_key_and_dir_permissions() {
        let dir = secure_dir();
        let pair = generate_ed25519_keypair(dir.path(), "k").unwrap();
        fs::set_permissions(&pair.private_path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let rep = inspect_key_file(&pair.private_path).unwrap();
        assert!(!rep.file_mode_ok);
        assert!(!rep.dir_mode_ok);
        assert!(rep.issues.len() >= 2);
        assert!(!rep.is_ok());
    }

    #[test]
    fn inspect_public_key_does_not_require_strict_modes() {
        let dir = secure_dir();
        let pair = generate_ed25519_keypair(dir.path(), "k").unwrap();
        let rep = inspect_key_file(&pair.public_path).unwrap();
        assert_eq!(rep.kind, KeyFileKind::PublicSpki);
        assert!(rep.is_ok(), "{:?}", rep.issues);
    }

    #[test]
    fn inspect_and_error_never_leak_private_pem_content() {
        let dir = secure_dir();
        let pair = generate_ed25519_keypair(dir.path(), "k").unwrap();
        let pem = fs::read_to_string(&pair.private_path).unwrap();
        let body_line = pem.lines().nth(1).unwrap().to_owned();
        let rep = inspect_key_file(&pair.private_path).unwrap();
        let json = serde_json::to_string(&rep).unwrap();
        assert!(!json.contains("BEGIN PRIVATE KEY"));
        assert!(!json.contains(&body_line));

        let bad = dir.path().join("bad.pem");
        fs::write(
            &bad,
            "-----BEGIN PRIVATE KEY-----\nSECRET_SENTINEL_DO_NOT_LEAK\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o600)).unwrap();
        let err = fingerprint_key_file(&bad).unwrap_err();
        let msg = format!("{err:#}");
        assert!(!msg.contains("SECRET_SENTINEL_DO_NOT_LEAK"));
        assert!(!msg.contains("BEGIN PRIVATE KEY"));
        let rep = inspect_key_file(&bad).unwrap();
        assert!(!rep.pem_valid);
        let json = serde_json::to_string(&rep).unwrap();
        assert!(!json.contains("SECRET_SENTINEL_DO_NOT_LEAK"));
    }
}
