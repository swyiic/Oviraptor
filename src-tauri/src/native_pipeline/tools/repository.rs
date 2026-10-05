//! Bounded, snapshot-verified repository read tools for source specialists.

use super::super::snapshot::{read_snapshot_file, DiffBaseState, RepositorySnapshot};
use super::{BrokerDenial, SourceBroker};
use crate::artifact_import::canonical::sha256_hex;
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};

const MAX_SLICE_LINES: usize = 400;
const MAX_SEARCH_FILES: usize = 2_000;
const MAX_SEARCH_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SEARCH_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SEARCH_QUERY_BYTES: usize = 256;
pub(super) const MAX_RESULTS: usize = 100;

impl SourceBroker {
    /// Rejects absolute paths, `..` escapes and anything not in the frozen manifest.
    pub(super) fn check_relative(&self, raw: &str) -> Result<(), BrokerDenial> {
        let text = raw;
        if text.is_empty() {
            return Ok(());
        }
        if text.starts_with('/') || text.starts_with('\\') || Path::new(text).is_absolute() {
            return Err(BrokerDenial::new(
                "absolute_path",
                format!("只允许仓库内相对路径，收到绝对路径：{text}"),
            ));
        }
        if cfg!(windows) && (text.as_bytes().get(1) == Some(&b':') || text.starts_with("\\\\")) {
            return Err(BrokerDenial::new(
                "absolute_path",
                format!("只允许仓库内相对路径，收到盘符路径：{text}"),
            ));
        }
        if text
            .split('/')
            .any(|part| part == ".." || part == "." || part.contains('\0'))
            || text.contains('\\')
        {
            return Err(BrokerDenial::new(
                "path_escape",
                format!("路径不得越出仓库：{text}"),
            ));
        }
        Ok(())
    }

    pub(super) fn normalized(raw: &str) -> String {
        // The frozen manifest preserves exact names, including spaces, newlines
        // and leading dots. Never turn a requested name into a different file.
        raw.to_string()
    }

    fn prefix_of(map: &serde_json::Map<String, JsonValue>) -> String {
        Self::normalized(map.get("prefix").and_then(JsonValue::as_str).unwrap_or(""))
    }

    pub(super) fn limit_of(
        map: &serde_json::Map<String, JsonValue>,
        default: usize,
        max: usize,
    ) -> usize {
        map.get("limit")
            .and_then(JsonValue::as_u64)
            .map(|value| (value as usize).clamp(1, max))
            .unwrap_or(default)
    }

    pub(super) fn inventory(
        &self,
        map: &serde_json::Map<String, JsonValue>,
    ) -> Result<JsonValue, BrokerDenial> {
        let prefix = Self::prefix_of(map);
        let limit = Self::limit_of(map, MAX_RESULTS, MAX_RESULTS);
        let files: Vec<JsonValue> = self
            .selected_files()
            .iter()
            .filter(|file| prefix.is_empty() || file.path.starts_with(&prefix))
            .take(limit)
            .map(|file| {
                json!({"path": file.path, "contentHash": file.content_hash, "bytes": file.bytes})
            })
            .collect();
        Ok(json!({
            "treeHash": self.snapshot.tree_hash,
            "commitSha": self.snapshot.commit_sha,
            "fileCount": self.selected_files().len(),
            "languages": RepositorySnapshot::languages_for_manifests(&self.selected_manifests()),
            "manifests": self.selected_manifests(),
            "files": files,
            "gaps": self.known_gaps(),
            "analysisManifestDigest": self.analysis_view.as_ref().map(|view|view.manifest.digest()),
        }))
    }

    pub(super) fn search(
        &self,
        map: &serde_json::Map<String, JsonValue>,
    ) -> Result<JsonValue, BrokerDenial> {
        let query = map
            .get("query")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                BrokerDenial::new("invalid_arguments", "repo.search 需要非空的 query 字面量")
            })?;
        if query.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(BrokerDenial::new(
                "invalid_arguments",
                format!("repo.search 的 query 不得超过 {MAX_SEARCH_QUERY_BYTES} 字节"),
            ));
        }
        let ignore_case = map
            .get("ignoreCase")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true);
        let prefix = Self::prefix_of(map);
        let limit = Self::limit_of(map, 50, MAX_RESULTS);
        let needle = if ignore_case {
            query.to_lowercase()
        } else {
            query.clone()
        };
        let mut matches = Vec::new();
        let mut scanned = 0usize;
        let mut scanned_bytes = 0u64;
        let mut truncated = false;
        for file in self.selected_files() {
            if scanned >= MAX_SEARCH_FILES {
                truncated = true;
                break;
            }
            if !prefix.is_empty() && !file.path.starts_with(&prefix) {
                continue;
            }
            if file.bytes > MAX_SEARCH_BYTES {
                truncated = true;
                continue;
            }
            if scanned_bytes.saturating_add(file.bytes) > MAX_SEARCH_TOTAL_BYTES {
                truncated = true;
                break;
            }
            scanned += 1;
            scanned_bytes += file.bytes;
            let Ok(text) = self.read_frozen(&file.path) else {
                truncated = true;
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                let haystack = if ignore_case {
                    line.to_lowercase()
                } else {
                    line.to_string()
                };
                if haystack.contains(&needle) {
                    if matches.len() >= limit {
                        truncated = true;
                        break;
                    }
                    matches.push(json!({
                        "path": file.path,
                        "line": index + 1,
                        "preview": line.trim().chars().take(240).collect::<String>(),
                        "contentHash": file.content_hash,
                    }));
                }
            }
        }
        Ok(json!({
            "query": query,
            "matches": matches,
            "scannedFiles": scanned,
            "scannedBytes": scanned_bytes,
            "truncated": truncated,
            "treeHash": self.snapshot.tree_hash,
        }))
    }

    /// Reads a file back through the frozen manifest: the bytes must still hash to what
    /// the snapshot recorded, so a mid-scan edit can never be quoted as evidence.
    pub(super) fn read_frozen(&self, relative: &str) -> Result<String, BrokerDenial> {
        let path = self.relative_path(relative)?;
        let bytes = read_snapshot_file(&path).map_err(|error| {
            BrokerDenial::new("unreadable", format!("无法读取 {relative}：{error}"))
        })?;
        let recorded = self.selected_hash(relative).unwrap_or_default();
        if sha256_hex(&bytes) != recorded {
            return Err(BrokerDenial::new(
                "snapshot_mismatch",
                format!("{relative} 的内容已经和冻结快照不一致，本轮结果不可复现"),
            ));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn relative_path(&self, relative: &str) -> Result<PathBuf, BrokerDenial> {
        let text = Self::normalized(relative);
        self.check_relative(relative)?;
        if self.selected_hash(&text).is_none() {
            return Err(BrokerDenial::new(
                "path_not_in_snapshot",
                format!("{text} 不在本轮获准分析的文件清单里"),
            ));
        }
        let root = self
            .analysis_view
            .as_ref()
            .map_or(self.snapshot.frozen_root.as_path(), |view| {
                view.repository()
            });
        Ok(root.join(&text))
    }

    pub(super) fn read_slice(
        &self,
        map: &serde_json::Map<String, JsonValue>,
    ) -> Result<JsonValue, BrokerDenial> {
        let raw = map
            .get("path")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                BrokerDenial::new("invalid_arguments", "repo.read_slice 需要 path 相对路径")
            })?;
        self.check_relative(&raw)?;
        let text = self.read_frozen(&raw)?;
        let start = map
            .get("startLine")
            .and_then(JsonValue::as_u64)
            .map(|value| value.max(1) as usize)
            .unwrap_or(1);
        let max = map
            .get("maxLines")
            .and_then(JsonValue::as_u64)
            .map(|value| (value as usize).clamp(1, MAX_SLICE_LINES))
            .unwrap_or(MAX_SLICE_LINES);
        let lines: Vec<String> = text
            .lines()
            .skip(start.saturating_sub(1))
            .take(max)
            .map(|line| line.chars().take(400).collect())
            .collect();
        let total = text.lines().count();
        Ok(json!({
            "path": Self::normalized(&raw),
            "contentHash": self.selected_hash(&raw).unwrap_or_default(),
            "startLine": start,
            "lines": lines,
            "endLine": if lines.is_empty() { start - 1 } else { start + lines.len() - 1 },
            "totalLines": total,
            "truncated": start.saturating_sub(1) + lines.len() < total,
        }))
    }

    pub(super) fn changed_files(&self) -> Result<JsonValue, BrokerDenial> {
        if self.snapshot.diff_base != DiffBaseState::Valid {
            return Err(BrokerDenial::new(
                "diff_base_unavailable",
                format!(
                    "diff base 状态是 {}，不能按增量范围回答；这是一条覆盖缺口",
                    self.snapshot.diff_base.as_str()
                ),
            ));
        }
        let head = self.snapshot.commit_sha.clone();
        let base = self.snapshot.base_sha.clone();
        for sha in [&head, &base] {
            if sha.is_empty() || !sha.chars().all(|character| character.is_ascii_hexdigit()) {
                return Err(BrokerDenial::new(
                    "diff_base_unavailable",
                    "commit 标识不是十六进制值，已拒绝执行 git 命令",
                ));
            }
        }
        let Some(files) = &self.snapshot.changed_files else {
            return Err(BrokerDenial::new(
                "diff_base_unavailable",
                "冻结时未获得 git diff 清单，不能重新读取可变仓库",
            ));
        };
        Ok(json!({
            "base": base,
            "head": head,
            "files": files.iter().take(MAX_RESULTS).cloned().collect::<Vec<_>>(),
            "fileCount": files.len(),
            "truncated": files.len() > MAX_RESULTS,
        }))
    }
}
