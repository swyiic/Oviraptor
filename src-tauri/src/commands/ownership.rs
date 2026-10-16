//! Asset ownership: profiles, organization directory, review groups, durable decisions,
//! authorization and read-only unit projection. Exposure consumes the explicit read API;
//! scan/model execution remains outside this domain.
use crate::{
    db,
    asset_models::{
        AssetOwnershipAssessmentResult, AssetOwnershipDecisionInput, AssetOwnershipProfile,
        AssetOwnershipSummary, AssetSelection,
    },
    AppState,
};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, OptionalExtension, Row};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};
use tauri::State;
use uuid::Uuid;

#[path = "../assets/owner_activity.rs"]
mod background_activity;
use background_activity::thread;
pub(crate) fn with_background_scope<F: FnOnce() -> T, T>(db_path: &Path, f: F) -> T {
    background_activity::with_scope(db_path, f)
}
pub(crate) fn active_background_workers(db_path: &Path) -> Result<usize, String> {
    background_activity::active_for(db_path)
}
#[cfg(test)]
pub(crate) fn spawn_background_for_test<F, T>(f: F) -> std::thread::JoinHandle<T>
where F: FnOnce() -> T + Send + 'static, T: Send + 'static {
    thread::spawn(f)
}

#[cfg(test)]
use crate::asset_models::AssetQuery;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use crate::commands::assets::asset_filter;
#[cfg(test)]
use crate::scanning::ownership_test_support::authorization_control_setup_in;

// These source units share only the cohesive ownership domain's explicit scope.
include!("asset_ownership.rs");
include!("ownership_directory.rs");
include!("ownership_public_suffix.rs");
include!("ownership_review.rs");
include!("ownership_operations.rs");
include!("ownership_authorization.rs");
include!("ownership_unit_projection.rs");
include!("ownership_o3_tests.rs");
