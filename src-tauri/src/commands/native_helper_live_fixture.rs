use serde_json::json;
struct HelperFixture {
    root: PathBuf,
    db: PathBuf,
    claim: String,
    branch: Option<NativeBranchGuard>,
}
impl HelperFixture {
    fn new() -> Self {
        Self::new_for("web")
    }
    fn new_for(branch: &'static str) -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-live-helper-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let db = db::initialize(&root.join("app")).unwrap();
        let c = db::open(&db).unwrap();
        c.execute(
            "INSERT INTO projects(name,status) VALUES('helper-fixture','active')",
            [],
        )
        .unwrap();
        let project = c.last_insert_rowid();
        c.execute("INSERT INTO sentinel_scans(id,project_id,status,scan_type,attempt_count) VALUES('web-a',?1,'scanning','web',1)",[project]).unwrap();
        // Entire fixture Native JSON bytes are preserved; no Root or SDK permission is invented.
        let native = "{\"bytes\":\"original-full-source-contract\"}";
        c.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,backend_plan_json) VALUES('web-a',1,?1)",[native]).unwrap();
        register_native_branches(&c, "web-a", 1, &[branch]).unwrap();
        let guard = NativeBranchGuard::claim(&db, "web-a", 1, branch).unwrap();
        let claim=c.query_row("SELECT claim_id FROM native_branch_dispatches WHERE scan_id='web-a' AND attempt_number=1 AND branch=?1",[branch],|r|r.get::<_,String>(0)).unwrap();
        assert!(Uuid::parse_str(&claim).is_ok());
        assert_ne!(claim, "11111111-1111-4111-8111-111111111111");
        Self {
            root,
            db,
            claim,
            branch: Some(guard),
        }
    }
    fn control(&self) -> NativeReconControl {
        NativeReconControl::new_logged(Duration::from_secs(12), &self.db, "web-a", 1, "about:blank")
            .unwrap()
    }
    fn worker(&self, name: &str, source: &str) -> PathBuf {
        let p = self.root.join(name);
        fs::write(&p, source).unwrap();
        p
    }
    fn rows(&self) -> Vec<crate::native_pipeline::process::log::Row> {
        let snapshot =
            read_native_process_log_snapshot(&self.db, "web-a", 1, None, None, 0, 300).unwrap();
        let raw = serde_json::to_value(snapshot).unwrap();
        let reader = rusqlite::Connection::open_with_flags(
            &self.db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let page =
            crate::native_pipeline::process::log::page(&reader, "web-a", 1, None, 0, 300).unwrap();
        assert_eq!(raw["rows"].as_array().unwrap().len(), page.rows.len());
        page.rows
    }
    fn native(&self) -> String {
        db::open(&self.db).unwrap().query_row("SELECT backend_plan_json FROM sentinel_scan_attempts WHERE scan_id='web-a' AND attempt_number=1",[],|r|r.get(0)).unwrap()
    }
}
impl Drop for HelperFixture {
    fn drop(&mut self) {
        if let Some(mut branch) = self.branch.take() {
            branch.disarm();
            drop(branch);
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn runtime() -> OsString {
    std::env::var_os("PATH").unwrap_or_default()
}
