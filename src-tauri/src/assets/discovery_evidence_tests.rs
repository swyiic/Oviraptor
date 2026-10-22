use super::*;
fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,extra_json TEXT,probe_outcome TEXT DEFAULT '',content_category TEXT DEFAULT '');CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,decision TEXT DEFAULT 'pending',note TEXT DEFAULT '');").unwrap();
    ensure(&c).unwrap();
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.com\"]')",
        [],
    )
    .unwrap();
    c
}
fn asset(c: &Connection, id: i64, names: &str, deleted: bool) {
    let extra = json!({"cert_sha256":"a".repeat(64),"cert.domain":names});
    c.execute("INSERT INTO assets(id,host,link,extra_json,probe_outcome) VALUES(?1,'https://app.example.com','https://app.example.com',?2,'blocked_content')",params![id,extra.to_string()]).unwrap();
    c.execute(
        "INSERT INTO project_assets VALUES(1,?1,?2,'rejected','保留原规则命中')",
        params![id, deleted],
    )
    .unwrap();
    ingest_observations(
        c,
        1,
        &[(
            id,
            "app.example.com".into(),
            "https://app.example.com".into(),
            extra.to_string(),
        )],
    )
    .unwrap();
}
#[test]
fn approved_host_does_not_admit_an_unrelated_certificate_or_shared_cname() {
    let c = fixture();
    asset(&c, 1, "unrelated.test", false);
    assert!(hunter_expansion::evidence_anchor(&c, 1, &"a".repeat(64), "fingerprint").is_err());
    insert_seed(&c, 1, "shared.cdn.test", "cname", "asset:1", false).unwrap();
    assert!(hunter_expansion::evidence_anchor(&c, 1, "shared.cdn.test", "cname").is_err());
}
#[test]
fn certificate_identity_cannot_borrow_names_from_another_certificate() {
    let roots = vec!["example.com".into()];
    let v = json!({"cert_sha256":"a".repeat(64),"cert.domain":"app.example.com","probe_cert_sha256":"b".repeat(64),"probe_cert_dns_names_json":"[\"other.test\"]"});
    assert!(certificate_anchor(&v, &"a".repeat(64), &roots).is_none());
    assert!(certificate_anchor(&v, &"b".repeat(64), &roots).is_none());
    assert!(certificate_anchor(
        &json!({"cert_sha256":"a".repeat(64)}),
        &"a".repeat(64),
        &roots
    )
    .is_none());
    assert!(certificate_anchor(
        &json!({"cert_sha256":"a".repeat(64),"cert.domain":["app.example.com","shared.test"]}),
        &"a".repeat(64),
        &roots
    )
    .is_none());
    assert_eq!(
        certificate_anchor(
            &json!({"cert_sha256":"a".repeat(64),"cert.domain":"*.example.com"}),
            &"a".repeat(64),
            &roots
        )
        .as_deref(),
        Some("example.com")
    );
}
#[test]
fn quarantine_review_keeps_rules_history_and_manual_trash_unchanged() {
    let c = fixture();
    asset(&c, 1, "app.example.com", true);
    asset(&c, 2, "unrelated.test", false);
    let before = c.total_changes();
    let result = quarantine_batch(&c, 1, 0).unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["assetId"], 1);
    assert_eq!(result["items"][0]["isDeleted"], true);
    assert_eq!(result["networkRequested"], false);
    assert_eq!(result["dataModified"], false);
    assert_eq!(c.total_changes(), before);
    assert!(quarantine_batch(&c, 2, 0).is_err());
    assert!(hunter_expansion::evidence_anchor(&c, 1, &"a".repeat(64), "fingerprint").is_err());
}
#[test]
fn valid_raw_der_controls_identity_instead_of_conflicting_metadata() {
    let v = json!({"certificate_pem":include_str!("../../test-fixtures/tls/leaf.pem"),"cert_sha256":"a".repeat(64),"cert.domain":"app.example.com"});
    assert!(certificate_anchor(&v, &"a".repeat(64), &["example.com".into()]).is_none());
    let hash = include_str!("../../test-fixtures/tls/leaf.sha256").trim();
    assert!(certificate_anchor(&v, hash, &["fixture.example.test".into()]).is_some());
}

#[test]
fn cn_only_leaf_uses_the_same_identity_for_paid_admission_and_local_extraction() {
    use base64::Engine;
    let cert = crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/cn-only.der")).unwrap();
    assert!(cert.dns_names.is_empty());
    let v = json!({"probe_cert_der_base64":base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../test-fixtures/tls/cn-only.der"))});
    assert_eq!(certificate_anchor(&v,&cert.sha256,&["fixture.example.test".into()]).as_deref(),Some("fixture.example.test"));
    assert!(certificate_anchor(&v,&cert.sha256,&["other.example.test".into()]).is_none());
    let cert = crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/ip-san.der")).unwrap();
    let v = json!({"probe_cert_der_base64":cert.der_base64});
    assert!(certificate_anchor(&v,&cert.sha256,&["fixture.example.test".into()]).is_none(),"an IP SAN cannot borrow DNS CN for paid-query admission");
}
