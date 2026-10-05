//! Original parent service exclusion, held before any Coordinator mutation.
use super::lease::CoordinatorLease;
use crate::agent_runtime::execution_owner::{claim_native_invocation, NativeInvocationOwner};
use std::path::{Path, PathBuf};

pub(crate) struct ParentInvocationOwner {
    path: PathBuf,
    scan: String,
    attempt: i64,
    root: String,
    _lock: NativeInvocationOwner,
}
impl ParentInvocationOwner {
    pub(crate) fn claim(path: &Path, scan: &str, attempt: i64, root: &str) -> Result<Self, String> {
        if scan.is_empty() || attempt <= 0 || root.is_empty() {
            return Err("parent_owner_scope_invalid".into());
        }
        let original =
            std::fs::canonicalize(path).map_err(|e| format!("parent_owner_database:{e}"))?;
        let lock = claim_native_invocation(&original, scan, attempt, "parent-supervisor", root)?;
        Ok(Self {
            path: original,
            scan: scan.into(),
            attempt,
            root: root.into(),
            _lock: lock,
        })
    }
    pub(crate) fn validate_scope(
        &self,
        path: &Path,
        scan: &str,
        attempt: i64,
        root: &str,
    ) -> Result<(), String> {
        let original =
            std::fs::canonicalize(path).map_err(|e| format!("parent_owner_database:{e}"))?;
        if original != self.path
            || scan != self.scan
            || attempt != self.attempt
            || root != self.root
        {
            return Err("parent_owner_scope_changed".into());
        }
        Ok(())
    }
    pub(crate) fn validate(&self, path: &Path, actor: &CoordinatorLease) -> Result<(), String> {
        self.validate_scope(
            path,
            &actor.scan_id,
            actor.attempt_number,
            &actor.root_run_id,
        )
    }
}
