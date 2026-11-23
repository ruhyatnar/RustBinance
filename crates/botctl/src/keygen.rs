//! Ed25519 keypair generation with atomic `0600` creation and zeroization (§12.3).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::pkcs8::spki::EncodePublicKey;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use rand::RngExt;
use zeroize::Zeroizing;

/// Paths written by [`generate_ed25519_keypair`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedKeypair {
    /// Path to private key (`<name>.pem`, mode `0600`).
    pub private_path: PathBuf,
    /// Path to public key (`<name>.pub.pem`, mode `0644`).
    pub public_path: PathBuf,
}

/// Write bytes using `create_new(true)` and an explicit Unix mode from creation time
/// (no chmod race, never overwrites existing keys, §12.3).
fn write_file_atomic_mode(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

/// Generate an Ed25519 keypair in `out_dir` named `<name>.pem` (0600) and `<name>.pub.pem` (0644).
///
/// Refuses to overwrite existing files. Zeroizes all secret key material on drop.
///
/// # Errors
/// Returns an error if `name` is invalid, files already exist, or filesystem I/O fails.
pub fn generate_ed25519_keypair(out_dir: &Path, name: &str) -> Result<GeneratedKeypair> {
    if name.is_empty() || name.contains('/') || name.contains("..") {
        bail!("invalid key name '{name}'");
    }

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create key directory {}", out_dir.display()))?;

    let private_path = out_dir.join(format!("{name}.pem"));
    let public_path = out_dir.join(format!("{name}.pub.pem"));

    if private_path.exists() || public_path.exists() {
        bail!(
            "refusing to overwrite existing key file(s) at {}",
            private_path.display()
        );
    }

    let mut seed = Zeroizing::new([0_u8; 32]);
    rand::rng().fill(&mut *seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();

    let priv_pem = signing_key
        .to_pkcs8_pem(LineEnding::LF)
        .context("failed to encode Ed25519 private key as PKCS#8 PEM")?;
    let pub_pem = verifying_key
        .to_public_key_pem(LineEnding::LF)
        .context("failed to encode Ed25519 public key as SPKI PEM")?;

    write_file_atomic_mode(&private_path, priv_pem.as_bytes(), 0o600).with_context(|| {
        format!(
            "failed to create private key file {} with mode 0600",
            private_path.display()
        )
    })?;

    if let Err(err) = write_file_atomic_mode(&public_path, pub_pem.as_bytes(), 0o644) {
        let _ = fs::remove_file(&private_path);
        return Err(err).with_context(|| {
            format!(
                "failed to create public key file {} with mode 0644",
                public_path.display()
            )
        });
    }

    Ok(GeneratedKeypair {
        private_path,
        public_path,
    })
}
