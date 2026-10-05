use super::*;

/// Originals are content-addressed and read back for a byte-for-byte comparison
/// (§IMP-001). A file that carries credentials is written as AES-256-GCM ciphertext
/// and only ever reads back through `sealing::open` (§9.4). No database row is
/// written here: that happens inside the commit transaction (缺口 7).
pub(super) fn write_objects(
    context: &ImportContext<'_>,
    manifest: &Manifest,
    payloads: &[(String, Vec<u8>)],
) -> Result<CasRows, Vec<Diagnostic>> {
    let mut rows = Vec::new();
    let mut failures: Vec<Diagnostic> = Vec::new();
    let mut key: Option<sealing::Key> = None;
    for file in &manifest.files {
        let Some((_, bytes)) = payloads
            .iter()
            .find(|(relative, _)| relative == &file.relative_path)
        else {
            continue;
        };
        let bucket = &file.content_hash[..2.min(file.content_hash.len())];
        let directory = context.cas_dir.join(bucket);
        if let Err(error) = std::fs::create_dir_all(&directory) {
            failures.push(Diagnostic::error(
                "cas_unwritable",
                file.relative_path.clone(),
                error.to_string(),
            ));
            continue;
        }
        let secret = sealing::secret_bearing(bytes);
        let stored_bytes = if secret {
            let loaded = match key.clone().or_else(|| {
                sealing::Key::load_or_create(context.key_path)
                    .ok()
                    .inspect(|created| key = Some(created.clone()))
            }) {
                Some(loaded) => loaded,
                None => {
                    failures.push(Diagnostic::error(
                        "key_unavailable",
                        file.relative_path.clone(),
                        "无法建立导入密钥，含凭据的原文不落盘".to_string(),
                    ));
                    continue;
                }
            };
            match sealing::seal(&loaded, bytes) {
                Ok(blob) => blob,
                Err(error) => {
                    failures.push(Diagnostic::error(
                        "seal_failed",
                        file.relative_path.clone(),
                        error,
                    ));
                    continue;
                }
            }
        } else {
            bytes.clone()
        };
        let stored = directory.join(&file.content_hash);
        if !stored.is_file() {
            if let Err(error) =
                crate::artifact_import::atomic_file::publish_new(&stored, &stored_bytes)
            {
                failures.push(Diagnostic::error(
                    "cas_unwritable",
                    file.relative_path.clone(),
                    error.to_string(),
                ));
                continue;
            }
        }
        let read_back = match crate::artifact_import::atomic_file::read_regular(&stored) {
            Ok(read_back) => read_back,
            Err(error) => {
                failures.push(Diagnostic::error(
                    "cas_unreadable",
                    file.relative_path.clone(),
                    error.to_string(),
                ));
                continue;
            }
        };
        if (secret && !sealing::is_sealed(&read_back)) || (!secret && read_back != *bytes) {
            failures.push(Diagnostic::error(
                "cas_byte_mismatch",
                file.relative_path.clone(),
                "对象格式或原文内容不一致".to_string(),
            ));
            continue;
        }
        // Losslessness is about the *source* bytes, so a sealed object must decrypt
        // back to them, not merely sit there as ciphertext.
        let recovered = match (secret, key.as_ref()) {
            (true, Some(current)) => match sealing::open(current, &read_back) {
                Ok(recovered) => recovered,
                Err(error) => {
                    failures.push(Diagnostic::error(
                        "cas_unreadable",
                        file.relative_path.clone(),
                        error,
                    ));
                    continue;
                }
            },
            (true, None) => {
                failures.push(Diagnostic::error(
                    "key_unavailable",
                    file.relative_path.clone(),
                    "封存对象没有可用密钥".to_string(),
                ));
                continue;
            }
            (false, _) => read_back.clone(),
        };
        if recovered != *bytes || sha256_hex(&recovered) != file.content_hash {
            failures.push(Diagnostic::error(
                "cas_hash_mismatch",
                file.relative_path.clone(),
                "原文哈希漂移".to_string(),
            ));
            continue;
        }
        let text = String::from_utf8(recovered.clone()).ok();
        // Use the same credential decision for encryption, metadata and preview.
        // Raw text scrubbing cannot safely display escaped/nested JSON secrets.
        // Structured, redacted records remain available separately.
        let secret_material = secret;
        let display = match text {
            _ if secret => context
                .limits
                .bounded_text("[REDACTED: credential-bearing original]"),
            Some(raw) if raw.len() <= 64 * 1024 => context.limits.bounded_text(&raw),
            Some(raw) => {
                let mut head: String = raw.chars().take(context.limits.text_chars).collect();
                head.push_str("…[truncated]");
                head
            }
            None => String::new(),
        };
        rows.push(StagedObject {
            relative_path: file.relative_path.clone(),
            content_hash: file.content_hash.clone(),
            bytes: file.bytes,
            storage_path: stored,
            display_text: display,
            secret_material,
        });
    }
    if !failures.is_empty() {
        return Err(failures);
    }
    Ok(rows)
}
