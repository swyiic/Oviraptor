//! Offline adapter over the actual application fingerprint-pack module.
//! Run through run_fingerprint_pack_cli.py. No application startup or database I/O.
#![allow(dead_code)]

#[path = "../src-tauri/src/scanning/fingerprint_logic.rs"]
mod fingerprint_logic;

#[path = "../src-tauri/src/scanning/fingerprint_pack.rs"]
mod fingerprint_pack;
mod current_snapshot {
    include!(env!("OVI_CURRENT_SNAPSHOT"));
}

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

// Only the unused Tauri wrappers require this shape. No AppState is instantiated.
struct AppState {
    app_data_dir: PathBuf,
}

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("fingerprint-offline-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_pack(path: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Input must be an ordinary JSON file".into());
    }
    if metadata.len() > fingerprint_pack::MAX_PACK_BYTES as u64 {
        return Err("Input exceeds the production 5 MiB limit".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(fingerprint_pack::MAX_PACK_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > fingerprint_pack::MAX_PACK_BYTES {
        return Err("Input grew beyond 5 MiB".into());
    }
    String::from_utf8(bytes).map_err(|_| "Input is not UTF-8".into())
}
fn data_directory(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if path
        .extension()
        .is_some_and(|e| e == "db" || e == "sqlite" || e == "sqlite3")
        || path.is_file()
    {
        return Err("DATA_DIR must be a directory, never a database file".into());
    }
    Ok(path)
}
fn save_new(path: &Path, content: &str, data_dir: Option<&Path>) -> Result<(), String> {
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        return Err("Output must have a .json extension".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    if let Some(data_dir) = data_dir {
        if let Ok(data_dir) = data_dir.canonicalize() {
            if parent.starts_with(data_dir) {
                return Err("Export must be outside the live data directory".into());
            }
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            format!("Cannot create new output (existing outputs are never overwritten): {e}")
        })?;
    if let Err(error) = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.to_string());
    }
    Ok(())
}
fn export(directory: &Path, scope: &str, output: &Path) -> Result<Value, String> {
    let result = fingerprint_pack::export_pack(directory, scope)?;
    let content = result["content"].as_str().ok_or("Export content missing")?;
    save_new(output, content, Some(directory))?;
    Ok(json!({"output": output, "count":result["count"], "sha256":result["sha256"]}))
}
fn usage() -> String {
    "Usage: validate-current PACK | smoke-current DATA_DIR PACK | validate PACK | validate-rules INPUT OUTPUT | normalize PACK OUTPUT | inspect DATA_DIR | preview DATA_DIR PACK | import DATA_DIR PACK EXPECTED_REVISION | export DATA_DIR user|effective OUTPUT".into()
}
fn run(args: &[String]) -> Result<Value, String> {
    match args.first().map(String::as_str) {
        Some("fresh-import-smoke") if args.len() == 2 => {
            let temporary = Temporary::new()?;
            current_snapshot::fresh_import_smoke(&temporary.0, &read_pack(Path::new(&args[1]))?)
        }
        Some("audit-responses") if args.len() == 3 => current_snapshot::audit_responses(
            &read_pack(Path::new(&args[1]))?,
            &read_pack(Path::new(&args[2]))?,
        ),
        Some("validate-current") if args.len() == 2 => {
            current_snapshot::validate_current(&read_pack(Path::new(&args[1]))?)
        }
        Some("smoke-current") if args.len() == 3 => current_snapshot::smoke_current(
            &data_directory(&args[1])?,
            &read_pack(Path::new(&args[2]))?,
        ),
        Some("validate") if args.len() == 2 => {
            let text = read_pack(Path::new(&args[1]))?;
            let rules = fingerprint_pack::validated_pack_rules(&text)?;
            Ok(json!({"valid":true,"count":rules.as_array().ok_or("Missing rules")?.len()}))
        }
        Some("validate-rules") if args.len() == 3 => {
            let input: Value = serde_json::from_str(&read_pack(Path::new(&args[1]))?)
                .map_err(|e| e.to_string())?;
            let rules = input["fingerprints"]
                .as_array()
                .ok_or("Expected fingerprints array")?;
            let rows = fingerprint_pack::validate_rule_values(rules.clone());
            let valid = rows.iter().filter(|r| r["valid"] == true).count();
            let report =
                json!({"total":rows.len(),"valid":valid,"invalid":rows.len()-valid,"results":rows});
            save_new(
                Path::new(&args[2]),
                &serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
                None,
            )?;
            Ok(
                json!({"output":args[2],"total":rules.len(),"valid":valid,"invalid":rules.len()-valid}),
            )
        }
        Some("normalize-boolean") if args.len() == 3 => {
            let result = fingerprint_pack::normalize_pack(&read_pack(Path::new(&args[1]))?, true)?;
            save_new(
                Path::new(&args[2]),
                result["content"]
                    .as_str()
                    .ok_or("Missing normalized content")?,
                None,
            )?;
            Ok(json!({"output":args[2],"count":result["count"],"sha256":result["sha256"]}))
        }
        Some("preview-boolean") if args.len() == 3 => fingerprint_pack::preview_boolean_pack(
            &data_directory(&args[1])?,
            &read_pack(Path::new(&args[2]))?,
        ),
        Some("import-boolean") if args.len() == 4 => fingerprint_pack::import_boolean_pack(
            &data_directory(&args[1])?,
            &read_pack(Path::new(&args[2]))?,
            &args[3],
        ),
        Some("normalize") if args.len() == 3 => {
            let result = fingerprint_pack::normalize_pack(&read_pack(Path::new(&args[1]))?, false)?;
            save_new(
                Path::new(&args[2]),
                result["content"]
                    .as_str()
                    .ok_or("Missing normalized content")?,
                None,
            )?;
            Ok(json!({"output":args[2],"count":result["count"],"sha256":result["sha256"]}))
        }
        Some("inspect") if args.len() == 2 => {
            let directory = data_directory(&args[1])?;
            let path = fingerprint_pack::user_rules_path(&directory);
            let user = fingerprint_pack::load_user_fingerprints(Some(&directory))?;
            let effective = fingerprint_pack::effective_fingerprints(Some(&directory))?;
            let revision = if path.exists() {
                format!("{:x}", Sha256::digest(read_pack(&path)?.as_bytes()))
            } else {
                "missing".into()
            };
            Ok(
                json!({"path":path,"booleanPath":fingerprint_pack::boolean_rules_path(&directory),"revision":revision,"legacyCount":user.iter().filter(|r|r.condition.is_none()).count(),"booleanCount":user.iter().filter(|r|r.condition.is_some()).count(),"userCount":user.len(),"effectiveCount":effective.len()}),
            )
        }
        Some("preview") if args.len() == 3 => fingerprint_pack::preview_pack(
            &data_directory(&args[1])?,
            &read_pack(Path::new(&args[2]))?,
            false,
        ),
        Some("import") if args.len() == 4 => fingerprint_pack::import_pack(
            &data_directory(&args[1])?,
            &read_pack(Path::new(&args[2]))?,
            false,
            &args[3],
        ),
        Some("export") if args.len() == 4 => {
            export(&data_directory(&args[1])?, &args[2], Path::new(&args[3]))
        }
        _ => Err(usage()),
    }
}
fn main() {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(value) => println!("{}", serde_json::to_string(&value).unwrap()),
        Err(error) => {
            eprintln!("{}", json!({"error":error}));
            std::process::exit(1);
        }
    }
}
