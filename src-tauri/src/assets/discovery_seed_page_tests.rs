use super::*;
#[test]
fn archived_local_seed_index_is_readable_and_missing_project_cannot_be_prepared() {
 let dir=std::env::temp_dir().join(format!("ovi-archived-seed-history-{}",uuid::Uuid::new_v4()));
 std::fs::create_dir(&dir).unwrap();
 let path=db::initialize(&dir).unwrap();let c=db::open(&path).unwrap();
 c.execute("INSERT OR IGNORE INTO projects(id,name,status) VALUES(991,'Archived historical project','archived')",[]).unwrap();
 crate::asset_discovery::ensure(&c).unwrap();drop(c);
 let prepared=prepare_on(&path,991).unwrap();assert_eq!(prepared["ready"],true);
 let c=db::open(&path).unwrap();let read=read_on(&c,&serde_json::from_value(json!({"projectId":991})).unwrap()).unwrap();
 assert_eq!(read["ready"],true);assert_eq!(read["roundsTotal"],0);
 assert!(project(&c,991).is_err(),"archived project remains ineligible for discovery execution");
 assert!(prepare_on(&path,992).is_err());
 assert!(read_on(&c,&serde_json::from_value(json!({"projectId":992})).unwrap()).is_err());
 assert!(!c.query_row("SELECT EXISTS(SELECT 1 FROM asset_project_seed_state WHERE project_id=992)",[],|r|r.get::<_,bool>(0)).unwrap());
 drop(c);std::fs::remove_dir_all(dir).unwrap();
}
#[test]
#[ignore="explicit consistent real-project snapshot; writes only a disposable copy, never the original"]
fn real_snapshot_seed_history_preserves_source_data_and_unknown_first_round(){
 let source=std::env::var_os("OVIRAPTOR_SEED_LEDGER_SNAPSHOT").expect("explicit snapshot required");
 let project:i64=std::env::var("OVIRAPTOR_SEED_LEDGER_PROJECT").expect("project required").parse().unwrap();
 let dir=std::env::temp_dir().join(format!("ovi-seed-snapshot-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();
 let path=dir.join("validation.sqlite3");std::fs::copy(source,&path).unwrap();let c=db::open(&path).unwrap();
 let before:(i64,i64,String)=c.query_row("SELECT (SELECT COUNT(*) FROM assets),(SELECT COUNT(*) FROM asset_seed_review_model_receipts),(SELECT roots_json FROM asset_discovery_scopes WHERE project_id=?1)",[project],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();drop(c);
 let started=std::time::Instant::now();let page=prepare_on(&path,project).unwrap();assert_eq!(page["ready"],true);let c=db::open(&path).unwrap();
 let after:(i64,i64,String)=c.query_row("SELECT (SELECT COUNT(*) FROM assets),(SELECT COUNT(*) FROM asset_seed_review_model_receipts),(SELECT roots_json FROM asset_discovery_scopes WHERE project_id=?1)",[project],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();assert_eq!(before,after);
 let nonhistorical:i64=c.query_row("SELECT COUNT(*) FROM asset_project_seeds WHERE project_id=?1 AND (first_round IS NOT NULL OR first_seen_at IS NOT NULL OR historical_baseline!=1)",[project],|r|r.get(0)).unwrap();assert_eq!(nonhistorical,0);
 let total=page["summary"]["total"].as_i64().unwrap();assert!(total>0);let id=page["items"][0]["id"].as_i64().unwrap();let detail=seed_ledger::detail(&c,project,id,1,20).unwrap();assert_eq!(detail["seed"]["firstSeenRound"],Value::Null);
 assert!(seed_ledger::detail(&c,project+100000,id,1,20).is_err());let indexed_ms=started.elapsed().as_millis();
 let read_started=std::time::Instant::now();let read=read_on(&c,&serde_json::from_value(json!({"projectId":project})).unwrap()).unwrap();
 eprintln!("REAL_SEED_SNAPSHOT {}",json!({"seedCount":total,"indexedMs":indexed_ms,"readMs":read_started.elapsed().as_millis(),"summary":read["summary"],"originalDataPreserved":true,"firstHistoricalRoundUnknown":true}));drop(c);let _=std::fs::remove_dir_all(dir);
}

#[test]
#[ignore="explicit real project snapshot; local reconciliation benchmark on disposable copy only"]
fn real_snapshot_unified_seed_decisions_and_exact_cleanup(){
 let source=std::env::var_os("OVIRAPTOR_SEED_LEDGER_SNAPSHOT").expect("snapshot");let p:i64=std::env::var("OVIRAPTOR_SEED_LEDGER_PROJECT").expect("project").parse().unwrap();
 let dir=std::env::temp_dir().join(format!("ovi-seed114-real-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();let path=dir.join("snapshot.sqlite3");std::fs::copy(source,&path).unwrap();let mut c=db::open(&path).unwrap();
 let counts=|c:&Connection|c.query_row("SELECT (SELECT count(*) FROM assets),(SELECT count(*) FROM asset_discovery_seeds),(SELECT count(*) FROM asset_cleanup_models),(SELECT generation FROM asset_cleanup_runs WHERE project_id=?1 ORDER BY created_at DESC LIMIT 1)",[p],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap();let before=counts(&c);
 let started=std::time::Instant::now();let result=seed_policy::reconcile_on(&mut c,p).unwrap();let sync_ms=started.elapsed().as_millis();
 let start=std::time::Instant::now();let q=serde_json::from_value(json!({"projectId":p,"search":"cmsrict.com"})).unwrap();let page=read_on(&c,&q).unwrap();let read_ms=start.elapsed().as_millis();
 assert!(page["items"].as_array().unwrap().iter().all(|s|s["trustStatus"]=="accepted"),"Every cmsrict domain must reuse confirmed scope; rejecting a mixed certificate cannot exclude the domain");assert!(page["items"].as_array().unwrap().iter().any(|s|s["subjects"].as_array().is_some_and(|v|!v.is_empty())));
 let default_started=std::time::Instant::now();let default_page=read_on(&c,&serde_json::from_value(json!({"projectId":p})).unwrap()).unwrap();let default_ms=default_started.elapsed().as_millis();
 let exact=c.prepare("SELECT value FROM asset_seed_exact_rules WHERE project_id=?1 AND enabled=1 ORDER BY value").unwrap().query_map([p],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
 assert!(exact.iter().any(|s|s=="hnvc.net.cn"||s=="imac.edu.cn"),"independently verified host must not wait for inaccessible siblings: {exact:?}");
 assert_eq!(counts(&c),before);assert_eq!(c.query_row("SELECT enabled FROM asset_cleanup_runs WHERE project_id=?1 ORDER BY created_at DESC LIMIT 1",[p],|r|r.get::<_,i64>(0)).unwrap(),0);
 let again=seed_policy::reconcile_on(&mut c,p).unwrap();assert_eq!(again["excluded"],0);assert_eq!(again["skipped"],0);
 eprintln!("REAL_UNIFIED_SEEDS {}",json!({"syncMs":sync_ms,"readCmsrictMs":read_ms,"readDefault50Ms":default_ms,"defaultSummary":default_page["summary"],"result":result,"exactExcluded":exact,"cmsrict":page,"countsUnchanged":true,"networkRequests":0,"modelRequests":0}));
 drop(c);std::fs::remove_dir_all(dir).unwrap();
}
