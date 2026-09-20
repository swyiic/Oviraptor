use crate::{db, llm_hook, models::*, AppState};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use csv::ReaderBuilder;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, OptionalExtension, Row};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};
use tauri::{image::Image, AppHandle, Emitter, Manager, State};
use uuid::Uuid;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(windows)]
fn configure_child_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_child_command(_command: &mut Command) {}

fn configure_strix_console(command: &mut Command) {
    configure_child_command(command);
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8:backslashreplace")
        .env_remove("PYTHONLEGACYWINDOWSSTDIO")
        .env("RICH_FORCE_TERMINAL", "0")
        .env("FORCE_COLOR", "0")
        .env("CLICOLOR", "0")
        .env("TERM", "dumb")
        .env("NO_COLOR", "1");
}

fn json(text: String) -> JsonValue {
    serde_json::from_str(&text).unwrap_or_else(|_| JsonValue::Object(Default::default()))
}

include!("commands/workspace_projects.rs");
include!("commands/assets.rs");
include!("commands/asset_ownership.rs");
include!("commands/exposure.rs");
include!("commands/hackerone.rs");
include!("commands/environment.rs");
include!("commands/src_assurance.rs");
include!("commands/web_investigation_policy.rs");
include!("commands/scan_lifecycle.rs");
include!("commands/knowledge_learning.rs");
include!("commands/knowledge_learning_skills.rs");
include!("commands/knowledge_learning_traces.rs");
include!("commands/knowledge_learning_candidates.rs");
include!("commands/knowledge_learning_quality.rs");
include!("commands/rule_packs.rs");
include!("commands/strix_compatibility.rs");
include!("commands/runtime_config.rs");
include!("commands/frontend_recon.rs");
include!("commands/native_frontend_recon.rs");
include!("commands/native_helpers.rs");
include!("commands/native_recon_contract.rs");
include!("commands/native_src_transport.rs");
#[path = "commands/native_sensitive.rs"]
mod native_sensitive;
include!("commands/runtime_environment.rs");
include!("commands/agent_contract.rs");
include!("commands/agent_runtime.rs");
include!("commands/agent_model_client.rs");
include!("commands/agent_tools.rs");
include!("commands/agent_tools_specs.rs");
include!("commands/agent_tools_schema.rs");
include!("commands/agent_tools_scope.rs");
include!("commands/agent_tools_http.rs");
include!("commands/agent_tools_replay.rs");
include!("commands/agent_tools_identity_diff.rs");
include!("commands/agent_tools_discovery.rs");
include!("commands/agent_tools_browser.rs");
include!("commands/agent_tools_verdict.rs");
include!("commands/agent_tools_dispatch.rs");
include!("commands/agent_native.rs");
include!("commands/agent_backend.rs");
include!("commands/scan_execution.rs");
include!("commands/scan_execution_frontend.rs");
include!("commands/scan_execution_runtime.rs");
include!("commands/scan_execution_metrics.rs");
include!("commands/scan_execution_routing.rs");
include!("commands/code_analysis.rs");
include!("commands/scan_control.rs");
include!("commands/appsec_validation.rs");
include!("commands/investigation.rs");
include!("commands/result_ingestion.rs");
include!("commands/result_ingestion_sync.rs");
include!("commands/result_ingestion_artifacts.rs");
include!("commands/result_ingestion_recon.rs");
include!("commands/result_ingestion_runs.rs");
include!("commands/result_ingestion_engines.rs");
include!("commands/asset_export.rs");
include!("commands/tests.rs");
include!("commands/agent_tests.rs");
include!("commands/agent_acceptance.rs");
include!("commands/agent_entries.rs");
