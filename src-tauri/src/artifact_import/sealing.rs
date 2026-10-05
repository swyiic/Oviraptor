//! §9.4 缺口 2 — an original artifact that carries credentials is stored sealed.
//! A file mode is not encryption: the bytes on disk are AES-256-GCM ciphertext under
//! a per-install key, so neither the display surface, the CAS directory yields plaintext without that key.

use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use std::path::Path;

/// Distinguishes a sealed object from a plain original. Versioned so a later stage
/// can re-key without guessing.
pub const SEALED_MAGIC: &[u8] = b"OVIR-SEALED-V1\n";
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const KEY_BYTES: usize = 32;

pub fn is_sealed(blob: &[u8]) -> bool {
    blob.starts_with(SEALED_MAGIC)
}

#[derive(Clone)]
pub struct Key {
    bytes: [u8; KEY_BYTES],
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

/// `magic || nonce || ciphertext || tag`.
pub fn seal(key: &Key, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let binding = key.binding()?;
    let mut nonce = [0u8; NONCE_BYTES];
    aws_lc_rs::rand::fill(&mut nonce).map_err(|error| format!("无法生成随机 nonce：{error}"))?;
    let mut buffer = plaintext.to_vec();
    let tag = binding
        .seal_in_place_separate_tag(
            Nonce::try_assume_unique_for_key(&nonce).map_err(|error| error.to_string())?,
            Aad::from(&[] as &[u8]),
            &mut buffer,
        )
        .map_err(|error| format!("封存失败：{error}"))?;
    let mut blob = SEALED_MAGIC.to_vec();
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&buffer);
    blob.extend_from_slice(tag.as_ref());
    Ok(blob)
}

/// Reverses `seal`. An unsealed blob is returned untouched, so a credential-free
/// original keeps its byte-for-byte storage.
pub fn open(key: &Key, blob: &[u8]) -> Result<Vec<u8>, String> {
    if !is_sealed(blob) {
        return Ok(blob.to_vec());
    }
    let body = &blob[SEALED_MAGIC.len()..];
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
            Aad::from(&[] as &[u8]),
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
