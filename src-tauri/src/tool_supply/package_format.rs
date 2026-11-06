//! Explicit flat-bundle-v1 syntax and byte-integrity verification only. No
//! publisher/admin approval, deployment, extraction, or execution is implied.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Seek},
};

pub(crate) const MAGIC: &[u8; 8] = b"OVFB0001";
const MAX_FILES: u32 = 16;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegularFileMode {
    ReadOnlyData,
    EntryCode,
}
#[derive(Debug)]
pub(crate) struct FlatBundleEntry {
    path: String,
    content_offset: u64,
    content_bytes: u64,
    content_sha256: [u8; 32],
    mode: RegularFileMode,
}
impl FlatBundleEntry {
    pub(crate) fn path(&self) -> &str {
        &self.path
    }
    pub(crate) fn content_offset(&self) -> u64 {
        self.content_offset
    }
    pub(crate) fn content_bytes(&self) -> u64 {
        self.content_bytes
    }
    pub(crate) fn content_sha256(&self) -> &[u8; 32] {
        &self.content_sha256
    }
    pub(crate) fn mode(&self) -> RegularFileMode {
        self.mode
    }
}
/// Private construction proves syntax and hashes, never trusted origin. A
/// deployer must bind this whole digest to the actual approved package inode.
#[derive(Debug)]
pub(crate) struct VerifiedFlatBundle {
    entries: Vec<FlatBundleEntry>,
    encoded_bytes: u64,
    sha256: [u8; 32],
}
impl VerifiedFlatBundle {
    pub(crate) fn entries(&self) -> &[FlatBundleEntry] {
        &self.entries
    }
    pub(crate) fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }
    pub(crate) fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }
}

struct BoundedReader<'a, R> {
    source: &'a mut R,
    total: u64,
    hash: Sha256,
}
impl<R: Read> BoundedReader<'_, R> {
    fn exact(&mut self, buffer: &mut [u8]) -> Result<(), String> {
        let length = u64::try_from(buffer.len()).map_err(|_| "flat_bundle_size_invalid")?;
        let next = self
            .total
            .checked_add(length)
            .filter(|n| *n <= MAX_BUNDLE_BYTES)
            .ok_or("flat_bundle_size_invalid")?;
        self.source.read_exact(buffer).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                "flat_bundle_truncated"
            } else {
                "flat_bundle_read_failed"
            }
        })?;
        self.hash.update(buffer);
        self.total = next;
        Ok(())
    }
}
fn require_path(path: &str) -> Result<(), String> {
    // A portable ASCII subset of UTF-8 avoids normalization aliases across
    // filesystems. Other Unicode names are explicitly unsupported, not folded.
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || !path.is_ascii()
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
    {
        return Err("flat_bundle_path_invalid".into());
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > 8 {
        return Err("flat_bundle_path_invalid".into());
    }
    for part in parts {
        if part.is_empty() || matches!(part, "." | "..") || part.ends_with('.') {
            return Err("flat_bundle_path_invalid".into());
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err("flat_bundle_path_invalid".into());
        }
    }
    Ok(())
}

pub(crate) fn parse_flat_bundle_v1<R: Read + Seek>(
    source: &mut R,
    entrypoint: &str,
) -> Result<VerifiedFlatBundle, String> {
    require_path(entrypoint)?;
    if source
        .stream_position()
        .map_err(|_| "flat_bundle_seek_failed")?
        != 0
    {
        return Err("flat_bundle_source_position_invalid".into());
    }
    let mut reader = BoundedReader {
        source,
        total: 0,
        hash: Sha256::new(),
    };
    let mut header = [0u8; 14];
    reader.exact(&mut header)?;
    if &header[..8] != MAGIC {
        return Err("flat_bundle_magic_invalid".into());
    }
    if u16::from_le_bytes([header[8], header[9]]) != 1 {
        return Err("flat_bundle_version_unsupported".into());
    }
    let count = u32::from_le_bytes([header[10], header[11], header[12], header[13]]);
    if count == 0 || count > MAX_FILES {
        return Err("flat_bundle_file_count_invalid".into());
    }
    let mut entries = Vec::new();
    let mut paths = BTreeSet::<String>::new();
    let mut entry_count = 0;
    for _ in 0..count {
        let mut record = [0u8; 43];
        reader.exact(&mut record)?;
        let path_size = usize::from(u16::from_le_bytes([record[0], record[1]]));
        if path_size == 0 || path_size > MAX_PATH_BYTES {
            return Err("flat_bundle_path_invalid".into());
        }
        let content_bytes = u64::from_le_bytes(
            record[2..10]
                .try_into()
                .map_err(|_| "flat_bundle_size_invalid")?,
        );
        if content_bytes > MAX_FILE_BYTES {
            return Err("flat_bundle_file_size_invalid".into());
        }
        let mode = match record[10] {
            0 => RegularFileMode::ReadOnlyData,
            1 => RegularFileMode::EntryCode,
            _ => return Err("flat_bundle_file_mode_invalid".into()),
        };
        let content_sha256: [u8; 32] = record[11..43]
            .try_into()
            .map_err(|_| "flat_bundle_digest_invalid")?;
        let mut path_bytes = vec![0; path_size];
        reader.exact(&mut path_bytes)?;
        let path = String::from_utf8(path_bytes).map_err(|_| "flat_bundle_path_invalid")?;
        require_path(&path)?;
        let normalized = path.to_ascii_lowercase();
        if paths.iter().any(|old| {
            old == &normalized
                || old.starts_with(&(normalized.clone() + "/"))
                || normalized.starts_with(&(old.clone() + "/"))
        }) {
            return Err("flat_bundle_path_conflict".into());
        }
        paths.insert(normalized);
        if mode == RegularFileMode::EntryCode {
            entry_count += 1;
            if path != entrypoint || content_bytes == 0 {
                return Err("flat_bundle_entrypoint_invalid".into());
            }
        } else if path == entrypoint {
            return Err("flat_bundle_entrypoint_invalid".into());
        }
        reader
            .total
            .checked_add(content_bytes)
            .filter(|n| *n <= MAX_BUNDLE_BYTES)
            .ok_or("flat_bundle_size_invalid")?;
        let content_offset = reader.total;
        let mut remaining = content_bytes;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let size = usize::try_from(
                remaining.min(u64::try_from(buffer.len()).map_err(|_| "flat_bundle_size_invalid")?),
            )
            .map_err(|_| "flat_bundle_size_invalid")?;
            reader.exact(&mut buffer[..size])?;
            hash.update(&buffer[..size]);
            remaining -= u64::try_from(size).map_err(|_| "flat_bundle_size_invalid")?;
        }
        let actual: [u8; 32] = hash.finalize().into();
        if actual != content_sha256 {
            return Err("flat_bundle_digest_invalid".into());
        }
        entries.push(FlatBundleEntry {
            path,
            content_offset,
            content_bytes,
            content_sha256,
            mode,
        });
    }
    if entry_count != 1 {
        return Err("flat_bundle_entrypoint_invalid".into());
    }
    let mut tail = [0u8; 1];
    match reader.source.read_exact(&mut tail) {
        Ok(()) => return Err("flat_bundle_trailing_bytes".into()),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {}
        Err(_) => return Err("flat_bundle_read_failed".into()),
    }
    if reader
        .source
        .stream_position()
        .map_err(|_| "flat_bundle_seek_failed")?
        != reader.total
    {
        return Err("flat_bundle_source_position_invalid".into());
    }
    Ok(VerifiedFlatBundle {
        entries,
        encoded_bytes: reader.total,
        sha256: reader.hash.finalize().into(),
    })
}

