//! §9.6 — the manifest that gives a bundle its identity. Relative paths are
//! normalized to `/` and Unicode NFC, files are sorted, and the bundle id is the
//! SHA-256 of that canonical text, so touching an mtime cannot make a bundle look
//! new while a same-length content change always does.

use super::canonical::sha256_hex;
use super::diagnostics::Diagnostic;
use super::limits::Limits;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Path identity in Unicode NFC (§9.6). Falls back to the raw text if the
/// normalizer is unavailable, which keeps identity stable within one run.
fn to_nfc(text: &str) -> String {
    icu_normalizer::ComposingNormalizer::new_nfc()
        .normalize(text)
        .into_owned()
}

/// `/`-joined, NFC-normalized path relative to `root`; `None` when the path escapes
/// the root through `..` or is not below it at all.
pub fn normalize_relative(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                // Lossy names can collide, and tab/newline would change the
                // canonical manifest field/record boundaries.
                let text = part.to_str()?;
                if text.chars().any(char::is_control) {
                    return None;
                }
                parts.push(text.to_string());
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(to_nfc(&parts.join("/")))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestFile {
    pub relative_path: String,
    pub content_hash: String,
    pub bytes: u64,
    pub source_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub bundle_id: String,
    pub canonical: String,
    pub files: Vec<ManifestFile>,
}

impl Manifest {
    pub fn content_hash(&self, relative_path: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|file| file.relative_path == relative_path)
            .map(|file| file.content_hash.as_str())
    }
}

/// Reads every file of a bundle once, bounded by `limits`, and returns the manifest
/// plus the bytes that were read, so no adapter has to touch the source again.
/// Files of a bundle as `(relative path, bytes)`, read once for the whole import.
pub type Payloads = Vec<(String, Vec<u8>)>;

pub fn build(
    root: &Path,
    source_dir: &Path,
    files: &[PathBuf],
    limits: &Limits,
) -> Result<(Manifest, Payloads), Vec<Diagnostic>> {
    build_with_reader(root, source_dir, files, limits, read_bounded_regular)
}

fn read_bounded_regular(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    // Do not allocate from the size returned by stat: the file can grow between
    // stat and read. The extra byte lets the caller reject, never truncate.
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn build_with_reader(
    root: &Path,
    source_dir: &Path,
    files: &[PathBuf],
    limits: &Limits,
    mut read_file: impl FnMut(&Path, u64) -> std::io::Result<Vec<u8>>,
) -> Result<(Manifest, Payloads), Vec<Diagnostic>> {
    if files.len() > limits.bundle_files {
        return Err(vec![Diagnostic::error(
            "bundle_files_limit",
            source_dir.display().to_string(),
            format!(
                "bundle 含 {} 个文件，超过上限 {}",
                files.len(),
                limits.bundle_files
            ),
        )]);
    }
    let mut failures: Vec<Diagnostic> = Vec::new();
    let mut paths = std::collections::BTreeMap::new();
    for path in files {
        let Some(relative_path) = normalize_relative(root, path) else {
            failures.push(Diagnostic::error(
                "path_traversal_rejected",
                path.display().to_string(),
                "相对路径越界、不能无损编码或包含控制字符".to_string(),
            ));
            continue;
        };
        if paths.insert(relative_path.clone(), path).is_some() {
            failures.push(Diagnostic::error(
                "duplicate_manifest_path",
                relative_path,
                "规范化后路径重复，拒绝歧义原文".to_string(),
            ));
        }
    }
    if !failures.is_empty() {
        return Err(failures);
    }
    let mut ordered = Vec::new();
    let mut payloads = Vec::new();
    let mut total = 0u64;
    for (relative_path, path) in paths {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            failures.push(Diagnostic::error(
                "unreadable",
                path.display().to_string(),
                "无法 stat".to_string(),
            ));
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            failures.push(Diagnostic::error(
                "not_a_regular_file",
                path.display().to_string(),
                "只导入普通文件".to_string(),
            ));
            continue;
        }
        let bytes = metadata.len();
        if bytes > limits.file_bytes {
            failures.push(Diagnostic::error(
                "file_bytes_limit",
                path.display().to_string(),
                format!("单文件 {bytes} 字节超过上限 {}", limits.file_bytes),
            ));
            continue;
        }
        let remaining = limits.bundle_bytes.saturating_sub(total);
        if bytes > remaining {
            failures.push(Diagnostic::error(
                "bundle_bytes_limit",
                source_dir.display().to_string(),
                "bundle 超过剩余字节预算".to_string(),
            ));
            break;
        }
        let read = match read_file(path, limits.file_bytes.min(remaining)) {
            Ok(read) => read,
            Err(error) => {
                failures.push(Diagnostic::error(
                    "unreadable",
                    path.display().to_string(),
                    error.to_string(),
                ));
                continue;
            }
        };
        let actual = read.len() as u64;
        if actual > limits.file_bytes {
            failures.push(Diagnostic::error(
                "file_bytes_limit",
                path.display().to_string(),
                "读取期间单文件超过上限".to_string(),
            ));
            break;
        }
        if actual > remaining {
            failures.push(Diagnostic::error(
                "bundle_bytes_limit",
                source_dir.display().to_string(),
                "读取期间 bundle 超过字节预算".to_string(),
            ));
            break;
        }
        total += actual;
        let hash = sha256_hex(&read);
        ordered.push(ManifestFile {
            relative_path: relative_path.clone(),
            content_hash: hash,
            bytes: actual,
            source_path: path.clone(),
        });
        // Move the exact hashed bytes into the parser/CAS payload. No reread,
        // and no read failure can be silently replaced by an empty document.
        payloads.push((relative_path, read));
    }
    if !failures.is_empty() {
        return Err(failures);
    }
    let canonical = ordered
        .iter()
        .map(|file| {
            format!(
                "{}\t{}\t{}\n",
                file.relative_path, file.content_hash, file.bytes
            )
        })
        .collect::<String>();
    if canonical.is_empty() {
        return Err(vec![Diagnostic::error(
            "empty_bundle",
            source_dir.display().to_string(),
            "bundle 没有任何可读文件".to_string(),
        )]);
    }
    Ok((
        Manifest {
            bundle_id: format!("sha256:{}", sha256_hex(canonical.as_bytes())),
            canonical,
            files: ordered,
        },
        payloads,
    ))
}

#[cfg(test)]
#[path = "tests_manifest_snapshot.rs"]
mod tests;
