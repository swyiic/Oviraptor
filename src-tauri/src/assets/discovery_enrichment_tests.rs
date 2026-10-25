use super::*;
#[test]
fn storage_contention_is_retryable_without_classifying_scope_or_network_failures_as_storage(){
    for value in ["database is locked","database table is locked: asset_enrichment_items","database schema is locked: main","SQLITE_BUSY_SNAPSHOT","SQLITE_LOCKED"]{assert!(storage_busy(value),"{value}");}
    for value in ["确认范围已变化","non_public_address","timeout contacting crt.sh","certificate_invalid","no such table: missing"]{assert!(!storage_busy(value),"{value}");}
}

#[test]
fn cleanup_blocks_old_free_queue_before_network_and_independent_evidence_can_restore_eligibility() {
    let(dir,mut c,id)=fixture();
    insert_seed(&c,9001,"a.example.test","certificate_domain","unrelated.example.net",false).unwrap();
    enqueue_host(&c,9001,&id,"a.example.test").unwrap();
    let preview=exclusions::manage(&mut c,exclusions::ExclusionInput{project_id:9001,domain:"unrelated.example.net".into(),action:"preview".into(),expected_digest:None}).unwrap();
    exclusions::manage(&mut c,exclusions::ExclusionInput{project_id:9001,domain:"unrelated.example.net".into(),action:"apply".into(),expected_digest:preview["digest"].as_str().map(str::to_owned)}).unwrap();
    assert!(!admitted(&c,9001,"a.example.test").unwrap());
    assert!(claim(&mut c,9001,&id).unwrap().is_none());
    let path=dir.join("oviraptor.sqlite3");let client=network::client("").unwrap();
    for kind in ["ct","dns_a","dns_aaaa","dns_cname","tls"] {
        let item=Item{id:0,kind:kind.into(),host:"a.example.test".into(),target:"https://a.example.test/".into()};
        assert!(network::execute(&path,&id,&item,&client).unwrap_err().contains("排除"),"{kind}");
    }
    // The same name has a separate live provenance edge. It is not a global
    // domain exclusion, so a new independent observation remains usable.
    insert_seed(&c,9001,"a.example.test","certificate_domain","good.example.test",false).unwrap();
    assert!(admitted(&c,9001,"a.example.test").unwrap());
    enqueue_host(&c,9001,&id,"a.example.test").unwrap();
    assert!(claim(&mut c,9001,&id).unwrap().is_some());
    drop(c);std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn legacy_empty_exclusion_rules_do_not_require_or_create_provenance_tables() {
    let(dir,c,_id)=fixture();
    c.execute_batch("CREATE TABLE IF NOT EXISTS asset_discovery_domain_exclusions(project_id INTEGER NOT NULL,domain TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 1,updated_at TEXT NOT NULL DEFAULT '',PRIMARY KEY(project_id,domain));
        DROP TABLE IF EXISTS asset_discovery_excluded_evidence;").unwrap();
    let before=c.total_changes();c.execute_batch("PRAGMA query_only=ON").unwrap();
    assert!(admitted(&c,9001,"a.example.test").unwrap());
    assert_eq!(hunter_expansion::visible_seed_sql(&c,9001).unwrap(),"1");
    assert!(!c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_excluded_evidence')",[],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(c.total_changes(),before);
    c.execute_batch("PRAGMA query_only=OFF;INSERT INTO asset_discovery_domain_exclusions(project_id,domain,enabled) VALUES(9001,'other.example.net',1)").unwrap();
    // An active rule with a damaged/missing provenance schema cannot silently
    // re-enable its related names; report the explicit repairable owner error.
    assert_eq!(admitted(&c,9001,"a.example.test").unwrap_err(),"discovery_exclusion_provenance_unavailable");
    drop(c);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn received_free_result_waits_for_sqlite_writer_and_commits_once_without_another_request(){
    let(dir,mut c,id)=fixture();
    enqueue(&c,&id,"dns_a","example.test","example.test").unwrap();let item=claim(&mut c,9001,&id).unwrap().unwrap();
    let path:String=c.query_row("PRAGMA database_list",[],|r|r.get(2)).unwrap();
    let signal=Arc::new(Signal::default());let worker_signal=signal.clone();
    let(ready_tx,ready_rx)=std::sync::mpsc::channel();let(go_tx,go_rx)=std::sync::mpsc::channel();
    let saved=id.clone();let worker=thread::spawn(move||{
        let mut c=db::open(Path::new(&path)).unwrap();c.busy_timeout(Duration::from_millis(20)).unwrap();ready_tx.send(()).unwrap();go_rx.recv().unwrap();
        finish_received(&mut c,9001,&saved,&item,Ok(json!({"records":[],"summary":"one actual received response"})),&worker_signal)
    });
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();c.execute_batch("BEGIN IMMEDIATE").unwrap();go_tx.send(()).unwrap();
    let start=std::time::Instant::now();while !signal.storage_busy.load(Ordering::SeqCst){assert!(start.elapsed()<Duration::from_secs(2));thread::sleep(Duration::from_millis(5));}
    assert_eq!(c.query_row("SELECT enabled FROM asset_enrichment_runs WHERE id=?1",[&id],|r|r.get::<_,bool>(0)).unwrap(),true);
    c.execute_batch("ROLLBACK").unwrap();worker.join().unwrap().unwrap();
    let row:(String,i64,i64)=c.query_row("SELECT status,attempts,failures FROM asset_enrichment_items WHERE run_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(row,("done".into(),1,0));
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_observations WHERE project_id=9001",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_events WHERE run_id=?1",[&id],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(c);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn receipt_event_busy_rolls_back_failure_increment_before_local_retry(){
    let(dir,mut c,id)=fixture();enqueue(&c,&id,"dns_a","example.test","example.test").unwrap();let item=claim(&mut c,9001,&id).unwrap().unwrap();
    c.execute_batch("CREATE TRIGGER emulate_receipt_busy BEFORE INSERT ON asset_enrichment_events BEGIN SELECT RAISE(ABORT,'database is locked'); END;").unwrap();
    assert!(storage_busy(&finish(&mut c,9001,&id,&item,Err("network timeout".into())).unwrap_err()));
    assert_eq!(c.query_row("SELECT failures FROM asset_enrichment_items WHERE id=?1",[item.id],|r|r.get::<_,i64>(0)).unwrap(),0);
    c.execute_batch("DROP TRIGGER emulate_receipt_busy").unwrap();finish(&mut c,9001,&id,&item,Err("network timeout".into())).unwrap();
    assert_eq!(c.query_row("SELECT failures FROM asset_enrichment_items WHERE id=?1",[item.id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_events WHERE run_id=?1",[&id],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(c);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn stop_and_wait_share_registration_mutex_and_wake_without_the_fifteen_second_poll() {
    let signal = Arc::new(Signal::default());
    let guard = signal.lock.lock().unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (stopped_tx, stopped_rx) = std::sync::mpsc::channel();
    let worker_signal = signal.clone();
    let stopper = thread::spawn(move || {
        started_tx.send(()).unwrap();
        worker_signal.stop();
        stopped_tx.send(()).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    // Replay the boundary after a false predicate check but before entering
    // Condvar::wait. Stop must not publish/notify while that guard is held.
    assert!(matches!(stopped_rx.recv_timeout(Duration::from_millis(50)), Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
    assert!(!signal.stop.load(Ordering::SeqCst));
    let (_guard, result) = signal.wake.wait_timeout_while(guard, Duration::from_secs(5), |_| !signal.stop.load(Ordering::SeqCst)).unwrap();
    assert!(!result.timed_out(), "stop notification must wake the registered waiter");
    assert!(signal.stop.load(Ordering::SeqCst));
    stopped_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    stopper.join().unwrap();

    for _ in 0..32 {
        let signal = Arc::new(Signal::default());
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let waiting = signal.clone();
        let waiter = thread::spawn(move || { ready_tx.send(()).unwrap(); waiting.wait(15); done_tx.send(()).unwrap(); });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        thread::yield_now();
        signal.stop();
        done_rx.recv_timeout(Duration::from_secs(5)).expect("actual Signal::wait must wake instead of sleeping until its 15-second poll");
        waiter.join().unwrap();
    }
}
#[test]
fn rejects_vpn_fake_ips_and_non_public_addresses() {
    use super::super::certificate_tasks::tls::public;
    for ip in [
        "198.18.0.1",
        "198.19.255.254",
        "100.64.0.1",
        "127.0.0.1",
        "10.0.0.1",
        "192.0.2.1",
        "::1",
    ] {
        assert!(!public(ip.parse().unwrap()), "{ip}");
    }
    assert!(public("8.8.8.8".parse().unwrap()));
}
#[test]
fn wildcard_certificate_is_only_one_label() {
    assert!(network::covers("a.example.com", "*.example.com"));
    assert!(!network::covers("example.com", "*.example.com"));
    assert!(!network::covers("a.b.example.com", "*.example.com"));
}
fn fixture() -> (std::path::PathBuf,Connection,String) {
 let dir=std::env::temp_dir().join(format!("ovi-enrichment-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&dir).unwrap();
 let path=db::initialize(&dir).unwrap();let c=db::open(&path).unwrap();c.execute("INSERT INTO projects(id,name) VALUES(9001,'Synthetic'),(9002,'Other')",[]).unwrap();ensure(&c).unwrap();c.execute_batch(TABLES).unwrap();
 c.execute("INSERT INTO asset_discovery_scopes VALUES(9001,'[\"example.test\"]')",[]).unwrap();
 c.execute("INSERT INTO asset_enrichment_runs(id,project_id,operation_id,roots_json,status,enabled,continuous,interval_seconds,workers,created_at,updated_at) VALUES('saved',9001,'op','[\"example.test\"]','running',1,0,3600,1,1,1)",[]).unwrap();
 (dir,c,"saved".into())
}
#[test]
fn pause_commits_pausing_before_notifying_an_owner_that_can_finish_immediately() {
    let (dir,c,id)=fixture();
    let signal=Arc::new(Signal::default());
    // Use this fixture's actual initialized database path, independent of other
    // tests' global owner registry and without starting any network worker.
    let path:String=c.query_row("PRAGMA database_list",[],|r|r.get(2)).unwrap();
    let manager=Manager{path:path.into(),active:Arc::new(Mutex::new(HashMap::from([(id.clone(),signal.clone())])))};
    let guard=signal.lock.lock().unwrap();
    let owner=manager.clone();let saved=id.clone();
    let pauser=thread::spawn(move || owner.pause(9001,&saved));
    let start=std::time::Instant::now();
    let mut persisted=false;
    while start.elapsed()<Duration::from_secs(2) {
        if c.query_row("SELECT status FROM asset_enrichment_runs WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap()=="pausing" {persisted=true;break;}
        thread::sleep(Duration::from_millis(2));
    }
    // stop() cannot pass the held Signal mutex. The durable state must already
    // be committed; otherwise a fast exit can write paused before pause writes
    // pausing and leave a permanently stale UI state.
    assert!(!signal.stop.load(Ordering::SeqCst));
    drop(guard);pauser.join().unwrap().unwrap();
    assert!(persisted,"pausing must be persisted before stop is signalled, not after the owner can exit");
    c.execute("UPDATE asset_enrichment_runs SET status='paused',reason='owner exit receipt' WHERE id=?1",[&id]).unwrap();
    manager.pause(9001,&id).unwrap();
    assert_eq!(c.query_row("SELECT status,reason FROM asset_enrichment_runs WHERE id=?1",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).unwrap(),("paused".into(),"owner exit receipt".into()),"a repeated pause must not overwrite the exited owner's final receipt");
    for state in ["paused","interrupted","completed","scope_changed"] {
        c.execute("UPDATE asset_enrichment_runs SET status=?1,reason='exiting owner receipt' WHERE id=?2",params![state,id]).unwrap();
        // An internally failed owner may have committed its terminal receipt
        // but still be waiting to remove its active entry, without any stop.
        signal.stop.store(false,Ordering::SeqCst);
        manager.pause(9001,&id).unwrap();
        assert_eq!(c.query_row("SELECT status,reason FROM asset_enrichment_runs WHERE id=?1",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).unwrap(),(state.to_string(),"exiting owner receipt".into()));
    }
    drop(c);drop(manager);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn ct_tail_beyond_1000_is_processed_and_only_scoped_names_survive() {
 let rows=(0..1501).map(|i|json!({"id":i,"name_value":format!("n{i}.example.test\nother.invalid\n*.example.test")})).collect::<Vec<_>>();
 let value=network::ct(&json!(rows),"example.test").unwrap();assert_eq!(value["entries"],1501);assert_eq!(value["names"].as_array().unwrap().len(),1502);assert!(value["names"].as_array().unwrap().iter().any(|v|v["name"]=="n1500.example.test"));assert!(!value.to_string().contains("other.invalid"));
}
#[test]
fn dns_requires_matching_complete_answers_and_follows_only_the_requested_chain() {
 let value=json!({"Status":0,"TC":false,"Question":[{"name":"a.example.test.","type":1}],"Answer":[{"name":"evil.test.","type":1,"data":"8.8.4.4"},{"name":"outside.cdn.test.","type":1,"data":"8.8.8.8"},{"name":"a.example.test.","type":5,"data":"outside.cdn.test."}]});
 let records=network::dns(&value,"a.example.test",1).unwrap();assert_eq!(records["records"].as_array().unwrap().len(),2);assert!(!records.to_string().contains("evil.test"));
 assert!(network::dns(&value,"b.example.test",1).is_err());let mut truncated=value.clone();truncated["TC"]=json!(true);assert!(network::dns(&truncated,"a.example.test",1).is_err());truncated["TC"]=json!(false);truncated["Status"]=json!(5);assert!(network::dns(&truncated,"a.example.test",1).is_err());
 truncated["Status"]=json!(3);truncated["Answer"]=json!([]);assert_eq!(network::dns(&truncated,"a.example.test",1).unwrap()["status"],3);
}
#[test]
fn external_cname_is_saved_without_expanding_the_network_queue_and_pause_stops_claims() {
 let(dir,mut c,id)=fixture();enqueue_host(&c,9001,&id,"a.example.test").unwrap();let before:i64=c.query_row("SELECT COUNT(*) FROM asset_enrichment_items",[],|r|r.get(0)).unwrap();
 let item=Item{id:1,kind:"dns_cname".into(),host:"a.example.test".into(),target:"a.example.test".into()};
 persist(&mut c,9001,&id,&item,&json!({"records":[{"type":5,"data":"external.cdn.test"}]}),&["example.test".into()]).unwrap();
 assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_items",[],|r|r.get::<_,i64>(0)).unwrap(),before);
 assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_discovery_seeds WHERE host='external.cdn.test'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
 c.execute("UPDATE asset_enrichment_runs SET enabled=0",[]).unwrap();assert!(claim(&mut c,9001,&id).unwrap().is_none());assert!(owned(&c,9002,&id).is_err());drop(c);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn revocation_before_result_prevents_any_new_evidence_and_retry_is_not_a_human_conflict() {
 let(dir,mut c,id)=fixture();enqueue(&c,&id,"ct","example.test","example.test").unwrap();let item=claim(&mut c,9001,&id).unwrap().unwrap();
 finish(&mut c,9001,&id,&item,Err("crt.sh HTTP 403; access denied".into())).unwrap();
 assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_conflicts",[],|r|r.get::<_,i64>(0)).unwrap(),0);assert_eq!(c.query_row("SELECT status FROM asset_enrichment_items",[],|r|r.get::<_,String>(0)).unwrap(),"retry");
 c.execute("UPDATE asset_discovery_scopes SET roots_json='[]' WHERE project_id=9001",[]).unwrap();assert!(finish(&mut c,9001,&id,&item,Ok(json!({"names":[{"name":"new.example.test"}]}))).is_err());
 assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_observations",[],|r|r.get::<_,i64>(0)).unwrap(),0);drop(c);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn real_der_writes_only_certificate_evidence_and_does_not_restore_recycled_asset() {
 let(dir,mut c,id)=fixture();let cert=crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/leaf.der")).unwrap();
 let endpoint="https://fixture.example.test/";let key=endpoint;
 c.execute("INSERT INTO assets(id,asset_key,canonical_key,host,link,extra_json,content_category,probe_outcome) VALUES(4,?1,?1,?2,?2,?3,'page_rule_isolated','web_alive')",params![key,endpoint,json!({"body":"original-page","provider":"original"}).to_string()]).unwrap();c.execute("INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(9001,4,1)",[]).unwrap();
 insert_seed(&c,9001,"fixture.example.test","observed_host","asset:4",false).unwrap();enqueue(&c,&id,"tls","fixture.example.test",endpoint).unwrap();let item=claim(&mut c,9001,&id).unwrap().unwrap();
 finish(&mut c,9001,&id,&item,Ok(json!({"der":cert.der_base64,"sha256":cert.sha256,"hostnameMatches":true}))).unwrap();
 let raw:String=c.query_row("SELECT extra_json FROM assets WHERE id=4",[],|r|r.get(0)).unwrap();let e:Value=serde_json::from_str(&raw).unwrap();assert_eq!(e["probe_cert_sha256"],cert.sha256);assert_eq!(e["body"],"original-page");assert_eq!(c.query_row("SELECT is_deleted FROM project_assets WHERE asset_id=4",[],|r|r.get::<_,i64>(0)).unwrap(),1);assert_eq!(c.query_row("SELECT content_category FROM assets WHERE id=4",[],|r|r.get::<_,String>(0)).unwrap(),"page_rule_isolated");
 // Actual DER coverage wins over a mistaken/malicious result flag.
 let mut wrong=item.clone();wrong.host="other.example.test".into();persist_tls(&mut c,9001,&id,&wrong,&json!({"der":cert.der_base64,"hostnameMatches":true}),&["example.test".into()]).unwrap();wrong.host="second.example.test".into();persist_tls(&mut c,9001,&id,&wrong,&json!({"der":cert.der_base64,"hostnameMatches":true}),&["example.test".into()]).unwrap();assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_conflicts",[],|r|r.get::<_,i64>(0)).unwrap(),1);
 drop(c);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn sync_is_incremental_deduplicated_and_does_not_reset_finished_items() {
 let(dir,mut c,id)=fixture();insert_seed(&c,9001,"a.example.test","observed_host","old",false).unwrap();sync(&mut c,9001,&id).unwrap();let n:i64=c.query_row("SELECT COUNT(*) FROM asset_enrichment_items",[],|r|r.get(0)).unwrap();c.execute("UPDATE asset_enrichment_items SET status='done',attempts=1,next_at=?1",[now()+500]).unwrap();sync(&mut c,9001,&id).unwrap();assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_items",[],|r|r.get::<_,i64>(0)).unwrap(),n);assert!(claim(&mut c,9001,&id).unwrap().is_none());
 insert_seed(&c,9001,"new.a.example.test","certificate_transparency","ct",false).unwrap();sync(&mut c,9001,&id).unwrap();assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_items WHERE host='new.a.example.test'",[],|r|r.get::<_,i64>(0)).unwrap(),4);drop(c);std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn complete_certificate_names_cannot_hide_an_external_san_after_128_entries() {
 let cert=crate::certificates::from_der(include_bytes!("../../test-fixtures/tls/many-san.der")).unwrap();
 assert_eq!(cert.dns_names.len(),141);assert!(cert.dns_names.iter().any(|n|n=="outside.invalid"));
}
#[test]
fn scope_change_on_resume_releases_the_active_slot_without_network_or_other_project_writes() {
 let(dir,c,id)=fixture();let m=Manager{path:dir.join("oviraptor.sqlite3"),active:Arc::new(Mutex::new(HashMap::new()))};
 c.execute("UPDATE asset_discovery_scopes SET roots_json='[]' WHERE project_id=9001",[]).unwrap();
 assert!(m.resume(9002,&id).is_err());assert_eq!(c.query_row("SELECT enabled FROM asset_enrichment_runs WHERE id=?1",[&id],|r|r.get::<_,bool>(0)).unwrap(),true);
 assert!(m.resume(9001,&id).is_err());assert_eq!(c.query_row("SELECT enabled FROM asset_enrichment_runs WHERE id=?1",[&id],|r|r.get::<_,bool>(0)).unwrap(),false);assert_eq!(c.query_row("SELECT status FROM asset_enrichment_runs WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"scope_changed");
 assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_events",[],|r|r.get::<_,i64>(0)).unwrap(),0);drop(c);std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn project_scan_finds_the_tail_san_on_an_external_ip_without_unscoped_network_work() {
    let(dir,mut c,id)=fixture();
    for n in 1..=501 {
        let endpoint=format!("https://198.51.100.10/{n}");
        let extra=if n==501 {json!({"cert.domain":"new.example.test","cert.subject.org":"目标单位"})}else{json!({"cert.domain":"unrelated.invalid"})};
        c.execute("INSERT INTO assets(id,asset_key,canonical_key,host,link,extra_json) VALUES(?1,?2,?2,?2,?2,?3)",params![n,endpoint,extra.to_string()]).unwrap();
        c.execute("INSERT INTO project_assets(project_id,asset_id) VALUES(9001,?1)",[n]).unwrap();
    }
    let mut pages=0;
    loop {pages+=1;if !sync(&mut c,9001,&id).unwrap(){break}assert!(pages<10);}
    assert!(pages>=3);
    assert!(c.query_row("SELECT EXISTS(SELECT 1 FROM asset_enrichment_items WHERE run_id=?1 AND kind='tls' AND host='new.example.test')",[&id],|r|r.get::<_,bool>(0)).unwrap());
    assert!(!c.query_row("SELECT EXISTS(SELECT 1 FROM asset_enrichment_items WHERE host IN ('unrelated.invalid','198.51.100.10'))",[],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(c.query_row("SELECT count(*) FROM asset_enrichment_observations",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(c);std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn one_pass_waits_for_persisted_retries_and_bounds_permanent_or_repeated_failures() {
    let (dir, mut c, id) = fixture();
    enqueue(&c,&id,"ct","example.test","example.test").unwrap();
    let item=claim(&mut c,9001,&id).unwrap().unwrap();
    finish(&mut c,9001,&id,&item,Err("transient connection reset".into())).unwrap();
    assert!(!finish_single_pass(&c,&id,false).unwrap(),"a delayed retry is unfinished work");
    assert!(claim(&mut c,9001,&id).unwrap().is_none(),"backoff is persisted, never spin-retried");
    for _ in 1..MAX_FREE_FAILURES {
        c.execute("UPDATE asset_enrichment_items SET next_at=0 WHERE id=?1",[item.id]).unwrap();
        let next=claim(&mut c,9001,&id).unwrap().unwrap();
        finish(&mut c,9001,&id,&next,Err("transient connection reset".into())).unwrap();
    }
    assert_eq!(c.query_row("SELECT status FROM asset_enrichment_items WHERE id=?1",[item.id],|r|r.get::<_,String>(0)).unwrap(),"failed");
    assert!(claim(&mut c,9001,&id).unwrap().is_none());
    assert!(finish_single_pass(&c,&id,false).unwrap());
    assert!(c.query_row("SELECT reason FROM asset_enrichment_runs WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap().contains("1 项"));
    c.execute("UPDATE asset_enrichment_runs SET enabled=1",[]).unwrap();
    enqueue(&c,&id,"tls","other.example.test","https://other.example.test/").unwrap();
    let item=claim(&mut c,9001,&id).unwrap().unwrap();
    finish(&mut c,9001,&id,&item,Err("non_public_address".into())).unwrap();
    assert_eq!(c.query_row("SELECT status FROM asset_enrichment_items WHERE id=?1",[item.id],|r|r.get::<_,String>(0)).unwrap(),"failed");
    drop(c);std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn one_pass_does_not_refresh_successful_items_while_waiting_for_a_different_retry() {
    let(dir,mut c,id)=fixture();
    enqueue(&c,&id,"ct","example.test","example.test").unwrap();
    c.execute("UPDATE asset_enrichment_items SET status='done',next_at=0",[]).unwrap();
    assert!(claim(&mut c,9001,&id).unwrap().is_none());
    c.execute("UPDATE asset_enrichment_runs SET continuous=1",[]).unwrap();
    assert!(claim(&mut c,9001,&id).unwrap().is_some());
    drop(c);std::fs::remove_dir_all(dir).unwrap();
}
