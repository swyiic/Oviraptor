//! Actual analyzer input, separate from the complete source provenance snapshot.
//! A stored view is never reconstructed from a changed checkout or silently widened.
use super::snapshot::{read_snapshot_file, RepositorySnapshot, SnapshotFile};
use crate::artifact_import::canonical::sha256_hex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalysisManifest {
    pub schema_version: u32,
    pub scan_id: String,
    pub attempt_number: i64,
    pub request: Value,
    pub source_binding: String,
    pub source_tree_hash: String,
    pub head_sha: String,
    pub base_sha: String,
    pub diff_base_state: String,
    pub scope: String,
    pub fallback_reason: Option<String>,
    pub files: Vec<SnapshotFile>,
    /// Deletions and excluded/unreadable changes cannot be distinguished from the
    /// persisted diff name list. Do not invent a verified deletion classification.
    pub changed_paths_without_content: Vec<String>,
    pub gaps: Vec<String>,
}

impl AnalysisManifest {
    pub fn select(
        snapshot: &RepositorySnapshot,
        scan_id: &str,
        attempt_number: i64,
        request: &Value,
    ) -> Result<Self, String> {
        if request["analysisPolicyVersion"].as_i64() != Some(1)
            || request["canonicalRoot"].as_str() != snapshot.root.to_str()
            || scan_id.is_empty()
            || attempt_number < 1
        {
            return Err("analysis_scope_request_binding_invalid".into());
        }
        let requested = request["scopeMode"]
            .as_str()
            .ok_or("analysis_scope_request_missing")?;
        let usable = snapshot.diff_base.is_usable() && snapshot.changed_files.is_some();
        let reason = if snapshot.diff_base.is_usable() {
            "diff_manifest_unavailable".to_string()
        } else {
            format!("diff_base_{}", snapshot.diff_base.as_str())
        };
        let (scope, fallback_reason) = match requested {
            "full" => ("full", None),
            "auto" if !usable => ("full", Some(reason.clone())),
            "diff" if !usable => ("unavailable", None),
            "auto" | "diff" => ("diff", None),
            _ => return Err("analysis_scope_request_invalid".into()),
        };
        let changed: BTreeSet<&str> = snapshot
            .changed_files
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .collect();
        let files: Vec<_> = snapshot
            .files
            .iter()
            .filter(|file| {
                scope == "full" || (scope == "diff" && changed.contains(file.path.as_str()))
            })
            .cloned()
            .collect();
        for file in &files {
            validate_relative(&file.path)?;
        }
        let present: BTreeSet<&str> = snapshot
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        let changed_paths_without_content: Vec<String> = if scope == "diff" {
            changed
                .difference(&present)
                .map(|path| (*path).to_string())
                .collect()
        } else {
            vec![]
        };
        let mut gaps = Vec::new();
        if scope == "unavailable" {
            gaps.push(format!("analysis_scope_unavailable:{reason}"));
        }
        if scope == "diff" && files.is_empty() {
            gaps.push("analysis_diff_no_materialized_files".into());
        }
        if !changed_paths_without_content.is_empty() {
            gaps.push("analysis_changed_paths_without_content".into());
        }
        Ok(Self {
            schema_version: 1,
            scan_id: scan_id.into(),
            attempt_number,
            request: request.clone(),
            source_binding: sha256_hex(snapshot.as_json().to_string().as_bytes()),
            source_tree_hash: snapshot.tree_hash.clone(),
            head_sha: snapshot.commit_sha.clone(),
            base_sha: snapshot.base_sha.clone(),
            diff_base_state: snapshot.diff_base.as_str().into(),
            scope: scope.into(),
            fallback_reason,
            files,
            changed_paths_without_content,
            gaps,
        })
    }

    pub fn digest(&self) -> String {
        sha256_hex(
            serde_json::to_vec(self)
                .expect("serializable manifest")
                .as_slice(),
        )
    }
}

#[derive(Clone, Debug)]
pub struct SourceAnalysisView {
    pub manifest: AnalysisManifest,
    root: PathBuf,
    receipt_text: String,
    receipt_digest: String,
}

fn validate_relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\\')
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("analysis_view_path_invalid".into());
    }
    Ok(())
}

impl SourceAnalysisView {
    fn new(manifest: AnalysisManifest, root: PathBuf) -> Result<Self, String> {
        let receipt_text = serde_json::to_string(&manifest).map_err(|e| e.to_string())?;
        let receipt_digest = sha256_hex(receipt_text.as_bytes());
        Ok(Self {
            manifest,
            root,
            receipt_text,
            receipt_digest,
        })
    }

    pub fn materialize(
        snapshot: &RepositorySnapshot,
        manifest: AnalysisManifest,
    ) -> Result<Self, String> {
        snapshot.verify_frozen()?;
        let scratch = snapshot
            .scratch_dir
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let root = scratch.join(format!("analysis-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        for file in &manifest.files {
            validate_relative(&file.path)?;
            let bytes = read_snapshot_file(&snapshot.frozen_root.join(&file.path))?;
            if bytes.len() as u64 != file.bytes || sha256_hex(&bytes) != file.content_hash {
                return Err("analysis_view_source_changed".into());
            }
            let destination = root.join(&file.path);
            std::fs::create_dir_all(destination.parent().ok_or("analysis_view_parent_missing")?)
                .map_err(|e| e.to_string())?;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            let mut output = options.open(destination).map_err(|e| e.to_string())?;
            output.write_all(&bytes).map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())?;
            let mut permissions = output.metadata().map_err(|e| e.to_string())?.permissions();
            permissions.set_readonly(true);
            output
                .set_permissions(permissions)
                .map_err(|e| e.to_string())?;
        }
        let view = Self::new(manifest, root)?;
        view.verify(snapshot)?;
        Ok(view)
    }

    pub fn repository(&self) -> &Path {
        &self.root
    }

    pub fn as_json(&self) -> Value {
        json!({ "manifest": self.manifest, "digest": self.manifest.digest(), "fileCount": self.manifest.files.len() })
    }

    pub fn verify(&self, snapshot: &RepositorySnapshot) -> Result<(), String> {
        let expected = AnalysisManifest::select(
            snapshot,
            &self.manifest.scan_id,
            self.manifest.attempt_number,
            &self.manifest.request,
        )?;
        if expected != self.manifest || self.receipt_digest != self.manifest.digest() {
            return Err("analysis_view_snapshot_binding_mismatch".into());
        }
        let metadata = std::fs::symlink_metadata(&self.root).map_err(|e| e.to_string())?;
        let scratch = snapshot
            .scratch_dir
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || self.root.parent() != Some(scratch.as_path())
            || !self
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .starts_with("analysis-")
            || self.root.canonicalize().map_err(|e| e.to_string())? != self.root
        {
            return Err("analysis_view_root_invalid".into());
        }
        let expected: BTreeSet<&str> = self
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        let mut expected_directories = BTreeSet::new();
        for file in &self.manifest.files {
            let mut path = Path::new(&file.path).parent();
            while let Some(parent) = path.filter(|p| !p.as_os_str().is_empty()) {
                expected_directories.insert(parent.to_string_lossy().replace('\\', "/"));
                path = parent.parent();
            }
        }
        let mut actual = BTreeSet::new();
        let mut pending = vec![(self.root.clone(), 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            if depth > 24 {
                return Err("analysis_view_depth_limit".into());
            }
            for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if metadata.file_type().is_symlink() {
                    return Err("analysis_view_symlink".into());
                }
                let relative = path
                    .strip_prefix(&self.root)
                    .map_err(|e| e.to_string())?
                    .to_str()
                    .ok_or("analysis_view_path_encoding")?
                    .replace('\\', "/");
                if metadata.is_dir() {
                    if !expected_directories.contains(&relative) {
                        return Err("analysis_view_extra_directory".into());
                    }
                    pending.push((path, depth + 1));
                } else if metadata.is_file() && expected.contains(relative.as_str()) {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        if metadata.nlink() != 1 {
                            return Err("analysis_view_hardlink".into());
                        }
                    }
                    actual.insert(relative);
                } else {
                    return Err("analysis_view_extra_or_special_file".into());
                }
            }
        }
        if actual.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected {
            return Err("analysis_view_missing_file".into());
        }
        for file in &self.manifest.files {
            let bytes = read_snapshot_file(&self.root.join(&file.path))?;
            if bytes.len() as u64 != file.bytes || sha256_hex(&bytes) != file.content_hash {
                return Err("analysis_view_content_changed".into());
            }
        }
        Ok(())
    }

    pub fn store(&self, connection: &Connection) -> Result<(), String> {
        let changed = connection.execute(
            "INSERT INTO source_analysis_views(scan_id,attempt_number,manifest_json,manifest_digest,view_root) VALUES(?1,?2,?3,?4,?5)",
            params![self.manifest.scan_id,self.manifest.attempt_number,self.receipt_text,
                self.receipt_digest,self.root.to_str().ok_or("analysis_view_path_encoding")?]).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("analysis_view_publication_ignored".into());
        }
        self.verify_receipt(connection)
    }

    pub fn restore(
        connection: &Connection,
        expected: &AnalysisManifest,
        snapshot: &RepositorySnapshot,
    ) -> Result<Self, String> {
        let row: Option<(String,String,String)> = connection.query_row(
            "SELECT manifest_json,manifest_digest,view_root FROM source_analysis_views WHERE scan_id=?1 AND attempt_number=?2",
            params![expected.scan_id,expected.attempt_number], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))
            .optional().map_err(|e| e.to_string())?;
        let (text, digest, root) = row.ok_or("analysis_view_receipt_missing_start_new_attempt")?;
        let manifest: AnalysisManifest = serde_json::from_str(&text)
            .map_err(|e| format!("analysis_view_receipt_corrupt:{e}"))?;
        if &manifest != expected || manifest.digest() != digest {
            return Err("analysis_view_receipt_mismatch".into());
        }
        let view = Self::new(manifest, PathBuf::from(root))?;
        view.verify(snapshot)?;
        Ok(view)
    }

    pub fn verify_receipt(&self, connection: &Connection) -> Result<(), String> {
        let matches = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_analysis_views WHERE scan_id=?1 AND attempt_number=?2 AND manifest_json=?3 AND manifest_digest=?4 AND view_root=?5)",
            params![self.manifest.scan_id,self.manifest.attempt_number,self.receipt_text,
                self.receipt_digest,self.root.to_str().ok_or("analysis_view_path_encoding")?], |row| row.get::<_,bool>(0)).map_err(|e| e.to_string())?;
        if !matches {
            return Err("analysis_view_receipt_changed".into());
        }
        Ok(())
    }

    pub fn verify_source_receipt(&self, connection: &Connection) -> Result<(), String> {
        let (count, matching): (i64,i64) = connection.query_row(
            "SELECT count(*),coalesce(sum(tree_hash=?3),0) FROM source_snapshots WHERE scan_id=?1 AND attempt_number=?2",
            params![self.manifest.scan_id,self.manifest.attempt_number,self.manifest.source_tree_hash],
            |row| Ok((row.get(0)?,row.get(1)?))).map_err(|e|e.to_string())?;
        if count != 1 || matching != 1 {
            return Err("analysis_source_receipt_missing_or_ambiguous".into());
        }
        let restored = RepositorySnapshot::restore(
            connection,
            &self.manifest.scan_id,
            self.manifest.attempt_number,
        )?
        .ok_or("analysis_source_receipt_missing")?;
        if sha256_hex(restored.as_json().to_string().as_bytes()) != self.manifest.source_binding {
            return Err("analysis_source_receipt_changed".into());
        }
        Ok(())
    }
}
