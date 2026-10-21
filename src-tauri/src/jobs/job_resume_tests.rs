use super::*;

struct Fixture {
    root: PathBuf,
    db_path: PathBuf,
    conn: rusqlite::Connection,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-job-resume-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        let conn = db::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO projects(id,name) VALUES(9001,'合成恢复项目')",
            [],
        )
        .unwrap();
        conn.execute("UPDATE config_profiles SET settings_json=?1 WHERE id=1",[r#"{"fofaKey":"fixture-secret","pageSize":900,"workers":128,"probeTimeout":99,"proxyUrl":"http://127.0.0.1:1234","collectionTimeout":20}"#]).unwrap();
        let run_dir = root.join("jobs/original");
        fs::create_dir_all(&run_dir).unwrap();
        write_seeds(
            &run_dir.join("seeds.csv"),
            &[("domain".into(), "original.example".into())],
        )
        .unwrap();
        conn.execute("INSERT INTO runs(id,project_id,profile_id,name,pipeline,status,error,config_snapshot,output_dir) VALUES(9001,9001,1,'合成失败','full','failed','original-error',?1,?2)",
            params![r#"{"fofaKey":"__CONFIGURED__","pageSize":500,"workers":64,"probeTimeout":6,"collectionCache":true}"#, run_dir.to_string_lossy()]).unwrap();
        Self {
            root,
            db_path,
            conn,
        }
    }
    fn state(&self) -> (String, String) {
        self.conn
            .query_row("SELECT status,error FROM runs WHERE id=9001", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
    }
    fn rejected(&mut self) -> String {
        let message = prepare_resume(&mut self.conn, 9001, &self.root)
            .err()
            .unwrap();
        assert_eq!(self.state(), ("failed".into(), "original-error".into()));
        message
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn full_resume_freezes_seeds_and_policy_but_reloads_credentials_and_transport() {
    let mut f = Fixture::new();
    let before = fs::read(f.root.join("jobs/original/seeds.csv")).unwrap();
    f.conn.execute("INSERT INTO targets(project_id,target_type,value,normalized_value) VALUES(9001,'domain','changed.example','changed.example')",[]).unwrap();
    let ready = prepare_resume(&mut f.conn, 9001, &f.root).unwrap();
    assert_eq!(ready.pipeline, "full");
    assert_eq!(ready.project_id, 9001);
    assert_eq!(ready.settings["fofaKey"], "fixture-secret");
    assert_eq!(ready.settings["pageSize"], 500);
    assert_eq!(ready.settings["workers"], 64);
    assert_eq!(ready.settings["probeTimeout"], 6);
    assert_eq!(ready.settings["collectionTimeout"], 20);
    assert_eq!(ready.settings["proxyUrl"], "http://127.0.0.1:1234");
    assert_eq!(fs::read(ready.run_dir.join("seeds.csv")).unwrap(), before);
    assert_eq!(f.state(), ("queued".into(), "".into()));
    assert!(prepare_resume(&mut f.conn, 9001, &f.root).is_err());
    let snapshot: String = f
        .conn
        .query_row("SELECT config_snapshot FROM runs WHERE id=9001", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!snapshot.contains("fixture-secret"));
}

#[test]
fn resume_uses_current_network_owner_and_persists_only_its_safe_reference() {
    let mut f = Fixture::new();
    let current = crate::configuration_policies::read_profile(&f.conn, 1).unwrap();
    let network = serde_json::json!({"proxyUrl":"http://resume-user:resume-password@127.0.0.1:1234","noProxy":"localhost,.synthetic.invalid","agentProxyEnabled":true,"authorizedProxyPool":["ALL|socks5h://pool-user:pool-password@[::1]:1080"]});
    let tx = f
        .conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let reference = crate::configuration_runtime_owners::save(
        &tx,
        1,
        "network",
        &network,
        &crate::configuration_runtime_owners::project(&current, "network").unwrap(),
        None,
        true,
    )
    .unwrap();
    tx.commit().unwrap();
    let ready = prepare_resume(&mut f.conn, 9001, &f.root).unwrap();
    assert_eq!(ready.settings["proxyUrl"], network["proxyUrl"]);
    assert_eq!(ready.settings["noProxy"], network["noProxy"]);
    assert_eq!(
        ready.settings["authorizedProxyPool"],
        network["authorizedProxyPool"]
    );
    assert_eq!(ready.settings["workers"], 64);
    assert_eq!(ready.settings["pageSize"], 500);
    let text: String = f
        .conn
        .query_row("SELECT config_snapshot FROM runs WHERE id=9001", [], |r| {
            r.get(0)
        })
        .unwrap();
    for secret in [
        "resume-user",
        "resume-password",
        "pool-user",
        "pool-password",
        "fixture-secret",
    ] {
        assert!(!text.contains(secret));
    }
    let snapshot: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        crate::configuration_policies::reference(&snapshot, "network").unwrap(),
        Some(reference)
    );
}

#[test]
fn resume_archived_busy_and_corrupt_inputs_leave_failed_task_unchanged() {
    let mut f = Fixture::new();
    f.conn
        .execute("UPDATE projects SET status='archived' WHERE id=9001", [])
        .unwrap();
    assert!(f.rejected().contains("归档"));
    f.conn
        .execute("UPDATE projects SET status='active' WHERE id=9001", [])
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO runs(id,project_id,name,status) VALUES(9002,9001,'正在运行','running')",
            [],
        )
        .unwrap();
    assert!(f.rejected().contains("运行中"));
    f.conn
        .execute("UPDATE runs SET status='completed' WHERE id=9002", [])
        .unwrap();
    fs::write(f.root.join("jobs/original/seeds.csv"), "broken\nrow\n").unwrap();
    assert!(f.rejected().contains("种子"));
    write_seeds(
        &f.root.join("jobs/original/seeds.csv"),
        &[("domain".into(), "original.example".into())],
    )
    .unwrap();
    f.conn
        .execute("UPDATE runs SET config_snapshot='[]' WHERE id=9001", [])
        .unwrap();
    assert!(f.rejected().contains("快照"));
}

#[test]
fn resume_rejects_external_directory_and_missing_original_seeds() {
    let mut f = Fixture::new();
    fs::remove_file(f.root.join("jobs/original/seeds.csv")).unwrap();
    assert!(f.rejected().contains("种子"));
    f.conn
        .execute(
            "UPDATE runs SET output_dir=?1 WHERE id=9001",
            [f.root.to_string_lossy()],
        )
        .unwrap();
    assert!(f.rejected().contains("不属于"));
}

#[test]
fn concurrent_database_resume_claims_only_one_worker() {
    let f = Fixture::new();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let root = f.root.clone();
            let db_path = f.db_path.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let mut c = db::open(&db_path).unwrap();
                barrier.wait();
                prepare_resume(&mut c, 9001, &root).is_ok()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|won| *won)
            .count(),
        1
    );
    assert_eq!(f.state().0, "queued");
}

#[test]
fn reprobe_resume_does_not_need_fofa_credentials_or_collection_seeds() {
    let mut f = Fixture::new();
    fs::remove_file(f.root.join("jobs/original/seeds.csv")).unwrap();
    f.conn
        .execute("UPDATE runs SET pipeline='reprobe' WHERE id=9001", [])
        .unwrap();
    f.conn
        .execute(
            "UPDATE config_profiles SET settings_json='{}' WHERE id=1",
            [],
        )
        .unwrap();
    let ready = prepare_resume(&mut f.conn, 9001, &f.root).unwrap();
    assert_eq!(ready.pipeline, "reprobe");
    assert_eq!(ready.settings["fofaKey"], "");
}

#[test]
fn original_ip_cidr_icp_and_asn_seeds_remain_valid_when_resuming() {
    for (kind, value) in [
        ("ip", "192.0.2.1"),
        ("cidr", "192.0.2.0/24"),
        ("icp", "京ICP备合成号"),
        ("asn", "Synthetic Organization"),
        ("keyword", "Synthetic Word"),
    ] {
        let mut f = Fixture::new();
        let seeds = f.root.join("jobs/original/seeds.csv");
        write_seeds(&seeds, &[(kind.into(), value.into())]).unwrap();
        validate_seeds(&seeds).unwrap();
        let before = fs::read(&seeds).unwrap();
        prepare_resume(&mut f.conn, 9001, &f.root).unwrap();
        assert_eq!(fs::read(&seeds).unwrap(), before);
    }
}

#[test]
fn content_rules_remain_frozen_for_probe_and_import_across_resume() {
    let mut f = Fixture::new();
    f.conn
        .execute(
            "INSERT INTO content_rules(keyword,normalized_keyword) VALUES('Welcome','welcome')",
            [],
        )
        .unwrap();
    let first = prepare_resume(&mut f.conn, 9001, &f.root).unwrap();
    assert_eq!(
        first.settings["_assetContentKeywords"],
        serde_json::json!(["Welcome"])
    );
    assert_eq!(first.settings["_assetLegacyRulesAdopted"], true);
    f.conn
        .execute("UPDATE runs SET status='failed' WHERE id=9001", [])
        .unwrap();
    f.conn
        .execute("UPDATE content_rules SET enabled=0", [])
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO content_rules(keyword,normalized_keyword) VALUES('OneNet','onenet')",
            [],
        )
        .unwrap();
    let again = prepare_resume(&mut f.conn, 9001, &f.root).unwrap();
    assert_eq!(
        again.settings["_assetContentKeywords"],
        serde_json::json!(["Welcome"])
    );
    assert_eq!(import_title_rules(&f.conn, 9001).unwrap(), vec!["welcome"]);
    let mut fresh = serde_json::json!({});
    freeze_content_rules(&f.conn, &mut fresh, true).unwrap();
    assert_eq!(
        fresh["_assetContentKeywords"],
        serde_json::json!(["OneNet"])
    );
}

#[test]
fn half_present_or_corrupt_frozen_rules_are_rejected_without_adopting_current_policy() {
    let mut f = Fixture::new();
    for snapshot in [
        r#"{"_assetContentKeywords":["Original"]}"#,
        r#"{"_assetTitleRules":["original"]}"#,
        r#"{"_assetContentKeywords":false,"_assetTitleRules":[]}"#,
    ] {
        f.conn
            .execute(
                "UPDATE runs SET config_snapshot=?1 WHERE id=9001",
                [snapshot],
            )
            .unwrap();
        assert!(f.rejected().contains("规则快照"));
        let stored: String = f
            .conn
            .query_row("SELECT config_snapshot FROM runs WHERE id=9001", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored, snapshot);
    }
}

#[test]
fn independent_default_resume_repairs_account_and_network_only_and_keeps_frozen_policies() {
    use crate::configuration_catalog as catalog;
    let mut f = Fixture::new();
    f.conn.execute("UPDATE runs SET profile_id=NULL,config_snapshot=json_set(config_snapshot,'$._configurationSource','current-defaults','$.fofaEmail','old-account@example.invalid','$.collectionTimeout',9,'$.interval',0.5) WHERE id=9001", []).unwrap();
    let mut references = std::collections::HashMap::new();
    for (section, updates) in [
        (
            "assetApis",
            serde_json::json!({"fofaKey":"current-category-key","fofaEmail":"current-account@example.invalid","configPath":""}),
        ),
        (
            "network",
            serde_json::json!({"proxyUrl":"http://current-user:current-password@127.0.0.1:1234","noProxy":"synthetic.invalid","agentProxyEnabled":false,"authorizedProxyPool":[]}),
        ),
        ("probe", serde_json::json!({"workers":3})),
        (
            "collection",
            serde_json::json!({"pageSize":23,"collectionTimeout":99,"interval":8}),
        ),
    ] {
        let view = catalog::read_category(&f.conn, section, None).unwrap();
        let mut settings = view.settings.clone();
        settings
            .as_object_mut()
            .unwrap()
            .extend(updates.as_object().unwrap().clone());
        let result = catalog::save(
            &mut f.conn,
            catalog::SaveCategoryInput {
                request_id: Uuid::new_v4().to_string(),
                section: section.into(),
                mode: catalog::SaveMode::Create,
                name: section.into(),
                settings,
                expected_settings: view.settings,
                expected_reference: None,
                expected_default: view.default_selection,
                make_default: true,
            },
        )
        .unwrap();
        references.insert(section, result.receipt.reference.unwrap());
    }
    let ready = prepare_resume(&mut f.conn, 9001, &f.root).unwrap();
    assert_eq!(ready.settings["fofaKey"], "current-category-key");
    assert_eq!(
        ready.settings["fofaEmail"],
        "current-account@example.invalid"
    );
    assert_eq!(
        ready.settings["proxyUrl"],
        "http://current-user:current-password@127.0.0.1:1234"
    );
    for (key, expected) in [
        ("workers", 64),
        ("pageSize", 500),
        ("probeTimeout", 6),
        ("collectionTimeout", 9),
    ] {
        assert_eq!(ready.settings[key], expected);
    }
    assert_eq!(ready.settings["interval"], 0.5);
    let text: String = f
        .conn
        .query_row("SELECT config_snapshot FROM runs WHERE id=9001", [], |r| {
            r.get(0)
        })
        .unwrap();
    for secret in ["current-category-key", "current-user", "current-password"] {
        assert!(!text.contains(secret));
    }
    let snapshot: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(snapshot["_configurationSource"], "current-defaults");
    assert_eq!(snapshot["fofaEmail"], "current-account@example.invalid");
    for section in ["assetApis", "network"] {
        assert_eq!(
            crate::configuration_policies::reference(&snapshot, section)
                .unwrap()
                .as_ref(),
            references.get(section)
        );
        assert_eq!(
            snapshot["_configurationCategorySelections"][section]["reference"],
            serde_json::to_value(&references[section]).unwrap()
        );
    }
    assert!(crate::configuration_policies::reference(&snapshot, "probe")
        .unwrap()
        .is_none());
    assert!(
        crate::configuration_policies::reference(&snapshot, "collection")
            .unwrap()
            .is_none()
    );
    assert_eq!(snapshot["workers"], 64);
    assert_eq!(snapshot["pageSize"], 500);
}

#[test]
fn old_null_profile_and_mismatched_new_source_are_never_reinterpreted_as_current_defaults() {
    let mut f = Fixture::new();
    f.conn
        .execute("UPDATE runs SET profile_id=NULL WHERE id=9001", [])
        .unwrap();
    assert!(f.rejected().contains("配置来源"));
    f.conn.execute("UPDATE runs SET profile_id=1,config_snapshot=json_set(config_snapshot,'$._configurationSource','current-defaults') WHERE id=9001", []).unwrap();
    assert!(f.rejected().contains("配置来源"));
}
