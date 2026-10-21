use super::*;
#[test]
fn pem_and_der_are_indexed_as_real_leaf_fingerprints_but_names_and_serials_are_not() {
    use base64::Engine;
    let c = fixture();
    let pem = include_str!("../../test-fixtures/tls/leaf.pem");
    let hash = include_str!("../../test-fixtures/tls/leaf.sha256").trim();
    asset(&c, 1, 1, "https://localhost", json!({"ssl_certificate":pem}));
    asset(&c, 1, 2, "https://other.example", json!({"probe_cert_der_base64":base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../test-fixtures/tls/leaf.der"))}));
    asset(&c, 1, 3, "https://only-name.example", json!({"cert.subject.cn":"only-name.example","cert.sn":"a".repeat(64),"sha256":"b".repeat(64),"body_sha256":"c".repeat(64)}));
    index(&c,1);index(&c,1);
    assert_eq!(c.query_row("SELECT count(*) FROM asset_discovery_seeds WHERE source='certificate_sha256' AND host=?1",[hash],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(c.query_row("SELECT count(*) FROM asset_discovery_seed_evidence WHERE source='certificate_sha256' AND host=?1",[hash],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert!(certificate_facts(&json!({"cert.sn":"a".repeat(64),"sha256":"b".repeat(64),"body_sha256":"c".repeat(64)})).hashes.is_empty());
    assert_eq!(lookup_on(&c,1,hash).unwrap()["certificateCount"],1);
    assert_eq!(lookup_on(&c,2,hash).unwrap()["matchCount"],0);
}
fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,domain TEXT DEFAULT '',extra_json TEXT);CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0,PRIMARY KEY(project_id,asset_id));").unwrap();
    c
}
fn asset(c: &Connection, p: i64, id: i64, endpoint: &str, extra: Value) {
    c.execute(
        "INSERT INTO assets(id,host,link,extra_json) VALUES(?1,?2,?2,?3)",
        params![id, endpoint, extra.to_string()],
    )
    .unwrap();
    c.execute("INSERT INTO project_assets VALUES(?1,?2,0)", params![p, id])
        .unwrap();
}
fn index(c: &Connection, p: i64) {
    let rows=c.prepare("SELECT a.id,a.host,a.link,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0").unwrap().query_map([p],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    ingest_observations(c, p, &rows).unwrap();
}
#[test]
fn direct_asset_and_certificate_are_found_before_indexing_without_any_write_or_network() {
    let c = fixture();
    asset(
        &c,
        1,
        1,
        "https://Login.Example.com:8443/path",
        json!({"cert_sha256":"a".repeat(64),"ssl_certificate":"saved certificate content","hunter_key":"synthetic-secret-never-return"}),
    );
    let tables: i64 = c
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
        .unwrap();
    let found = lookup_on(&c, 1, "LOGIN.example.com").unwrap();
    assert_eq!(found["endpointMatches"], 1);
    assert_eq!(found["certificateMatches"], 1);
    assert_eq!(found["networkRequested"], false);
    assert!(!found.to_string().contains("synthetic-secret"));
    assert_eq!(found["certificates"][0]["eligible"], false);
    assert_eq!(
        c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        tables
    );
    assert_eq!(lookup_on(&c, 1, "example.com").unwrap()["matchCount"], 0);
    assert_eq!(host("login.example.com:443").unwrap(), "login.example.com");
}
#[test]
fn certificate_names_find_ip_assets_without_asn_restrictions_and_offer_the_saved_fingerprint() {
    let c = fixture();
    asset(
        &c,
        1,
        1501,
        "https://192.0.2.14:443/",
        json!({"cert.domain":"api.example.com","cert_sha256":"b".repeat(64)}),
    );
    index(&c, 1);
    c.execute(
        "INSERT INTO asset_discovery_scopes VALUES(1,'[\"example.com\"]')",
        [],
    )
    .unwrap();
    let found = lookup_on(&c, 1, "api.example.com").unwrap();
    assert_eq!(found["endpointMatches"], 0);
    assert_eq!(found["certificateMatches"], 1);
    assert_eq!(found["assets"][0]["matchType"], "certificate_exact");
    assert_eq!(found["certificates"][0]["fingerprint"], "b".repeat(64));
    assert_eq!(found["certificates"][0]["eligible"], true);
    assert_eq!(
        lookup_on(&c, 1, &"b".repeat(64)).unwrap()["assets"][0]["matchType"],
        "fingerprint"
    );
}
#[test]
fn wildcard_is_only_a_one_label_certificate_relation_not_a_discovered_host() {
    let c = fixture();
    asset(
        &c,
        1,
        1,
        "https://192.0.2.4/",
        json!({"cert.domain":"*.example.com","ssl_certificate":"certificate content"}),
    );
    index(&c, 1);
    let found = lookup_on(&c, 1, "app.example.com").unwrap();
    assert_eq!(found["endpointMatches"], 0);
    assert_eq!(found["certificateMatches"], 1);
    assert_eq!(found["assets"][0]["matchType"], "certificate_wildcard");
    assert_eq!(found["certificateCount"], 0);
    for name in [
        "example.com",
        "a.app.example.com",
        "badexample.com",
        "example.com.attacker.test",
    ] {
        assert_eq!(lookup_on(&c, 1, name).unwrap()["matchCount"], 0, "{name}");
    }
}
#[test]
fn deleted_and_other_project_certificates_are_not_local_matches() {
    let c = fixture();
    for (p, id) in [(1, 1), (2, 2)] {
        asset(
            &c,
            p,
            id,
            "https://shared.example.com",
            json!({"cert.domain":"shared.example.com","cert_sha256":"a".repeat(64)}),
        );
        index(&c, p);
    }
    c.execute(
        "UPDATE project_assets SET is_deleted=1 WHERE project_id=1",
        [],
    )
    .unwrap();
    assert_eq!(
        lookup_on(&c, 1, "shared.example.com").unwrap()["matchCount"],
        0
    );
    assert_eq!(
        lookup_on(&c, 2, "shared.example.com").unwrap()["matchCount"],
        1
    );
}
#[test]
fn structured_certificate_content_is_harvested_but_arbitrary_certificate_text_never_invents_a_hash()
{
    let c = fixture();
    asset(
        &c,
        1,
        1,
        "https://192.0.2.5",
        json!({"ssl_certificate":json!({"sha256":"c".repeat(64),"subject":{"CN":"structured.example.com"},"dns_names":["api.example.com"]}).to_string()}),
    );
    index(&c, 1);
    let found = lookup_on(&c, 1, "structured.example.com").unwrap();
    assert_eq!(found["certificateMatches"], 1);
    assert_eq!(found["certificates"][0]["fingerprint"], "c".repeat(64));
    assert!(
        certificate_facts(&json!({"ssl_certificate":"not a parsed DER certificate"}))
            .hashes
            .is_empty()
    );
}
#[test]
fn searchable_seed_pages_reach_older_than_the_last_200_and_escape_literal_wildcards() {
    let c = fixture();
    ensure(&c).unwrap();
    for n in 0..503 {
        insert_seed(
            &c,
            1,
            &format!("seed-{n}.example.com"),
            "observed_host",
            "asset:1",
            false,
        )
        .unwrap();
    }
    let p = view_filtered(&c, 1, "seed-0.example.com", "all", 1, 50).unwrap();
    assert_eq!(p["seedTotal"], 1);
    assert_eq!(p["seeds"][0]["host"], "seed-0.example.com");
    assert_eq!(p["allSeeds"], 503);
    let last = view_filtered(&c, 1, "", "all", 11, 50).unwrap();
    assert_eq!(last["seeds"].as_array().unwrap().len(), 3);
    assert_eq!(last["seedPage"], 11);
    assert_eq!(
        view_filtered(&c, 1, "%", "all", 1, 50).unwrap()["seedTotal"],
        0
    );
    assert!(view_filtered(&c, 1, "", "not-a-source", 1, 50).is_err());
    assert!(view_filtered(&c, 1, "", "all", 1, 0).is_err());
}
