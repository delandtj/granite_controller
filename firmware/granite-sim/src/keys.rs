//! The fleet recovery key: generate it, sign a recovery request with it,
//! verify one against it (ADR 0001 component 13).
//!
//! The private key never touches a board. `granite-sim keygen` writes it
//! to `~/.config/granite/fleet-recovery.key` with mode 0600 and prints
//! the public PEM, which goes on the Security page or into the image at
//! build time. `granite-sim recover` fetches a nonce from `/id`, signs
//! `device || nonce || "factory-reset"` and posts `/recover`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use p256::ecdsa::signature::{Signer as _, Verifier as _};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey, LineEnding};

/// Where the private key lives by default.
pub fn default_key_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("."));
    Path::new(&home).join(".config/granite/fleet-recovery.key")
}

/// Base64 of a DER ECDSA signature, which is what `/recover` expects.
pub fn encode_sig(der: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(der)
}

/// Generate a key pair. Returns `(private PEM, public PEM)`.
pub fn generate() -> (String, String) {
    let signing = <SigningKey as p256::elliptic_curve::Generate>::generate();
    let secret = signing.as_nonzero_scalar();
    let secret_key = p256::SecretKey::from(secret);
    let private_pem = secret_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("a P-256 key always encodes")
        .to_string();
    let public_pem = signing
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .expect("a P-256 public key always encodes");
    (private_pem, public_pem)
}

/// Write the private key with mode 0600 and return the public PEM.
pub fn write_key_pair(path: &Path) -> anyhow::Result<String> {
    if path.exists() {
        anyhow::bail!(
            "{} exists; move it aside first, a new key invalidates every board that trusts the old one",
            path.display()
        );
    }
    let (private_pem, public_pem) = generate();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(private_pem.as_bytes())?;
    file.sync_all()?;
    Ok(public_pem)
}

/// Load a private key from a PEM file.
pub fn load_signing_key(path: &Path) -> anyhow::Result<SigningKey> {
    let pem = fs::read_to_string(path)?;
    let secret = p256::SecretKey::from_pkcs8_pem(&pem)
        .map_err(|e| anyhow::anyhow!("{}: not a PKCS#8 P-256 private key: {e}", path.display()))?;
    Ok(SigningKey::from(&secret))
}

/// The public PEM matching a private key.
pub fn public_pem_of(key: &SigningKey) -> anyhow::Result<String> {
    Ok(key.verifying_key().to_public_key_pem(LineEnding::LF)?)
}

/// What `/recover` signs over: `device || nonce || "factory-reset"`.
pub fn recovery_message(device: &str, nonce: &str) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(device.as_bytes());
    message.extend_from_slice(nonce.as_bytes());
    message.extend_from_slice(granite_core::api::RECOVER_PURPOSE);
    message
}

/// Sign a recovery message; the result is what goes in the `sig` field.
pub fn sign_recovery(key: &SigningKey, device: &str, nonce: &str) -> String {
    let signature: Signature = key.sign(&recovery_message(device, nonce));
    encode_sig(signature.to_der().as_bytes())
}

/// Parse a public key PEM, as the Security page validates it.
pub fn parse_public_pem(pem: &str) -> Result<VerifyingKey, String> {
    VerifyingKey::from_public_key_pem(pem.trim())
        .map_err(|e| format!("not a PEM P-256 public key: {e}"))
}

/// Verify a `/recover` signature. Accepts DER or fixed 64-byte form, in
/// base64 or hex, because a host tool in any language should be able to
/// produce one.
pub fn verify(public_pem: &str, message: &[u8], sig: &str) -> bool {
    let Ok(key) = parse_public_pem(public_pem) else {
        return false;
    };
    let trimmed = sig.trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(trimmed)
                .ok()
        })
        .or_else(|| granite_core::api::unhex(trimmed));
    let Some(bytes) = bytes else {
        return false;
    };
    let signature = Signature::from_der(&bytes)
        .ok()
        .or_else(|| Signature::from_slice(&bytes).ok());
    match signature {
        Some(s) => key.verify(message, &s).is_ok(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_round_trips_through_the_recovery_message() {
        let (private_pem, public_pem) = generate();
        let secret = p256::SecretKey::from_pkcs8_pem(&private_pem).unwrap();
        let key = SigningKey::from(&secret);
        let sig = sign_recovery(&key, "granite-510000", "abcd");
        let message = recovery_message("granite-510000", "abcd");
        assert!(verify(&public_pem, &message, &sig));
        // A different nonce does not verify.
        let other = recovery_message("granite-510000", "abce");
        assert!(!verify(&public_pem, &other, &sig));
        // Neither does another key.
        let (_, stranger) = generate();
        assert!(!verify(&stranger, &message, &sig));
    }
}
