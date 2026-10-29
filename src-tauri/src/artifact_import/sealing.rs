//! §9.4 缺口 2 — an original artifact that carries credentials is stored sealed.
//! A file mode is not encryption: the bytes on disk are AES-256-GCM ciphertext under
//! a per-install key, so neither the display surface, the CAS directory yields plaintext without that key.

use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use std::path::Path;
use std::sync::atomic::{compiler_fence, Ordering};

/// Legacy format. V1 sealed objects bind no associated data; retained so originals
/// already on disk keep opening after the format advanced to V2.
pub const SEALED_MAGIC: &[u8] = b"OVIR-SEALED-V1\n";
/// Current format. The version header is bound as AEAD associated data, so a V2
/// blob cannot be reinterpreted under a different version header (a downgrade)
/// without failing the tag. New seals always use this.
pub const SEALED_MAGIC_V2: &[u8] = b"OVIR-SEALED-V2\n";
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const KEY_BYTES: usize = 32;

pub fn is_sealed(blob: &[u8]) -> bool {
    blob.starts_with(SEALED_MAGIC) || blob.starts_with(SEALED_MAGIC_V2)
}

/// Associated data bound for a given version header: V2 authenticates its header,
/// V1 bound nothing.
fn aad_for(magic: &[u8]) -> &[u8] {
    if magic == SEALED_MAGIC_V2 {
        magic
    } else {
        &[]
    }
}

#[derive(Clone)]
pub struct Key {
    bytes: [u8; KEY_BYTES],
}

impl Drop for Key {
    fn drop(&mut self) {
        // The key is a credential; wipe it from memory on drop. A volatile write
        // plus a compiler fence stops the optimiser eliding the overwrite of a
        // value that is never read again.
        for byte in &mut self.bytes {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        compiler_fence(Ordering::SeqCst);
    }
}

impl Key {
    /// Loads the installation key, creating it on first use. The file is written with
    /// mode 0600 because it is a credential in its own right.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        match super::atomic_file::read_regular(path) {
            Ok(existing) => return Self::from_bytes(&existing),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法读取导入密钥：{error}")),
        }
        let mut bytes = [0u8; KEY_BYTES];
        aws_lc_rs::rand::fill(&mut bytes).map_err(|error| format!("无法生成导入密钥：{error}"))?;
        super::atomic_file::publish_new(path, &bytes)
            .map_err(|error| format!("无法写入导入密钥：{error}"))?;
        // Always load the published winner, never retain a losing random key.
        let existing = super::atomic_file::read_regular(path).map_err(|error| error.to_string())?;
        Self::from_bytes(&existing)
    }

    fn from_bytes(raw: &[u8]) -> Result<Self, String> {
        if raw.len() != KEY_BYTES {
            return Err(format!("导入密钥长度应为 {KEY_BYTES} 字节"));
        }
        let mut bytes = [0u8; KEY_BYTES];
        bytes.copy_from_slice(raw);
        Ok(Self { bytes })
    }

    fn binding(&self) -> Result<LessSafeKey, String> {
        let key = UnboundKey::new(&AES_256_GCM, &self.bytes)
            .map_err(|error| format!("导入密钥不可用：{error}"))?;
        Ok(LessSafeKey::new(key))
    }
}

fn seal_with(key: &Key, plaintext: &[u8], magic: &[u8]) -> Result<Vec<u8>, String> {
    let binding = key.binding()?;
    let mut nonce = [0u8; NONCE_BYTES];
    aws_lc_rs::rand::fill(&mut nonce).map_err(|error| format!("无法生成随机 nonce：{error}"))?;
    let mut buffer = plaintext.to_vec();
    let tag = binding
        .seal_in_place_separate_tag(
            Nonce::try_assume_unique_for_key(&nonce).map_err(|error| error.to_string())?,
            Aad::from(aad_for(magic)),
            &mut buffer,
        )
        .map_err(|error| format!("封存失败：{error}"))?;
    let mut blob = magic.to_vec();
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&buffer);
    blob.extend_from_slice(tag.as_ref());
    Ok(blob)
}

/// `magic || nonce || ciphertext || tag`, current format (V2, header bound as AAD).
pub fn seal(key: &Key, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    seal_with(key, plaintext, SEALED_MAGIC_V2)
}

/// Reverses `seal`. An unsealed blob is returned untouched, so a credential-free
/// original keeps its byte-for-byte storage.
pub fn open(key: &Key, blob: &[u8]) -> Result<Vec<u8>, String> {
    // Dispatch on the version header so the associated data matches what was sealed.
    // An unsealed blob (a credential-free original) is returned byte-for-byte.
    let magic: &[u8] = if blob.starts_with(SEALED_MAGIC_V2) {
        SEALED_MAGIC_V2
    } else if blob.starts_with(SEALED_MAGIC) {
        SEALED_MAGIC
    } else {
        return Ok(blob.to_vec());
    };
    let body = &blob[magic.len()..];
    if body.len() < NONCE_BYTES + TAG_BYTES {
        return Err("封存数据不完整".to_string());
    }
    let (nonce_bytes, rest) = body.split_at(NONCE_BYTES);
    let mut nonce = [0u8; NONCE_BYTES];
    nonce.copy_from_slice(nonce_bytes);
    let (cipher, tag) = rest.split_at(rest.len() - TAG_BYTES);
    let binding = key.binding()?;
    let mut buffer = cipher.to_vec();
    let plaintext = binding
        .open_in_place_separate_tag(
            Nonce::try_assume_unique_for_key(&nonce).map_err(|error| error.to_string())?,
            Aad::from(aad_for(magic)),
            tag,
            &mut buffer,
        )
        .map_err(|error| format!("解封失败（密钥不匹配或数据被改动）：{error}"))?;
    Ok(plaintext.to_vec())
}

/// Byte-level credential sniff over an original file. A JSON string nested inside
/// another JSON value keeps its quotes escaped on disk, which the text scrubber's key
/// scan does not see, so the unescaped view is checked as well.
pub fn secret_bearing(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    if crate::agent_runtime::secrets::redact_text_with(&text, None) != *text {
        return true;
    }
    let unescaped = text.replace("\\\"", "\"");
    crate::agent_runtime::secrets::redact_text_with(&unescaped, None) != unescaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> Key {
        Key::from_bytes(&[7u8; KEY_BYTES]).unwrap()
    }

    #[test]
    fn v2_round_trips_and_authenticates_its_header() {
        let key = test_key();
        let blob = seal(&key, b"secret-token=abc").unwrap();
        assert!(blob.starts_with(SEALED_MAGIC_V2));
        assert!(is_sealed(&blob));
        assert_eq!(open(&key, &blob).unwrap(), b"secret-token=abc");
        // Rewriting the header to V1 (same length) changes the bound AAD, so the
        // tag no longer verifies: the version header cannot be downgraded.
        let mut downgraded = blob.clone();
        downgraded[..SEALED_MAGIC_V2.len()].copy_from_slice(SEALED_MAGIC);
        assert!(open(&key, &downgraded).is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let key = test_key();
        let mut blob = seal(&key, b"payload").unwrap();
        let byte = blob.len() - TAG_BYTES - 1;
        blob[byte] ^= 0xff;
        assert!(open(&key, &blob).is_err());
    }

    #[test]
    fn legacy_v1_blobs_still_open() {
        let key = test_key();
        let blob = seal_with(&key, b"legacy original", SEALED_MAGIC).unwrap();
        assert!(blob.starts_with(SEALED_MAGIC));
        assert_eq!(open(&key, &blob).unwrap(), b"legacy original");
    }

    #[test]
    fn unsealed_blob_passes_through_untouched() {
        let key = test_key();
        assert_eq!(open(&key, b"plain original").unwrap(), b"plain original");
    }
}
