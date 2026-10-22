//! Atomic birth record for the discovery coordinator's original FOFA job.
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS asset_discovery_collections(
 operation_id TEXT PRIMARY KEY, project_id INTEGER NOT NULL REFERENCES projects(id),
 request_sha256 TEXT NOT NULL, targets_json TEXT NOT NULL,
 run_id INTEGER NOT NULL UNIQUE REFERENCES runs(id),
 created_at TEXT NOT NULL DEFAULT(datetime('now'))
);
CREATE TRIGGER IF NOT EXISTS asset_discovery_collection_immutable
 BEFORE UPDATE ON asset_discovery_collections
 BEGIN SELECT RAISE(ABORT,'discovery collection birth is immutable'); END;
CREATE TRIGGER IF NOT EXISTS asset_discovery_collection_no_delete
 BEFORE DELETE ON asset_discovery_collections
 BEGIN SELECT RAISE(ABORT,'discovery collection birth is immutable'); END;
";

pub(super) struct Intent {
    project_id: i64,
    operation_id: String,
    digest: String,
    pub(super) targets: Vec<(String, String)>,
}
impl Intent {
    pub(super) fn new(project_id: i64, operation_id: &str, values: &[String]) -> Result<Self, String> {
        if project_id <= 0 || operation_id.is_empty() || operation_id.len() > 160
            || !operation_id.chars().all(|c| c.is_ascii_alphanumeric() || "-_:.".contains(c))
            || values.is_empty() || values.len() > 256 {
            return Err("首次采集的持久请求或种子无效".into());
        }
        let targets = crate::commands::assets::normalize_collection_targets(values)?;
        let normalized=targets.iter().map(|(kind,value)|(kind,value.to_lowercase())).collect::<Vec<_>>();
        let canonical = serde_json::to_string(&(project_id, normalized))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            project_id, operation_id: operation_id.into(),
            digest: format!("{:x}", Sha256::digest(canonical.as_bytes())),
            targets,
        })
    }
    pub(super) fn existing_on(&self, c: &Connection) -> Result<Option<i64>, String> {
        c.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        self.lookup_on(c)
    }
    pub(super) fn lookup_on(&self, c: &Connection) -> Result<Option<i64>, String> {
        let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_collections')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !exists { return Ok(None); }
        let row: Option<(i64, String, i64, i64, String)> = c.query_row(
            "SELECT d.project_id,d.request_sha256,d.run_id,r.project_id,r.pipeline
             FROM asset_discovery_collections d JOIN runs r ON r.id=d.run_id WHERE d.operation_id=?1",
            [&self.operation_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).optional().map_err(|e| e.to_string())?;
        match row {
            Some((p, hash, run, rp, pipeline)) if p == self.project_id && rp == p
                && hash == self.digest && pipeline == "collect" => Ok(Some(run)),
            Some(_) => Err("首次采集请求已绑定其他项目或不同种子，未重复发送 FOFA 请求".into()),
            None => {
                let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_collections WHERE operation_id=?1)", [&self.operation_id], |r| r.get(0)).map_err(|e| e.to_string())?;
                if exists { Err("首次采集原任务缺失，保留记录且未重复发送 FOFA 请求".into()) } else { Ok(None) }
            }
        }
    }
    pub(super) fn bind_on(&self, c: &Connection, run_id: i64) -> Result<(), String> {
        // The surrounding start transaction owns both rows, settings snapshot
        // and service owner. A failed binding rolls all of them back.
        c.execute("INSERT INTO asset_discovery_collections(operation_id,project_id,request_sha256,targets_json,run_id) VALUES(?1,?2,?3,?4,?5)",
            params![self.operation_id, self.project_id, self.digest,
                serde_json::to_string(&self.targets).map_err(|e| e.to_string())?, run_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub(super) struct StagedSeeds { path: std::path::PathBuf, pub(super) sha256: String, published: bool }
impl StagedSeeds {
    pub(super) fn prepare(data: &std::path::Path, dir: &std::path::Path, targets: &[(String,String)]) -> Result<Self,String> {
        let root=data.join("jobs");std::fs::create_dir_all(&root).map_err(|e|e.to_string())?;
        let canonical_data=std::fs::canonicalize(data).map_err(|e|e.to_string())?;
        let canonical_root=std::fs::canonicalize(&root).map_err(|e|e.to_string())?;
        if canonical_root.parent()!=Some(canonical_data.as_path()) || std::fs::symlink_metadata(&root).map_err(|e|e.to_string())?.file_type().is_symlink() || dir.parent()!=Some(root.as_path()) {
            return Err("首采种子目录不属于当前应用，未发布任务".into());
        }
        std::fs::create_dir(dir).map_err(|e|e.to_string())?;
        let mut staged=Self{path:dir.into(),sha256:String::new(),published:false};
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(dir,std::fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?; }
        let path=dir.join("seeds.csv");super::write_seeds(&path,targets)?;
        let bytes=std::fs::read(&path).map_err(|e|e.to_string())?;
        staged.sha256=format!("{:x}",Sha256::digest(&bytes));
        #[cfg(unix)] std::fs::File::open(&root).and_then(|f|f.sync_all()).map_err(|e|e.to_string())?;
        Ok(staged)
    }
    pub(super) fn publish(&mut self) { self.published=true; }
}
impl Drop for StagedSeeds { fn drop(&mut self) { if !self.published { let _=std::fs::remove_dir_all(&self.path); } } }

pub(super) fn validate_snapshot_seeds(settings:&serde_json::Value,path:&std::path::Path)->Result<(),String>{
    let Some(expected)=settings.get("_discoverySeedsSha256") else{return Ok(())};
    let expected=expected.as_str().filter(|h|h.len()==64&&h.bytes().all(|b|b.is_ascii_hexdigit())).ok_or("原首采种子摘要损坏，未重发请求")?;
    let meta=std::fs::symlink_metadata(path).map_err(|_|"原首采种子文件缺失，未重发请求")?;
    if !meta.is_file()||meta.file_type().is_symlink()||meta.len()>1024*1024 {return Err("原首采种子文件无效，未重发请求".into())}
    let bytes=std::fs::read(path).map_err(|e|e.to_string())?;
    if format!("{:x}",Sha256::digest(bytes))!=expected{return Err("原首采种子内容已变化，未采用当前项目种子或重发请求".into())}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn database() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE projects(id INTEGER PRIMARY KEY); INSERT INTO projects VALUES(1),(2); CREATE TABLE runs(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL REFERENCES projects(id),pipeline TEXT NOT NULL); INSERT INTO runs VALUES(11,1,'collect'),(22,2,'collect'),(33,1,'full');").unwrap();
        c
    }
    #[test]
    fn same_intent_returns_original_terminal_or_active_run_without_launch() {
        let c = database();
        let intent = Intent::new(1,"assist:stable", &["EXAMPLE.com".into(), "example.com".into()]).unwrap();
        assert_eq!(intent.existing_on(&c).unwrap(), None);
        assert_eq!(intent.targets.len(), 1);
        intent.bind_on(&c, 11).unwrap();
        let repeated = Intent::new(1,"assist:stable", &["example.com".into()]).unwrap();
        assert_eq!(repeated.existing_on(&c).unwrap(), Some(11));
        assert_eq!(c.query_row("SELECT count(*) FROM runs", [], |r|r.get::<_,i64>(0)).unwrap(),3);
        assert!(Intent::new(2,"assist:stable", &["example.com".into()]).unwrap().existing_on(&c).is_err());
        assert!(Intent::new(1,"assist:stable", &["other.com".into()]).unwrap().existing_on(&c).is_err());
    }
    #[test]
    fn failed_start_rolls_back_birth_and_run_together() {
        let mut c = database();
        let intent = Intent::new(1,"crash", &["单位名称".into()]).unwrap();
        { let tx = c.transaction().unwrap();
          assert_eq!(intent.existing_on(&tx).unwrap(),None);
          tx.execute("INSERT INTO runs VALUES(44,1,'collect')",[]).unwrap();
          intent.bind_on(&tx,44).unwrap();
        }
        assert_eq!(intent.existing_on(&c).unwrap(),None);
        assert_eq!(c.query_row("SELECT count(*) FROM runs WHERE id=44", [], |r|r.get::<_,i64>(0)).unwrap(),0);
    }
    #[test]
    fn committed_birth_cannot_be_rebound_or_deleted() {
        let c=database(); let i=Intent::new(1,"original", &["10086.cn".into()]).unwrap();
        i.existing_on(&c).unwrap(); i.bind_on(&c,11).unwrap();
        assert!(c.execute("UPDATE asset_discovery_collections SET run_id=22",[]).is_err());
        assert!(c.execute("DELETE FROM asset_discovery_collections",[]).is_err());
        assert_eq!(i.existing_on(&c).unwrap(),Some(11));
    }
    #[test]
    fn malformed_inputs_never_create_a_birth() {
        for (op,targets) in [("",vec!["example.com".into()]),("op",vec![]),("op",vec!["line\nbreak".into()]),("x/y",vec!["example.com".into()])] {
            assert!(Intent::new(1,op,&targets).is_err());
        }
    }
    #[test]
    fn committed_seed_file_survives_prelaunch_crash_and_rejects_tampering(){
        let data=std::env::temp_dir().join(format!("ovi-seed-birth-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&data).unwrap();
        let dir=data.join("jobs").join(uuid::Uuid::new_v4().to_string());
        let digest;
        { let mut staged=StagedSeeds::prepare(&data,&dir,&[("domain".into(),"original.example".into())]).unwrap();digest=staged.sha256.clone();staged.publish(); }
        let settings=serde_json::json!({"_discoverySeedsSha256":digest});
        super::super::job_resume::validate_seeds(&dir.join("seeds.csv")).unwrap();validate_snapshot_seeds(&settings,&dir.join("seeds.csv")).unwrap();
        super::super::write_seeds(&dir.join("seeds.csv"),&[("domain".into(),"changed.example".into())]).unwrap();assert!(validate_snapshot_seeds(&settings,&dir.join("seeds.csv")).is_err());
        let orphan=data.join("jobs").join(uuid::Uuid::new_v4().to_string());{let _guard=StagedSeeds::prepare(&data,&orphan,&[("company".into(),"Original Company".into())]).unwrap();}assert!(!orphan.exists());
        std::fs::remove_dir_all(data).unwrap();
    }
}
