use super::*;
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
