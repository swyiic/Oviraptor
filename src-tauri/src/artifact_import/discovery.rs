//! §13 COR-006 — finding bundles without following anything dangerous. A symlink,
//! FIFO, socket or device node is rejected with a code; only regular files below a
//! recognized bundle root are ever opened.

use super::diagnostics::Diagnostic;
use super::manifest::normalize_relative;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 8;

/// A directory holding one of these is a result bundle worth importing.
const ANCHORS: &[&str] = &[
    "findings.sarif",
    "oviraptor_recon.json",
    "asset_atlas_recon.json",
    "llm-hook.jsonl",
    "model-prompt-audit.json",
];

const STAGE_PREFIXES: &[&str] = &["s1", "s2", "s3", "s4", "s5", "summary"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    pub root: PathBuf,
    pub source_dir: PathBuf,
    /// `attempt-0002` when the bundle sits under an attempt directory, else empty.
    pub attempt_key: String,
    pub files: Vec<PathBuf>,
    /// An incomplete enumeration cannot replace a previously imported projection.
    pub discovery_errors: Vec<Diagnostic>,
}

fn is_stage_file(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    path.extension().and_then(|value| value.to_str()) == Some("json")
        && STAGE_PREFIXES
            .iter()
            .any(|prefix| stem.to_ascii_lowercase().starts_with(prefix))
}

/// Files that belong to a bundle: recognized artifacts plus stage/summary/meta.
fn is_artifact(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if name == "meta.json" || super::adapters::source_report::is_json_name(name) {
        return true;
    }
    ANCHORS.contains(&name) || extension.eq_ignore_ascii_case("sarif") || is_stage_file(path)
}

fn attempt_key_of(root: &Path, dir: &Path) -> String {
    let relative = dir.strip_prefix(root).unwrap_or(dir);
    let parts: Vec<String> = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .map(str::to_string)
        .collect();
    parts
        .iter()
        .find(|part| part.starts_with("attempt-"))
        .cloned()
        .unwrap_or_default()
}

pub fn discover(roots: &[&Path], limits_depth: usize) -> (Vec<Bundle>, Vec<Diagnostic>) {
    let depth_cap = limits_depth.clamp(1, MAX_DEPTH);
    let mut bundles = Vec::new();
    let mut diagnostics = Vec::new();
    for root in roots.iter() {
        // A trailing slash or `/.` can make stat follow a terminal directory
        // symlink. Normalize syntax only, never canonicalize through the link.
        let inspection_root: PathBuf = root.components().collect();
        let metadata = match std::fs::symlink_metadata(&inspection_root) {
            Ok(metadata) => metadata,
            // Default historical directories need not exist on a fresh install.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    "unreadable_root_metadata",
                    root.display().to_string(),
                    format!("无法确认导入根目录类型：{error}"),
                ));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            diagnostics.push(Diagnostic::error(
                "symlink_rejected",
                root.display().to_string(),
                "导入根目录不能是符号链接".to_string(),
            ));
            continue;
        }
        if !metadata.is_dir() {
            diagnostics.push(Diagnostic::error(
                if metadata.is_file() {
                    "root_not_directory"
                } else {
                    "special_file_rejected"
                },
                root.display().to_string(),
                "导入根必须是普通目录".to_string(),
            ));
            continue;
        }
        walk(root, root, 0, depth_cap, &mut bundles, &mut diagnostics);
    }
    bundles.sort_by(|left, right| left.source_dir.cmp(&right.source_dir));
    (bundles, diagnostics)
}

fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    depth_cap: usize,
    bundles: &mut Vec<Bundle>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if depth > depth_cap {
        diagnostics.push(Diagnostic::error(
            "depth_limit",
            dir.display().to_string(),
            format!("超过 {depth_cap} 层，不再向下查找"),
        ));
        return;
    }
    let mut errors = Vec::new();
    let (mut files, subdirectories) = read_directory(dir, root, &mut errors);
    if !files.is_empty() {
        for subdirectory in &subdirectories {
            collect_bundle_files(
                subdirectory,
                root,
                depth_cap - depth,
                &mut files,
                &mut errors,
            );
        }
        diagnostics.extend(errors.iter().cloned());
        bundles.push(Bundle {
            root: root.to_path_buf(),
            source_dir: dir.to_path_buf(),
            attempt_key: attempt_key_of(root, dir),
            files,
            discovery_errors: errors,
        });
        return;
    }
    diagnostics.extend(errors);
    for subdirectory in subdirectories {
        walk(
            root,
            &subdirectory,
            depth + 1,
            depth_cap,
            bundles,
            diagnostics,
        );
    }
}

/// Everything below a bundle directory belongs to that bundle; no further anchors.
fn collect_bundle_files(
    dir: &Path,
    root: &Path,
    remaining_depth: usize,
    collected: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if remaining_depth == 0 {
        diagnostics.push(Diagnostic::error(
            "depth_limit",
            dir.display().to_string(),
            "超过目录深度上限，结果包不完整，本轮不导入",
        ));
        return;
    }
    let (files, subdirectories) = read_directory(dir, root, diagnostics);
    collected.extend(files);
    for subdirectory in subdirectories {
        collect_bundle_files(
            &subdirectory,
            root,
            remaining_depth - 1,
            collected,
            diagnostics,
        );
    }
}

/// One enumeration policy for both anchor discovery and bundle descendants.
/// Never turn a read/metadata failure into an apparently empty directory.
fn read_directory(
    dir: &Path,
    root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut subdirectories = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        diagnostics.push(Diagnostic::error(
            "unreadable_directory",
            dir.display().to_string(),
            "无法读取目录".to_string(),
        ));
        return (files, subdirectories);
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    "unreadable_directory_entry",
                    dir.display().to_string(),
                    format!("无法完整枚举目录：{error}"),
                ));
                continue;
            }
        };
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    "unreadable_entry_metadata",
                    path.display().to_string(),
                    format!("无法确认目录条目类型：{error}"),
                ));
                continue;
            }
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            diagnostics.push(Diagnostic::error(
                "symlink_rejected",
                path.display().to_string(),
                "不跟随符号链接".to_string(),
            ));
            continue;
        }
        if file_type.is_dir() {
            subdirectories.push(path);
            continue;
        }
        if !file_type.is_file() {
            diagnostics.push(Diagnostic::error(
                "special_file_rejected",
                path.display().to_string(),
                "只接受普通文件，拒绝 FIFO/设备文件".to_string(),
            ));
            continue;
        }
        if normalize_relative(root, &path).is_none() {
            diagnostics.push(Diagnostic::error(
                "path_traversal_rejected",
                path.display().to_string(),
                "相对路径越界".to_string(),
            ));
            continue;
        }
        if !is_artifact(&path) {
            continue;
        }
        files.push(path);
    }
    files.sort();
    subdirectories.sort();
    (files, subdirectories)
}
