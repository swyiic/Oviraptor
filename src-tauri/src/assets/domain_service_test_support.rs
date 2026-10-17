use crate::db;
use rusqlite::params;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) exports: PathBuf,
    pub(crate) instance: String,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-domain-adapter-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let exports = root.join("exports");
        fs::create_dir(&exports).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&root, &exports] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let path = db::initialize(&root).unwrap();
        let instance = uuid::Uuid::new_v4().to_string();
        let conn = db::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE asset_service_store(singleton INTEGER PRIMARY KEY CHECK(singleton=1),instance_id TEXT NOT NULL UNIQUE,schema_version INTEGER NOT NULL CHECK(schema_version=1));").unwrap();
        conn.execute(
            "INSERT INTO asset_service_store VALUES(1,?1,1)",
            [&instance],
        )
        .unwrap();
        conn.execute("INSERT INTO projects(id,name) VALUES(9001,'Synthetic domain project'),(9002,'Other private project')", []).unwrap();
        for (id, project) in [(1, 9001), (2, 9001), (3, 9002)] {
            conn.execute("INSERT INTO assets(id,asset_key,host,domain,link,title,probe_outcome) VALUES(?1,?2,?3,?3,?4,'Synthetic title','web_alive')",params![id,format!("domain-fixture-{id}"),format!("host-{id}.example.test"),format!("https://host-{id}.example.test")]).unwrap();
            conn.execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)",
                [project, id],
            )
            .unwrap();
        }
        Self {
            root,
            path,
            exports,
            instance,
        }
    }
    pub(crate) fn exposure_run(&self, project: i64, status: &str) -> i64 {
        let conn = db::open(&self.path).unwrap();
        conn.execute(
            "INSERT INTO exposure_runs(project_id,status) VALUES(?1,?2)",
            params![project, status],
        )
        .unwrap();
        conn.last_insert_rowid()
    }
    pub(crate) fn ownership_run(&self, project: i64, status: &str) -> i64 {
        let conn = db::open(&self.path).unwrap();
        conn.execute("INSERT INTO ownership_analysis_runs(project_id,status,scope_version,data_revision,policy_version,directory_version,binding_version,algorithm_version) VALUES(?1,?2,'scope','revision',1,0,0,'synthetic')",params![project,status]).unwrap();
        conn.last_insert_rowid()
    }
    pub(crate) fn finding(&self, project: i64, title: &str) -> i64 {
        let conn = db::open(&self.path).unwrap();
        conn.execute("INSERT INTO exposure_findings(project_id,category,title,source_type,source_url,severity,confidence,status,fingerprint) VALUES(?1,'source_code',?2,'passive_web','https://example.test/public','medium',80,'new',?3)",params![project,title,uuid::Uuid::new_v4().to_string()]).unwrap();
        conn.last_insert_rowid()
    }
    pub(crate) fn expectation(asset: i64) -> Value {
        json!({"assetId":asset,"status":"unreviewed","source":"","confidence":0,"authorizationStatus":"unknown","exposureEligible":false,"reason":""})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
