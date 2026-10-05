// Actual existing Native Web invocation; diagnostics do not claim branch/Root.
#[derive(Clone)]
struct NativeHelperLogBinding {
    path: PathBuf,
    scan_id: String,
    attempt: i64,
    claim: String,
    target_digest: String,
}
impl NativeHelperLogBinding {
    fn load(path: &Path, scan_id: &str, attempt: i64, target: &str) -> Result<Self, String> {
        let c =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| "native_helper_log_database_unavailable")?;
        let claim:String=c.query_row("SELECT d.claim_id FROM native_branch_dispatches d
            JOIN native_scan_branches b ON b.scan_id=d.scan_id AND b.attempt_number=d.attempt_number AND b.branch=d.branch
            JOIN sentinel_scans s ON s.id=d.scan_id AND s.attempt_count=d.attempt_number
            JOIN sentinel_scan_attempts a ON a.scan_id=d.scan_id AND a.attempt_number=d.attempt_number
            WHERE d.scan_id=?1 AND d.attempt_number=?2 AND d.branch='web' AND d.claim_id<>'' AND d.claimed_at<>''
            AND b.status='pending' AND s.status='scanning'
            AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            params![scan_id,attempt],|r|r.get(0)).map_err(|_|"native_helper_log_web_branch_not_claimed")?;
        if Uuid::parse_str(&claim).is_err() || attempt <= 0 || target.is_empty() {
            return Err("native_helper_log_scope_invalid".into());
        }
        Ok(Self {
            path: path.into(),
            scan_id: scan_id.into(),
            attempt,
            claim,
            target_digest: format!("{:x}", Sha256::digest(target.as_bytes())),
        })
    }
    fn scope(
        &self,
        helper: &Path,
        input: &[u8],
        environment: &JsonValue,
    ) -> Result<crate::native_pipeline::process::log::Scope, String> {
        let name = helper
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("native_helper_log_helper_invalid")?;
        let stage = match name {
            "8_js_ast_analyzer.cjs" => "node_ast:parse",
            "9_frontend_runtime_probe.cjs" => "node_browser:probe",
            #[cfg(test)]
            "fixture.cjs" | "fake_runtime_probe.cjs" => "node_fixture:helper",
            _ => return Err("native_helper_log_helper_invalid".into()),
        };
        let bytes = fs::read(helper).map_err(|_| "native_helper_log_helper_unavailable")?;
        let dependency = match name {
            "8_js_ast_analyzer.cjs" => Some(helper.with_file_name("babel-parser.cjs")),
            "9_frontend_runtime_probe.cjs" => helper
                .parent()
                .and_then(Path::parent)
                .map(|p| p.join("config/browser-locations.json")),
            _ => None,
        };
        let dependency_digest = dependency
            .map(|p| fs::read(p).map(|b| format!("{:x}", Sha256::digest(b))))
            .transpose()
            .map_err(|_| "native_helper_log_dependency_unavailable")?;
        let material = serde_json::to_vec(&serde_json::json!([
            "native-helper-log-v1",
            self.scan_id,
            self.attempt,
            self.claim,
            self.target_digest,
            name,
            format!("{:x}", Sha256::digest(bytes)),
            format!("{:x}", Sha256::digest(input)),
            dependency_digest,
            environment
        ]))
        .map_err(|_| "native_helper_log_scope_invalid")?;
        Ok(crate::native_pipeline::process::log::Scope {
            scan_id: self.scan_id.clone(),
            attempt: self.attempt,
            branch: "web".into(),
            dispatch_claim_id: self.claim.clone(),
            invocation_key: format!("{:x}", Sha256::digest(material)),
            execution_id: Uuid::new_v4().to_string(),
            stage: stage.into(),
        })
    }
}
