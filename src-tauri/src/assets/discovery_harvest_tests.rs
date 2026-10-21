use super::*;
fn fixture(c: &mut Connection, count: i64) {
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,domain TEXT DEFAULT '',extra_json TEXT);CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,PRIMARY KEY(project_id,asset_id));").unwrap();
    let tx = c.transaction().unwrap();
    {
        let mut a = tx
            .prepare("INSERT INTO assets(id,host,link,extra_json) VALUES(?1,?2,?2,?3)")
            .unwrap();
        let mut pa = tx
            .prepare("INSERT INTO project_assets VALUES(1,?1,0)")
            .unwrap();
        for id in 1..=count {
            a.execute(params![
                id,
                format!("https://h{id}.example.com"),
                if id == count {
                    json!({"cert_sha256":"a".repeat(64),"cert.domain":"tail.example.com"})
                        .to_string()
                } else {
                    "{}".into()
                }
            ])
            .unwrap();
            pa.execute([id]).unwrap();
        }
    }
    tx.commit().unwrap();
}
#[test]
fn automatically_extracts_all_70013_assets_and_the_tail_certificate_without_manual_batches() {
    let mut c = Connection::open_in_memory().unwrap();
    fixture(&mut c, 70_013);
    let (id, start) = prepare(&mut c, 1).unwrap();
    assert!(start);
    let mut batches = 0;
    while batch(&mut c, 1, &id).unwrap() {
        batches += 1;
    }
    assert!(batches > 350);
    let result = status_on(&c, 1).unwrap();
    assert_eq!(result["status"], "completed");
    assert_eq!(result["processed"], 70_013);
    assert_eq!(result["total"], 70_013);
    assert_eq!(result["certificates"], 1);
    assert!(c.query_row("SELECT EXISTS(SELECT 1 FROM asset_discovery_seeds WHERE host='tail.example.com' AND project_id=1)",[],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_discovery_harvest_items",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        70_013
    );
}
#[test]
fn frozen_membership_skips_deleted_associations_and_defers_new_ones() {
    let mut c = Connection::open_in_memory().unwrap();
    fixture(&mut c, 3);
    let (id, _) = prepare(&mut c, 1).unwrap();
    c.execute_batch("UPDATE project_assets SET is_deleted=1 WHERE asset_id=2;INSERT INTO assets(id,host,link,extra_json) VALUES(4,'https://later.example.com','https://later.example.com','{}');INSERT INTO project_assets VALUES(1,4,0);").unwrap();
    while batch(&mut c, 1, &id).unwrap() {}
    let state = status_on(&c, 1).unwrap();
    assert_eq!(state["total"], 3);
    assert_eq!(state["processed"], 3);
    assert_eq!(state["skipped"], 1);
    assert!(!c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM asset_discovery_seeds WHERE host='later.example.com')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    let (next, _) = prepare(&mut c, 1).unwrap();
    assert_ne!(id, next);
    while batch(&mut c, 1, &next).unwrap() {}
    assert_eq!(status_on(&c, 1).unwrap()["total"], 3);
    assert_eq!(status_on(&c, 1).unwrap()["added"], 1);
}
#[test]
fn pause_and_process_restart_resume_the_committed_cursor_and_cannot_cross_projects() {
    let root = std::env::temp_dir().join(format!("ovi-local-harvest-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("test.sqlite");
    let mut c = Connection::open(&path).unwrap();
    fixture(&mut c, 503);
    let (id, _) = prepare(&mut c, 1).unwrap();
    assert!(batch(&mut c, 1, &id).unwrap());
    c.execute(
        "UPDATE asset_discovery_harvests SET status='stopping' WHERE id=?1",
        [&id],
    )
    .unwrap();
    assert!(!batch(&mut c, 1, &id).unwrap());
    assert_eq!(status_on(&c, 1).unwrap()["processed"], 200);
    drop(c);
    let mut c = Connection::open(&path).unwrap();
    let (same, _) = prepare(&mut c, 1).unwrap();
    assert_eq!(same, id);
    assert!(batch(&mut c, 2, &id).is_err());
    c.execute(
        "UPDATE asset_discovery_harvests SET owner_pid=0 WHERE id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(status_on(&c, 1).unwrap()["status"], "interrupted");
    assert_eq!(prepare(&mut c, 1).unwrap().0, id);
    while batch(&mut c, 1, &id).unwrap() {}
    assert_eq!(status_on(&c, 1).unwrap()["processed"], 503);
    drop(c);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn failed_batch_rolls_back_seeds_and_progress_then_can_be_retried() {
    let mut c = Connection::open_in_memory().unwrap();
    fixture(&mut c, 3);
    let (id, _) = prepare(&mut c, 1).unwrap();
    c.execute_batch("CREATE TRIGGER synthetic_abort BEFORE INSERT ON asset_discovery_seeds WHEN NEW.host='h2.example.com' BEGIN SELECT RAISE(ABORT,'synthetic extraction failure'); END;").unwrap();
    assert!(batch(&mut c, 1, &id).is_err());
    assert_eq!(status_on(&c, 1).unwrap()["processed"], 0);
    assert_eq!(
        c.query_row("SELECT count(*) FROM asset_discovery_seeds", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER synthetic_abort;").unwrap();
    while batch(&mut c, 1, &id).unwrap() {}
    assert_eq!(status_on(&c, 1).unwrap()["processed"], 3);
}
#[test]
fn active_job_is_reused_and_empty_project_completes_without_a_fake_percentage() {
    let mut c = Connection::open_in_memory().unwrap();
    fixture(&mut c, 0);
    let (id, _) = prepare(&mut c, 1).unwrap();
    active().lock().unwrap().insert(id.clone());
    assert_eq!(prepare(&mut c, 1).unwrap(), (id.clone(), false));
    active().lock().unwrap().remove(&id);
    assert!(!batch(&mut c, 1, &id).unwrap());
    let s = status_on(&c, 1).unwrap();
    assert_eq!(s["status"], "completed");
    assert_eq!(s["total"], 0);
    assert_eq!(s["processed"], 0);
}
#[test]
fn reads_of_an_existing_project_do_not_initialize_the_harvest_store() {
    let c = Connection::open_in_memory().unwrap();
    assert!(status_on(&c, 1).unwrap().is_null());
    assert_eq!(
        c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn missing_original_store_is_never_recreated_by_readers_or_background_writers() {
    let path = std::env::temp_dir().join(format!(
        "ovi-absent-local-store-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    assert!(open_existing(&path, false).is_err());
    assert!(open_existing(&path, true).is_err());
    assert!(!path.exists());
}
