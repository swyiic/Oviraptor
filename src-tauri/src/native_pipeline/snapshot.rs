//! §10.2 items 1-4 — the frozen, read-only repository snapshot that every Native code,
//! greybox and CI run is bound to. Nothing in here writes to the source tree; scratch
//! space is a separate directory handed in by the caller.

use crate::artifact_import::canonical::sha256_hex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};
use std::process::Command;

/// §10.2 item 3. Build output, dependency caches and VCS metadata are not source, and
/// leaving them in would turn a repository scan into an archive scan.
pub const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".venv",
    "venv",
    "coverage",
    ".next",
    "out",
    "__pycache__",
];

/// §10.2 item 3 / CODE-003: a monorepo is detected by *all* of its manifests, not by
/// whichever one the walk happened to meet first.
pub const LANGUAGE_MANIFESTS: &[&str] = &[
    "package.json",
    "pnpm-workspace.yaml",
    "lerna.json",
    "turbo.json",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "requirements.txt",
    "pyproject.toml",
    "setup.py",
    "composer.json",
    "Gemfile",
    "Cargo.toml",
    "mix.exs",
];

const MAX_DEPTH: usize = 24;
const MAX_FILES: usize = 100_000;
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Where the requested CI diff base stands. An unusable base must never quietly mean
/// "nothing changed" (§10.4, CI-005).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffBaseState {
    Valid,
    Missing,
    NotAncestor,
    NoRepository,
}

impl DiffBaseState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Missing => "missing",
            Self::NotAncestor => "not_ancestor",
            Self::NoRepository => "no_repository",
        }
    }

    /// Only a `Valid` base may narrow a scan to a diff.
    pub fn is_usable(self) -> bool {
        self == Self::Valid
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotFile {
    pub path: String,
    pub content_hash: String,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct RepositorySnapshot {
    /// Original location is provenance only; tools must read `frozen_root`.
    pub root: PathBuf,
    pub frozen_root: PathBuf,
    pub changed_files: Option<Vec<String>>,
    pub scratch_dir: PathBuf,
    pub commit_sha: String,
    pub base_sha: String,
    pub diff_base: DiffBaseState,
    /// Content hash of the whole frozen manifest. Results cite this, never a path that
    /// may have moved since (§10.2 item 4, CODE-007).
    pub tree_hash: String,
    pub files: Vec<SnapshotFile>,
    pub manifests: Vec<String>,
    pub gaps: Vec<String>,
}

fn is_excluded(name: &str) -> bool {
    EXCLUDED_DIRECTORIES.contains(&name)
}

fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    files: &mut Vec<SnapshotFile>,
    gaps: &mut Vec<String>,
    visited: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        gaps.push(format!("depth_limit:{}", dir.display()));
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|error| error.to_string())?;
    let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    children.sort();
    for path in children {
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            gaps.push(format!("unreadable:{}", path.display()));
            continue;
        };
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        // A link is only usable when it resolves inside the frozen root: following one
        // out of the repository would turn a read-only scan into a read of whatever the
        // developer's home directory holds (§10.2 item 3).
        let mut followed = path.clone();
        if metadata.file_type().is_symlink() {
            let Ok(target) = std::fs::canonicalize(&path) else {
                gaps.push(format!("symlink_unresolved:{name}"));
                continue;
            };
            if !target.starts_with(root) {
                gaps.push(format!("symlink_outside_repository:{name}"));
                continue;
            }
            let Ok(shared) = target.metadata() else {
                gaps.push(format!("symlink_unreachable:{name}"));
                continue;
            };
            if shared.is_dir() {
                // Walking a linked directory would record the same file under the target's
                // path and lose the link's identity, so it is reported instead.
                gaps.push(format!("symlink_directory_not_traversed:{name}"));
                continue;
            }
            if !shared.is_file() {
                gaps.push(format!("special_file_skipped:{name}"));
                continue;
            }
            followed = target;
        }
        if followed.is_dir() {
            if is_excluded(&name) {
                gaps.push(format!("excluded_directory:{name}"));
                continue;
            }
            if visited.contains(&followed) {
                gaps.push(format!("symlink_cycle:{name}"));
                continue;
            }
            visited.push(followed.clone());
            walk(root, &followed, depth + 1, files, gaps, visited)?;
            visited.pop();
            continue;
        }
        if !followed.is_file() {
            gaps.push(format!("special_file_skipped:{name}"));
            continue;
        }
        if files.len() >= MAX_FILES {
            gaps.push(format!("file_count_limit:{MAX_FILES}"));
            return Ok(());
        }
        // The link itself is a handful of bytes; the size cap has to describe the body.
        let size = std::fs::metadata(&followed)
            .map(|shared| shared.len())
            .unwrap_or_else(|_| metadata.len());
        if size > MAX_FILE_BYTES {
            gaps.push(format!("file_too_large:{name}"));
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let text = relative
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("./")
            .to_string();
        let Ok(bytes) = std::fs::read(&followed) else {
            gaps.push(format!("unreadable:{text}"));
            continue;
        };
        files.push(SnapshotFile {
            content_hash: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
            path: text,
        });
    }
    Ok(())
}

fn tree_hash_of(files: &[SnapshotFile]) -> String {
    let joined = files
        .iter()
        .map(|file| format!("{}\u{1}{}\n", file.path, file.content_hash))
        .collect::<String>();
    sha256_hex(joined.as_bytes())
}

/// The canonical root, resolved from the parent so a symlinked source directory still
/// freezes to the tree it points at rather than to the link itself.
fn canonical_root(source_dir: &Path) -> Result<PathBuf, String> {
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut candidate = source_dir.to_path_buf();
    loop {
        if let Ok(resolved) = candidate.canonicalize() {
            let mut root = resolved;
            for part in missing.iter().rev() {
                root = root.join(part);
            }
            return Ok(root);
        }
        let parent = candidate.parent().map(PathBuf::from);
        let name = candidate.file_name().map(|value| value.to_os_string());
        let (Some(parent), Some(name)) = (parent, name) else {
            return Err(format!("无法确定源码根目录：{}", source_dir.display()));
        };
        missing.push(name);
        candidate = parent;
    }
}

fn manifests_of(files: &[SnapshotFile]) -> Vec<String> {
    files
        .iter()
        .filter(|file| {
            file.path
                .rsplit('/')
                .next()
                .is_some_and(|name| LANGUAGE_MANIFESTS.contains(&name))
        })
        .map(|file| file.path.clone())
        .collect()
}

/// `git` in the repository, argv as an array, no shell, no credential prompts.
pub fn git_text(repository: &Path, args: &[&str]) -> Option<String> {
    let bytes = git_bytes(repository, args)?;
    Some(String::from_utf8_lossy(&bytes).trim().to_string())
}

// Path lists are NUL-delimited bytes, never trimmed human-readable text. Disable
// optional index writes and fsmonitor hooks: inspecting a source tree must not
// execute a repository-configured monitor or refresh the developer's index.
fn git_bytes(repository: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .args([
            "--no-optional-locks",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .current_dir(repository)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output.stdout)
}

fn nul_paths(bytes: &[u8]) -> Result<Vec<String>, String> {
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        return Err("diff_path_list_unterminated".into());
    }
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = std::str::from_utf8(path).map_err(|_| "diff_path_not_utf8")?;
            if Path::new(path)
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err("diff_path_not_repository_relative".into());
            }
            Ok(path.to_string())
        })
        .collect()
}

/// Compare the frozen base with the working tree, not just HEAD. Renames are
/// represented as an old-path deletion plus a new path; untracked, non-ignored
/// files are included. All names are relative to the selected source directory.
/// A failed or hidden index is unknown coverage, never an empty successful diff.
fn working_tree_changes(repository: &Path, base_sha: &str) -> Result<Vec<String>, String> {
    let index = git_bytes(repository, &["ls-files", "-v", "-z", "--", "."])
        .ok_or("diff_index_unavailable")?;
    for entry in index
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        if entry.len() < 3 || entry[1] != b' ' {
            return Err("diff_index_invalid".into());
        }
        // Git is allowed to omit working-tree changes under either flag. Do not
        // silently promise a complete diff or clear the operator's index flags.
        if entry[0] == b'S' || entry[0].is_ascii_lowercase() {
            return Err("diff_index_hidden_paths".into());
        }
    }
    let diff = git_bytes(
        repository,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--relative",
            "--name-only",
            "-z",
            base_sha,
            "--",
            ".",
        ],
    )
    .ok_or("diff_working_tree_unavailable")?;
    let untracked = git_bytes(
        repository,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
    )
    .ok_or("diff_untracked_unavailable")?;
    let mut paths = nul_paths(&diff)?;
    paths.extend(nul_paths(&untracked)?);
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn classify_base(repository: &Path, requested: Option<&str>) -> (String, DiffBaseState) {
    let Some(base) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return (String::new(), DiffBaseState::Missing);
    };
    if git_text(repository, &["rev-parse", "--git-dir"]).is_none() {
        return (base.to_string(), DiffBaseState::NoRepository);
    }
    let Some(resolved) = git_text(
        repository,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ],
    ) else {
        return (base.to_string(), DiffBaseState::Missing);
    };
    let head = git_text(repository, &["rev-parse", "HEAD"]).unwrap_or_default();
    // The merge base is what "what changed in this branch" actually means; a base that
    // is not an ancestor of HEAD cannot produce a diff at all.
    let ancestor = git_text(
        repository,
        &["merge-base", "--is-ancestor", &resolved, &head],
    );
    match ancestor {
        Some(_) => (resolved, DiffBaseState::Valid),
        None => (resolved, DiffBaseState::NotAncestor),
    }
}

impl RepositorySnapshot {
    pub fn capture(
        source_dir: &Path,
        scratch_dir: &Path,
        requested_base: Option<&str>,
    ) -> Result<Self, String> {
        if !source_dir.is_dir() {
            return Err("源码目录不存在或不可读取".to_string());
        }
        let root = canonical_root(source_dir)?;
        if canonical_root(scratch_dir)?.starts_with(&root) {
            return Err("scratch_inside_repository: 分析输出目录不能位于源码内".into());
        }
        std::fs::create_dir_all(scratch_dir).map_err(|error| error.to_string())?;
        let mut files = Vec::new();
        let mut gaps = Vec::new();
        walk(
            &root,
            &root,
            0,
            &mut files,
            &mut gaps,
            &mut vec![root.clone()],
        )?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let manifests = manifests_of(&files);
        let commit_sha = git_text(&root, &["rev-parse", "HEAD"]).unwrap_or_default();
        let (base_sha, diff_base) = classify_base(&root, requested_base);
        if commit_sha.is_empty() && diff_base == DiffBaseState::Missing {
            gaps.push("no_git_repository".to_string());
        }
        let changed_files = if diff_base.is_usable() {
            match working_tree_changes(&root, &base_sha) {
                Ok(paths) => Some(paths),
                Err(gap) => {
                    gaps.push(gap);
                    None
                }
            }
        } else {
            None
        };
        let frozen_root = scratch_dir.join(format!("source-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&frozen_root).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&frozen_root, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        // Independent copies, never hard links to mutable user files. Resolve each
        // source again and verify the exact bytes before publishing the manifest.
        for file in &files {
            let source = root
                .join(&file.path)
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if !source.starts_with(&root) {
                return Err("snapshot_source_escaped".into());
            }
            let bytes = read_snapshot_file(&source)?;
            if bytes.len() as u64 != file.bytes || sha256_hex(&bytes) != file.content_hash {
                return Err(format!("snapshot_source_changed:{}", file.path));
            }
            let destination = frozen_root.join(&file.path);
            std::fs::create_dir_all(destination.parent().ok_or("snapshot_path_invalid")?)
                .map_err(|e| e.to_string())?;
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut output = options.open(&destination).map_err(|e| e.to_string())?;
            output.write_all(&bytes).map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())?;
            let mut permissions = output.metadata().map_err(|e| e.to_string())?.permissions();
            permissions.set_readonly(true);
            output
                .set_permissions(permissions)
                .map_err(|e| e.to_string())?;
        }
        let snapshot = Self {
            tree_hash: tree_hash_of(&files),
            root,
            frozen_root,
            changed_files,
            scratch_dir: scratch_dir.to_path_buf(),
            commit_sha,
            base_sha,
            diff_base,
            files,
            manifests,
            gaps,
        };
        snapshot.verify_unchanged()?;
        // Bind the diff to this capture, not to Git metadata changed while files
        // were being copied. Restore reads the stored list and never recomputes it.
        if git_text(&snapshot.root, &["rev-parse", "HEAD"]).unwrap_or_default()
            != snapshot.commit_sha
        {
            return Err("snapshot_git_head_changed_during_capture".into());
        }
        if snapshot.diff_base.is_usable()
            && working_tree_changes(&snapshot.root, &snapshot.base_sha).ok()
                != snapshot.changed_files
        {
            return Err("snapshot_git_diff_changed_during_capture".into());
        }
        Ok(snapshot)
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn content_hash_of(&self, repo_relative: &str) -> Option<&str> {
        let wanted = repo_relative.trim_start_matches("./").replace('\\', "/");
        self.files
            .iter()
            .find(|file| file.path == wanted)
            .map(|file| file.content_hash.as_str())
    }

    /// Languages this repository actually declares, in first-seen path order.
    pub fn languages(&self) -> Vec<String> {
        Self::languages_for_manifests(&self.manifests)
    }

    pub(super) fn languages_for_manifests(manifests: &[String]) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for manifest in manifests {
            let language = match manifest.rsplit('/').next().unwrap_or(manifest.as_str()) {
                "package.json" | "pnpm-workspace.yaml" | "lerna.json" | "turbo.json" => {
                    "javascript"
                }
                "go.mod" => "go",
                "pom.xml" | "build.gradle" | "build.gradle.kts" | "settings.gradle" => "java",
                "requirements.txt" | "pyproject.toml" | "setup.py" => "python",
                "composer.json" => "php",
                "Gemfile" => "ruby",
                "Cargo.toml" => "rust",
                "mix.exs" => "elixir",
                _ => continue,
            };
            if !seen.iter().any(|known| known == language) {
                seen.push(language.to_string());
            }
        }
        seen
    }

    /// §10.2 item 2: the source must still be exactly what was frozen.
    pub fn verify_unchanged(&self) -> Result<(), String> {
        self.verify_frozen()?;
        let mut files = Vec::new();
        let mut gaps = Vec::new();
        walk(
            &self.root,
            &self.root,
            0,
            &mut files,
            &mut gaps,
            &mut vec![self.root.clone()],
        )?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        if files != self.files {
            let changed = files
                .iter()
                .zip(self.files.iter())
                .filter(|(left, right)| {
                    left.path != right.path || left.content_hash != right.content_hash
                })
                .map(|(left, _)| left.path.clone())
                .chain(
                    self.files
                        .iter()
                        .filter(|mine| !files.iter().any(|now| now.path == mine.path))
                        .map(|mine| mine.path.clone()),
                )
                .take(20)
                .collect::<Vec<_>>();
            return Err(format!(
                "源码在扫描期间被改动，结果不可复现：{}",
                changed.join(", ")
            ));
        }
        Ok(())
    }

    pub fn verify_frozen(&self) -> Result<(), String> {
        if self.frozen_root.as_os_str().is_empty() || !self.frozen_root.is_dir() {
            return Err("snapshot_materialization_missing: start a new attempt".into());
        }
        let root = self.frozen_root.canonicalize().map_err(|e| e.to_string())?;
        let mut actual = Vec::new();
        let mut gaps = Vec::new();
        walk(
            &root,
            &root,
            0,
            &mut actual,
            &mut gaps,
            &mut vec![root.clone()],
        )?;
        actual.sort_by(|left, right| left.path.cmp(&right.path));
        if actual != self.files || !gaps.is_empty() {
            return Err("snapshot_materialization_manifest_mismatch".into());
        }
        for file in &self.files {
            let relative = Path::new(&file.path);
            if relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err("snapshot_manifest_path_invalid".into());
            }
            let path = root.join(relative);
            if !path
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&root)
            {
                return Err("snapshot_materialization_escaped".into());
            }
            let bytes = read_snapshot_file(&path)?;
            if bytes.len() as u64 != file.bytes || sha256_hex(&bytes) != file.content_hash {
                return Err(format!("snapshot_materialization_changed:{}", file.path));
            }
        }
        Ok(())
    }

    pub fn as_json(&self) -> JsonValue {
        json!({
            "root": self.root.to_string_lossy(),
            "frozenRoot": self.frozen_root.to_string_lossy(),
            "changedFiles": self.changed_files,
            "scratchDir": self.scratch_dir.to_string_lossy(),
            "commitSha": self.commit_sha,
            "baseSha": self.base_sha,
            "diffBase": self.diff_base.as_str(),
            "treeHash": self.tree_hash,
            "files": self.files.iter().map(|file| json!({
                "path": file.path, "contentHash": file.content_hash, "bytes": file.bytes
            })).collect::<Vec<_>>(),
            "manifests": self.manifests,
            "languages": self.languages(),
            "gaps": self.gaps,
        })
    }

    /// repo-relative path -> content hash, for callers that only need the manifest.
    pub fn manifest_pairs(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .map(|file| (file.path.clone(), file.content_hash.clone()))
            .collect()
    }

    /// §10.2 item 1 / item 11: the freeze has to survive the process that made it, so a
    /// resume reads the same manifest instead of walking a tree that may have moved.
    pub fn store(
        &self,
        connection: &Connection,
        scan_id: &str,
        attempt_number: i64,
    ) -> Result<(), String> {
        let manifest = self
            .files
            .iter()
            .map(|file| {
                json!({"path": file.path, "contentHash": file.content_hash, "bytes": file.bytes})
            })
            .collect::<Vec<_>>();
        connection
            .execute(
                "INSERT OR IGNORE INTO source_snapshots(
                    scan_id,attempt_number,root_path,scratch_dir,commit_sha,base_sha,diff_base,
                    tree_hash,file_count,manifest_json,languages_json,gaps_json,frozen_root,changed_files_json
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![
                    scan_id,
                    attempt_number,
                    self.root.to_string_lossy(),
                    self.scratch_dir.to_string_lossy(),
                    self.commit_sha,
                    self.base_sha,
                    self.diff_base.as_str(),
                    self.tree_hash,
                    self.file_count() as i64,
                    json!(manifest).to_string(),
                    json!(self.languages()).to_string(),
                    json!(self.gaps).to_string(),
                    self.frozen_root.to_string_lossy(),
                    json!(self.changed_files).to_string(),
                ],
            )
            .map_err(|error| format!("无法冻结快照：{error}"))?;
        Ok(())
    }

    /// The stored freeze for this scan and attempt, if one was ever written.
    pub fn restore(
        connection: &Connection,
        scan_id: &str,
        attempt_number: i64,
    ) -> Result<Option<Self>, String> {
        #[allow(clippy::type_complexity)]
        let row: Option<(String, String, String, String, String, String, i64, String, String, String)> = connection
            .query_row(
                "SELECT root_path,scratch_dir,commit_sha,base_sha,diff_base,manifest_json,file_count,frozen_root,changed_files_json,gaps_json
                 FROM source_snapshots WHERE scan_id=?1 AND attempt_number=?2
                 ORDER BY id DESC LIMIT 1",
                params![scan_id, attempt_number],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("无法读取冻结快照：{error}"))?;
        let Some((
            root,
            scratch_dir,
            commit_sha,
            base_sha,
            diff_base,
            manifest_json,
            file_count,
            frozen_root,
            changed_files,
            gaps,
        )) = row
        else {
            return Ok(None);
        };
        let parsed: Vec<JsonValue> = serde_json::from_str(&manifest_json).map_err(|error| {
            format!("冻结快照的文件清单不可解析：{error}（记录 {scan_id}/{attempt_number}）")
        })?;
        let mut files = Vec::new();
        for entry in &parsed {
            let (Some(path), Some(content_hash)) = (
                entry.get("path").and_then(JsonValue::as_str),
                entry.get("contentHash").and_then(JsonValue::as_str),
            ) else {
                continue;
            };
            files.push(SnapshotFile {
                path: path.to_string(),
                content_hash: content_hash.to_string(),
                bytes: entry.get("bytes").and_then(JsonValue::as_u64).unwrap_or(0),
            });
        }
        if files.len() as i64 != file_count {
            return Err(format!(
                "冻结快照的清单与行数不一致（{} 对 {file_count}），拒绝按损坏的冻结继续扫描",
                files.len()
            ));
        }
        let tree_hash = tree_hash_of(&files);
        let manifests = manifests_of(&files);
        Ok(Some(Self {
            root: PathBuf::from(root),
            frozen_root: PathBuf::from(frozen_root),
            changed_files: serde_json::from_str(&changed_files)
                .map_err(|e| format!("snapshot_diff_corrupt:{e}"))?,
            scratch_dir: PathBuf::from(scratch_dir),
            commit_sha,
            base_sha,
            diff_base: match diff_base.as_str() {
                "valid" => DiffBaseState::Valid,
                "not_ancestor" => DiffBaseState::NotAncestor,
                "no_repository" => DiffBaseState::NoRepository,
                _ => DiffBaseState::Missing,
            },
            tree_hash,
            files,
            manifests,
            gaps: serde_json::from_str(&gaps).map_err(|e| format!("snapshot_gaps_corrupt:{e}"))?,
        }))
    }
}

pub(super) fn read_snapshot_file(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    if std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("snapshot_symlink_rejected".into());
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("snapshot_special_file_rejected".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("snapshot_file_too_large".into());
    }
    Ok(bytes)
}
