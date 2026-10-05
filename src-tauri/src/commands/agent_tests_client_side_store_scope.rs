// Exercise actual Root/HTTP producer and SDK, with unrelated temporary business data.
const CLIENT_BUSINESS_KEYED: &str = "CREATE TABLE client_business_history(key TEXT PRIMARY KEY, payload BLOB NOT NULL) WITHOUT ROWID; INSERT INTO client_business_history VALUES('asset',x'000102ff');";
#[test]
fn client_side_actual_dispatch_does_not_assume_business_tables_have_rowid() {
    let mut f = client_hook_fixture("client-unrelated-keyed-dispatch", 60000, 20);
    f.db.execute_batch(CLIENT_BUSINESS_KEYED).unwrap();
    let original = client_business_keyed_row(&f.db);
    f.finish()
        .expect("unrelated keyed business records cannot prevent original Client SDK dispatch");
    assert_eq!(client_business_keyed_row(&f.db), original);
    client_business_assert_completed(&f);
}
#[test]
fn client_side_actual_paid_delivery_does_not_assume_business_tables_have_rowid() {
    let mut f = client_hook_fixture("client-unrelated-keyed-delivery", 60000, 20);
    *f.model_boundary_sql.lock().unwrap() = Some(CLIENT_BUSINESS_KEYED.into());
    f.finish()
        .expect("unrelated keyed business records cannot prevent original paid receipt delivery");
    assert_eq!(
        client_business_keyed_row(&f.db),
        ("asset".into(), vec![0, 1, 2, 255])
    );
    client_business_assert_completed(&f);
}
#[test]
fn client_side_actual_unrelated_business_writes_stay_denied_without_snapshot_credit() {
    for paid in [false, true] {
        for write in [
            "INSERT INTO client_business_history VALUES(2,x'01');",
            "UPDATE client_business_history SET payload=x'ff' WHERE id=1;",
            "DELETE FROM client_business_history WHERE id=1;",
        ] {
            let mut f = client_hook_fixture("client-unrelated-business-write", 60000, 20);
            f.db.execute_batch("CREATE TABLE client_business_history(id INTEGER PRIMARY KEY,payload BLOB NOT NULL); INSERT INTO client_business_history VALUES(1,x'000102ff');").unwrap();
            let boundary = if paid {
                "AFTER INSERT ON agent_messages WHEN NEW.from_agent='client_side'"
            } else {
                "AFTER INSERT ON agent_specialist_calls WHEN NEW.role='client_side'"
            };
            let sql =
                format!("CREATE TRIGGER client_unrelated_write {boundary} BEGIN {write} END;");
            if paid {
                *f.model_boundary_sql.lock().unwrap() = Some(sql);
            } else {
                f.db.execute_batch(&sql).unwrap();
            }
            let before = client_hook_untouched_scope(&f);
            let costs = client_hook_target_costs(&f.db);
            let error = f.finish().unwrap_err();
            let expected = if paid {
                "client_side_delivery_write_rejected:"
            } else {
                "client_side_sdk_dispatch_write_rejected:"
            };
            assert!(error.contains(expected), "{paid}/{write}: {error}");
            assert_eq!(client_hook_untouched_scope(&f), before);
            assert_eq!(client_hook_target_costs(&f.db), costs);
            client_hook_assert_supplier_closure(&f);
            if paid {
                client_delivery_assert_paid_and_running(&f);
            } else {
                client_sdk_dispatch_assert_original_unsent_grant(&f);
            }
        }
    }
}
#[test]
fn client_side_actual_large_business_history_preserves_paid_delivery_and_replay() {
    let mut f = client_hook_fixture("client-large-unrelated-business", 60000, 20);
    f.db.execute_batch("CREATE TABLE client_business_history(id INTEGER PRIMARY KEY,payload BLOB NOT NULL); WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<107558) INSERT INTO client_business_history SELECT i,randomblob(512) FROM n;").unwrap();
    let original = client_business_history_digest(&f.db);
    let started = std::time::Instant::now();
    f.finish().unwrap();
    let delivered_ms = started.elapsed().as_millis();
    assert_eq!(client_business_history_digest(&f.db), original);
    client_business_assert_completed(&f);
    let paid = client_delivery_rows(&f.db, "agent_budget_entries");
    let started = std::time::Instant::now();
    f.finish().unwrap();
    let replay_ms = started.elapsed().as_millis();
    assert_eq!(client_business_history_digest(&f.db), original);
    assert_eq!(client_delivery_rows(&f.db, "agent_budget_entries"), paid);
    client_business_assert_completed(&f);
    eprintln!("temporary unrelated history rows={} bytes={} delivered_ms={delivered_ms} replay_ms={replay_ms}",original.0,original.1);
}
fn client_business_keyed_row(db: &rusqlite::Connection) -> (String, Vec<u8>) {
    db.query_row("SELECT key,payload FROM client_business_history", [], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .unwrap()
}
fn client_business_history_digest(db: &rusqlite::Connection) -> (i64, i64, String) {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut q = db
        .prepare("SELECT id,payload FROM client_business_history ORDER BY id")
        .unwrap();
    let mut rows = q.query([]).unwrap();
    let (mut count, mut bytes) = (0i64, 0i64);
    while let Some(row) = rows.next().unwrap() {
        let id: i64 = row.get(0).unwrap();
        let payload: Vec<u8> = row.get(1).unwrap();
        hash.update(id.to_be_bytes());
        hash.update((payload.len() as u64).to_be_bytes());
        hash.update(&payload);
        count += 1;
        bytes += payload.len() as i64;
    }
    assert_eq!((count, bytes), (107558, 55069696));
    (count, bytes, format!("{:x}", hash.finalize()))
}
fn client_business_assert_completed(f: &ClientHookFixture) {
    client_hook_assert_supplier_closure(f);
    assert_eq!(f.model_seen.lock().unwrap().len(), 1);
    assert_eq!(f.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side' AND state='completed'",[&f.root],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(f.db.query_row("SELECT count(*) FROM agent_messages WHERE from_agent='client_side' AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts=1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
