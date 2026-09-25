//! The key that encrypts the refresh token.
//!
//! Never stored. Recomputed on every use from `SHA256(context ‖ machine_id ‖
//! user_home)`, see [`derive_master_key`]. Why not the OS keychain: ADR-0002.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::platform::read_machine_id;

/// The AES key is SHA-256 over three things: this string, the machine id, and
/// the home path. The last two cannot be chosen, so this string is the only way
/// to derive a different key. Give every new purpose its own string.
///
/// Changing the value signs every user out: the stored token was encrypted with
/// the old key, and the new one cannot open it.
const AUTH_KEY_CONTEXT: &[u8] = b"hindsight-auth-v1";

/// Derives the 32-byte AES key: `SHA256(context || machine_id || user_home)`.
///
/// Recomputed on every call and never stored, whether in an OS keyring, the
/// Keychain or a file. The three inputs:
/// - `context` = [`AUTH_KEY_CONTEXT`]: a domain-separation constant
/// - `machine_id`: the platform's stable machine identifier; it changes only
///   when the system is reinstalled or the hardware changes
/// - `user_home`: [`dirs::home_dir`]; it changes only if the user account is
///   deleted
///
/// Why not a keyring: see the module documentation.
pub(crate) fn derive_master_key() -> Result<[u8; 32]> {
    let machine = read_machine_id()?;
    let user = read_user_home_bytes();
    let mut hasher = Sha256::new();
    hasher.update(AUTH_KEY_CONTEXT);
    hasher.update(b"|machine|");
    hasher.update(&machine);
    hasher.update(b"|user|");
    hasher.update(&user);
    let result = hasher.finalize();
    let mut k = [0u8; 32];
    k.copy_from_slice(&result);
    Ok(k)
}

/// Returns the current user's home directory path as UTF-8 bytes.
/// It is one input of the local key, and usually changes only when the user
/// account or the home directory changes.
fn read_user_home_bytes() -> Vec<u8> {
    dirs::home_dir()
        .map(|p| {
            p.into_os_string()
                .to_string_lossy()
                .into_owned()
                .into_bytes()
        })
        .unwrap_or_default()
}

/// Encrypts `plaintext` with AES-256-GCM using `key`.
/// The output contains, in order, a 12-byte nonce, the ciphertext, and a
/// 16-byte authentication tag. [`aes_decrypt`] parses and verifies this format.
pub(crate) fn aes_encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| Error::Crypto("aes encrypt"))?;

    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts an AES-256-GCM value. The first 12 bytes of `ciphertext` are the
/// nonce; the remaining bytes contain the ciphertext and authentication tag.
/// `key` comes from [`derive_master_key`], which returns the same key for the
/// same `(machine_id, user_home)` pair.
pub(crate) fn aes_decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.len() < 13 {
        return Err(Error::Crypto("ciphertext too short"));
    }
    let (nonce_bytes, ct) = ciphertext.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ct)
        .map_err(|_| Error::Crypto("aes decrypt"))
}
