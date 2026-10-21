use super::*;
fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,extra_json TEXT);CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0);CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);").unwrap();
    ensure(&c).unwrap();
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.com\"]')",
        [],
    )
    .unwrap();
    asset(&c, 1, 1, "a.bad.example.net", 'a', "a.bad.example.net");
    asset(&c, 1, 2, "b.bad.example.net", 'b', "app.example.com");
    asset(&c, 1, 3, "app.example.com", 'b', "app.example.com");
    asset(
        &c,
        1,
        4,
        "bad.example.net.evil.com",
        'c',
        "bad.example.net.evil.com",
    );
    asset(&c, 2, 5, "a.bad.example.net", 'd', "a.bad.example.net");
    insert_seed(
        &c,
        1,
        "page.example.com",
        "page_or_inline_script",
        "asset:1",
        false,
    )
    .unwrap();
    c
}
fn asset(c: &Connection, p: i64, id: i64, h: &str, ch: char, name: &str) {
    let extra = json!({"cert_sha256":ch.to_string().repeat(64),"cert.domain":name});
    let link = format!("https://{h}");
    c.execute(
        "INSERT INTO assets VALUES(?1,?2,?3,?4)",
        params![id, h, link, extra.to_string()],
    )
    .unwrap();
    c.execute("INSERT INTO project_assets VALUES(?1,?2,0)", params![p, id])
        .unwrap();
    ingest_observations(c, p, &[(id, h.into(), link, extra.to_string())]).unwrap();
}
fn change(c: &mut Connection, domain: &str, action: &str) -> Value {
    let preview = manage(
        c,
        ExclusionInput {
            project_id: 1,
            domain: domain.into(),
            action: "preview".into(),
            expected_digest: None,
        },
    )
    .unwrap();
    manage(
        c,
        ExclusionInput {
            project_id: 1,
            domain: domain.into(),
            action: action.into(),
            expected_digest: preview["digest"].as_str().map(str::to_owned),
        },
    )
    .unwrap()
}
fn visible(c: &Connection, source: &str) -> Vec<Value> {
    super::super::view_filtered(c, 1, "", source, 1, 200).unwrap()["seeds"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn domain_exclusion_preview_is_read_only_and_preserves_independent_fingerprints() {
    let c = fixture();
    let before = c.total_changes();
    let result = plan(&c, 1, "bad.example.net").unwrap().receipt;
    assert_eq!(c.total_changes(), before);
    assert!(!exists(&c).unwrap());
    assert_eq!(result["relatedFingerprints"], 1);
    assert_eq!(result["protectedSharedFingerprints"], 1);
    assert!(result["domainLeads"].as_u64().unwrap() >= 3);
    assert_eq!(result["assetsDeleted"], 0);
}
#[test]
fn exclude_and_restore_hide_only_project_leads_keep_assets_and_stop_paid_admission() {
    let mut c = fixture();
    let original = visible(&c, "all");
    let other = super::super::view_filtered(&c, 2, "", "all", 1, 200).unwrap();
    change(&mut c, "bad.example.net", "apply");
    let active = visible(&c, "all");
    let excluded = visible(&c, "excluded");
    assert!(active
        .iter()
        .any(|s| s["host"] == "bad.example.net.evil.com"));
    assert!(active.iter().any(|s| s["host"] == "b".repeat(64)));
    assert!(!active
        .iter()
        .any(|s| s["host"] == "a".repeat(64) || s["host"] == "page.example.com"));
    assert!(excluded
        .iter()
        .all(|s| s["excluded"] == true && s["eligible"] == false && s["queryEligible"] == false));
    assert!(hunter_expansion::evidence_anchor(&c, 1, &"a".repeat(64), "fingerprint").is_err());
    assert!(hunter_expansion::evidence_anchor(&c, 1, "page.example.com", "domain").is_err());
    assert!(hunter_expansion::evidence_anchor(&c, 1, &"b".repeat(64), "fingerprint").is_ok());
    assert_eq!(
        c.query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        super::super::view_filtered(&c, 2, "", "all", 1, 200).unwrap(),
        other
    );
    change(&mut c, "bad.example.net", "restore");
    assert_eq!(visible(&c, "all"), original);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM asset_discovery_exclusion_events",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn later_harvest_remains_excluded_but_new_independent_reference_protects_shared_hash() {
    let mut c = fixture();
    change(&mut c, "bad.example.net", "apply");
    asset(&c, 1, 6, "new.bad.example.net", 'e', "new.bad.example.net");
    assert!(!visible(&c, "all")
        .iter()
        .any(|s| s["host"] == "e".repeat(64)));
    asset(&c, 1, 7, "valid.example.com", 'a', "valid.example.com");
    assert!(visible(&c, "all")
        .iter()
        .any(|s| s["host"] == "a".repeat(64)));
    assert!(hunter_expansion::evidence_anchor(&c, 1, &"a".repeat(64), "fingerprint").is_ok());
}
#[test]
fn stale_preview_cannot_apply_and_an_excluded_child_prevents_broad_parent_queries() {
    let mut c = fixture();
    let preview = plan(&c, 1, "bad.example.net").unwrap().receipt;
    insert_seed(&c, 1, "new.example.com", "observed_host", "asset:3", false).unwrap();
    assert!(manage(
        &mut c,
        ExclusionInput {
            project_id: 1,
            domain: "bad.example.net".into(),
            action: "apply".into(),
            expected_digest: preview["digest"].as_str().map(str::to_owned)
        }
    )
    .is_err());
    assert!(rules(&c, 1).unwrap().is_empty());
    change(&mut c, "bad.example.com", "apply");
    assert!(hunter_expansion::evidence_anchor(&c, 1, "example.com", "domain_tree").is_err());
    assert!(hunter_expansion::evidence_anchor(&c, 1, "example.com", "domain").is_ok());
    assert!(approved_host(&c, 1, "bad.example.com").is_err());
    for bad in [
        "*.example.com",
        "https://example.com",
        "co.uk",
        "example.com%",
        "192.0.2.1",
    ] {
        assert!(domain(bad).is_err(), "{bad}");
    }
}

#[test]
fn exclusion_covers_the_whole_library_beyond_the_visible_page() {
    let mut c = fixture();
    for i in 0..1003 {
        insert_seed(
            &c,
            1,
            &format!("h{i}.bulk.example.net"),
            "certificate_transparency",
            "bulk.example.net",
            false,
        )
        .unwrap();
    }
    let preview = plan(&c, 1, "bulk.example.net").unwrap().receipt;
    assert_eq!(preview["domainLeads"], 1003);
    assert_eq!(preview["samples"].as_array().unwrap().len(), 12);
    change(&mut c, "bulk.example.net", "apply");
    let excluded =
        super::super::view_filtered(&c, 1, "bulk.example.net", "excluded", 6, 200).unwrap();
    assert_eq!(excluded["seedTotal"], 1003);
    assert_eq!(excluded["seeds"].as_array().unwrap().len(), 3);
    assert_eq!(
        super::super::view_filtered(&c, 1, "bulk.example.net", "all", 1, 200).unwrap()["seedTotal"],
        0
    );
}
