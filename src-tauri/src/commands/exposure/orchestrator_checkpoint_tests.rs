//! Temporary-database regressions for the real orchestrator commit boundary. No sockets,
//! model calls, App data or VPN changes are used here.
use super::*;
use super::super::fixture::seed_scope;

const PROJECT: i64 = 91703;
struct Fixture { root: std::path::PathBuf, c: Connection, run: i64, scope: ExposureScope }
impl Fixture {
    fn new() -> Self {
        let root=std::env::temp_dir().join(format!("ovi-web-progress-{}",uuid::Uuid::new_v4()));
        let path=crate::db::initialize(&root).unwrap();
        let c=crate::db::open(&path).unwrap();
        ensure_exposure_schema(&c).unwrap();
        seed_scope(&c,PROJECT,"https://127.0.0.1/",true);
        let settings=json!({"exposureSources":["passive_web"],"exposureMaxRequests":200,
            "exposureMaxTotalBytes":10_000_000,"exposureMaxResponseBytes":10_000,
            "exposureAllowSameHostHttpRedirects":true,"exposureAllowPrivateDestinations":true});
        let started=start_or_reuse(&c,PROJECT,&settings).unwrap();
        let frozen=frozen_settings_strict(&c,started.run_id,PROJECT).unwrap().to_settings_json();
        let scope=ExposureScope::load(&c,PROJECT,&frozen).unwrap();
        Self{root,c,run:started.run_id,scope}
    }
    fn cursor(&self,item:&WorkItem,queue:Vec<String>,visited:Vec<String>,in_flight:Option<String>,unknown:Vec<String>) -> JsonValue {
        json!({"version":1,"runId":item.run_id,"projectId":item.project_id,"workKey":item.work_key,
            "inputHash":item.input_hash,"origin":item.origin,"scopeHash":scope_hash_of(&self.scope),
            "queue":queue,"successes":visited.len(),"visited":visited,"inFlight":in_flight,"unknown":unknown,
            "sourceReason":"","coverageReason":"","queueTruncated":false,"deferredDiscoveryCount":0,
            "reservedBytes":0,"unknownReservedBytes":0})
    }
    fn source(&self)->(String,i64,String) {
        self.c.query_row("SELECT status,item_count,error FROM exposure_source_results WHERE run_id=?1 AND source_key='passive_web'",[self.run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.root); } }

fn public_observation(url:&str,final_url:&str,trust:bool)->NewObservation {
    let mut row=NewObservation::new(KIND_PASSIVE_WEB,"resource",url);
    row.final_url=final_url.into();row.status_code=200;row.content_type="text/plain".into();
    row.body=b"synthetic public evidence".to_vec();row.body_bytes=row.body.len() as i64;
    if trust {row.extra=json!({"tlsValidation":{"status":"certificate_exception","validationBypassed":true,"scope":"authorized_public_target","reasonCode":"expired"},
        "httpDowngrade":{"status":"same_host_http","transportEncrypted":false,"scope":"authorized_same_host_public_get"}});}
    row
}

#[test]
fn redirected_finding_binds_real_response_and_preserves_tls_http_trust_and_cache() {
    let f=Fixture::new();
    let original="https://127.0.0.1/redirect";
    let final_url="http://127.0.0.1/public.txt";
    let mut executor=|item:&WorkItem| {
        let row=public_observation(original,final_url,true);
        let draft=FindingDraft{category:"other".into(),title:"synthetic redirected content candidate".into(),
            url:final_url.into(),source_type:"public_web".into(),evidence:"synthetic excerpt".into(),
            severity:"low".into(),confidence:40,asset_id:item.asset_id,content_hash:super::super::redaction::content_hash(&row.body)};
        Ok(ItemOutcome{status:Some(WorkItemStatus::Completed),observations:vec![row],findings:vec![draft],
            source_status:Some(SourceStatus::Completed),observed:1,..Default::default()})
    };
    run_plan(&f.c,f.run,"redirect-test",&mut executor).unwrap();
    let count:i64=f.c.query_row("SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1",[f.run],|r|r.get(0)).unwrap();
    assert_eq!(count,1,"redirect does not fabricate a second observation without transport provenance");
    let (url,evidence):(String,String)=f.c.query_row("SELECT o.url,o.evidence_json FROM exposure_finding_observations l JOIN exposure_observations o ON o.id=l.observation_id WHERE l.run_id=?1",[f.run],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(url,original);
    let saved:JsonValue=serde_json::from_str(&evidence).unwrap();
    assert_eq!(saved["detail"]["tlsValidation"]["status"],"certificate_exception");
    assert_eq!(saved["detail"]["httpDowngrade"]["transportEncrypted"],false);
    let cached:i64=f.c.query_row("SELECT COUNT(*) FROM exposure_analysis_cache WHERE project_id=?1",[PROJECT],|r|r.get(0)).unwrap();
    assert_eq!(cached,1,"redirected real body receives the same classification cache path");
}

#[test]
fn multi_url_source_rebuild_counts_prior_observations_and_pause_is_not_unconfigured() {
    let f=Fixture::new();
    let first="https://127.0.0.1/".to_string();
    let next="https://127.0.0.1/next".to_string();
    let mut initial=|item:&WorkItem| {
        let cursor=f.cursor(item,vec![next.clone()],vec![first.clone()],None,vec![]);
        f.c.execute("UPDATE exposure_runs SET cancel_requested=1 WHERE id=?1",[f.run]).unwrap();
        Ok(ItemOutcome{status:Some(WorkItemStatus::Pending),continuation:true,cursor_json:Some(cursor.to_string()),
            observations:vec![public_observation(&first,&first,true)],observed:1,..Default::default()})
    };
    run_plan(&f.c,f.run,"first-url",&mut initial).unwrap();
    let paused=f.source();
    assert_eq!((&*paused.0,paused.1),("cancelled",1));
    assert!(paused.2.contains("same_host_http"));
    assert!(super::super::recovery::prepare_resume(&f.c,f.run,PROJECT).unwrap());
    let mut tail=|item:&WorkItem| {
        let cursor=f.cursor(item,vec![],vec![first.clone(),next.clone()],None,vec![]);
        Ok(ItemOutcome{status:Some(WorkItemStatus::Completed),cursor_json:Some(cursor.to_string()),
            observations:vec![public_observation(&next,&next,false)],observed:1,source_status:Some(SourceStatus::Completed),..Default::default()})
    };
    run_plan(&f.c,f.run,"tail-url",&mut tail).unwrap();
    let completed=f.source();
    assert_eq!((&*completed.0,completed.1),("completed",2),"a resumed tail cannot erase the first URL or keep the earlier pause as a coverage gap");
    assert!(completed.2.contains("same_host_http"));
    assert!(completed.2.contains("tls_certificate_exception"));
}

#[test]
fn source_rebuild_does_not_green_unknown_truncated_or_transport_failures() {
    for mode in ["unknown","truncated","transport"] {
        let f=Fixture::new();
        let mut executor=|item:&WorkItem| {
            let url="https://127.0.0.1/".to_string();
            let mut cursor=f.cursor(item,vec![],vec![url.clone()],None,if mode=="unknown" {vec!["https://127.0.0.1/unknown".into()]} else {vec![]});
            if mode=="truncated" {cursor["queueTruncated"]=json!(true);cursor["deferredDiscoveryCount"]=json!(2);}
            let mut observations=vec![public_observation(&url,&url,false)];
            if mode=="transport" {observations.push(NewObservation::new(KIND_PASSIVE_WEB,"transport","https://127.0.0.1/failed"));}
            Ok(ItemOutcome{status:Some(WorkItemStatus::Completed),source_status:Some(SourceStatus::Completed),cursor_json:Some(cursor.to_string()),observations,..Default::default()})
        };
        run_plan(&f.c,f.run,"gap-test",&mut executor).unwrap();
        assert_eq!(f.source().0,"partial","{mode}");
    }
}

#[test]
fn pre_send_denial_with_empty_original_cursor_keeps_valid_frontier_and_zero_usage() {
    let f=Fixture::new();
    let mut executor=|item:&WorkItem| {
        let cursor=f.cursor(item,vec!["https://127.0.0.1/".into()],vec![],None,vec![]);
        Ok(ItemOutcome{status:Some(WorkItemStatus::BlockedAuthorization),reason:"scope_revoked:synthetic pre-send denial".into(),
            cursor_json:Some(cursor.to_string()),source_status:Some(SourceStatus::Partial),..Default::default()})
    };
    run_plan(&f.c,f.run,"deny-before-send",&mut executor).unwrap();
    let plan=super::super::recovery::load_plan(&f.c,f.run,PROJECT).unwrap();
    assert_eq!((plan.requests_used,plan.bytes_used),(0,0));
    let item=super::super::work_items::read_run_items(&f.c,f.run,PROJECT).unwrap().remove(0);
    assert_eq!(item.status,WorkItemStatus::BlockedAuthorization);
    assert_eq!(super::super::adapters::passive_web::cursor_pending_count(&super::super::work_items::load_cursor(&f.c,item.id).unwrap(),&item.origin).unwrap(),1);
}

#[test]
fn settled_unknown_reservation_is_retained_once_and_success_refunds_before_actual_body_usage() {
    let current=json!({"inFlight":"https://127.0.0.1/","reservedBytes":100,"unknownReservedBytes":25});
    let next=json!({"inFlight":null,"unknown":["https://127.0.0.1/"],"reservedBytes":0,"unknownReservedBytes":25});
    let (settled,refund)=settle_cursor_reservation(&current.to_string(),&next.to_string()).unwrap();
    assert_eq!(refund,0);
    let value:JsonValue=serde_json::from_str(&settled).unwrap();
    assert_eq!(value["unknownReservedBytes"],125);
    assert_eq!(value["reservedBytes"],0);
    let (already,refund)=settle_cursor_reservation(&current.to_string(),&settled).unwrap();
    assert_eq!(refund,0);assert_eq!(serde_json::from_str::<JsonValue>(&already).unwrap()["unknownReservedBytes"],125);
    let (_,refund)=settle_cursor_reservation(&current.to_string(),&json!({"inFlight":null,"unknown":[],"reservedBytes":0,"unknownReservedBytes":25}).to_string()).unwrap();
    assert_eq!(refund,100);
    assert!(settle_cursor_reservation("", "not-json").is_err());
    assert_eq!(settle_cursor_reservation("", "{}").unwrap().1,0);

    let f=Fixture::new();
    let mut executor=|item:&WorkItem| {
        let url="https://127.0.0.1/".to_string();
        let current=f.cursor(item,vec![],vec![],Some(url.clone()),vec![]);
        assert!(super::super::work_items::save_cursor(&f.c,item,&current.to_string()).unwrap());
        reserve_request(&f.c,item,100).unwrap();
        let mut complete=f.cursor(item,vec![],vec![url.clone()],None,vec![]);
        complete["unknownReservedBytes"]=json!(0);
        Ok(ItemOutcome{status:Some(WorkItemStatus::Completed),cursor_json:Some(complete.to_string()),
            observations:vec![public_observation(&url,&url,false)],bytes:23,requests:0,..Default::default()})
    };
    run_plan(&f.c,f.run,"byte-settle",&mut executor).unwrap();
    let plan=super::super::recovery::load_plan(&f.c,f.run,PROJECT).unwrap();
    assert_eq!((plan.requests_used,plan.bytes_used),(1,23));
    let item=super::super::work_items::read_run_items(&f.c,f.run,PROJECT).unwrap().remove(0);
    assert_eq!((item.requests,item.bytes),(1,23));

    let unknown=Fixture::new();
    let mut executor=|item:&WorkItem| {
        let url="https://127.0.0.1/".to_string();
        let current=unknown.cursor(item,vec![],vec![],Some(url.clone()),vec![]);
        assert!(super::super::work_items::save_cursor(&unknown.c,item,&current.to_string()).unwrap());
        reserve_request(&unknown.c,item,100).unwrap();
        let complete=unknown.cursor(item,vec![],vec![],None,vec![url]);
        Ok(ItemOutcome{status:Some(WorkItemStatus::Completed),cursor_json:Some(complete.to_string()),..Default::default()})
    };
    run_plan(&unknown.c,unknown.run,"unknown-settle",&mut executor).unwrap();
    let plan=super::super::recovery::load_plan(&unknown.c,unknown.run,PROJECT).unwrap();
    assert_eq!((plan.requests_used,plan.bytes_used),(1,100),"unknown response allowance is never silently refunded");
    let item=super::super::work_items::read_run_items(&unknown.c,unknown.run,PROJECT).unwrap().remove(0);
    let cursor=super::super::work_items::load_cursor(&unknown.c,item.id).unwrap();
    assert_eq!(super::super::adapters::passive_web::cursor_progress_counts(&cursor,&item.origin).unwrap(),(1,0,100));
    assert_eq!(unknown.source().0,"partial");
}

#[test]
fn clean_peer_does_not_clear_previously_seen_http_or_tls_warnings() {
    assert!(merge_source_reason("same_host_http:plaintext","").contains("same_host_http"));
    let both=merge_source_reason("same_host_http:plaintext","tls_certificate_exception:expired");
    assert!(both.contains("same_host_http")&&both.contains("tls_certificate_exception"));
    assert_eq!(merge_source_reason(&both,"tls_certificate_exception:expired"),both);
}
