//! Bounded Agent projections over the same project-owned exposure ledger.
//! Read paths do not migrate, fetch, classify or change review decisions.
use super::{contract::ExposureFindingPageInput, redaction, repository};
use crate::{
    asset_models::{ExposureFinding, ExposureRun},
    db,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    operation_id: Option<String>,
    run_id: Option<i64>,
    finding_id: Option<i64>,
    limit: Option<i64>,
    cursor: String,
    status: String,
    search: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    project: i64,
    kind: String,
    filter: String,
    after: String,
}
fn page_cursor(p: i64, kind: &str, filter: &str, raw: &str) -> Result<String, String> {
    if raw.is_empty() {
        return Ok(String::new());
    }
    let value: Cursor = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| "exposure_agent_cursor_invalid")?,
    )
    .map_err(|_| "exposure_agent_cursor_invalid")?;
    if value.project != p || value.kind != kind || value.filter != filter || value.after.is_empty()
    {
        return Err("exposure_agent_cursor_scope_changed".into());
    }
    Ok(value.after)
}
fn next_cursor(p: i64, kind: &str, filter: &str, after: &str) -> String {
    if after.is_empty() {
        return String::new();
    }
    URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Cursor {
            project: p,
            kind: kind.into(),
            filter: filter.into(),
            after: after.into(),
        })
        .expect("cursor is serializable"),
    )
}
fn text(v: &str, max: usize) -> String {
    redaction::redact_text(&redaction::redact_url(v))
        .chars()
        .take(max)
        .collect()
}
fn origin(raw: &str) -> String {
    reqwest::Url::parse(raw)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default()
}
fn finding(c: &Connection, p: i64, v: ExposureFinding) -> Result<Value, String> {
    let asset = if let Some(id) = v.asset_id {
        c.query_row("SELECT a.link,a.host FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE a.id=?1 AND pa.project_id=?2",params![id,p],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(|e|e.to_string())?
    } else {
        None
    };
    let asset_origin = asset.map(|(link, host)| {
        let primary = origin(&link);
        if primary.is_empty() {
            origin(&format!("https://{host}"))
        } else {
            primary
        }
    });
    let disposition=super::classify::dispositions_for(c,p,&[v.id])?.remove(&v.id).map(|d|json!({"disposition":d.disposition,"infoCategory":d.info_category,"sourceChannel":d.source_channel,"conflict":d.conflict,"classificationId":d.classification_id}));
    let category = match v.category.as_str() {
        "source_code" | "document" | "credential" | "storage" | "repository" | "metadata"
        | "intel_claim" => v.category.as_str(),
        _ => "other",
    };
    // Free-form titles can themselves contain a credential. The model gets a
    // category label and the saved classification, not a best-effort copy.
    let label = match category {
        "source_code" => "源代码线索",
        "document" => "文档线索",
        "credential" => "凭据线索",
        "storage" => "存储线索",
        "repository" => "仓库线索",
        "metadata" => "元数据线索",
        "intel_claim" => "情报声明",
        _ => "暴露线索",
    };
    Ok(
        json!({"id":v.id,"assetId":v.asset_id,"assetIdNamespace":"desktop_asset","assetOrigin":asset_origin,"category":category,"title":label,"savedTitleOmitted":true,"sourceType":text(&v.source_type,80),"sourceOrigin":origin(&v.source_url),"severity":v.severity,"confidence":v.confidence,"status":v.status,"classification":disposition,"eligibleNow":v.exposure_eligible,"firstSeenAt":v.first_seen_at,"lastSeenAt":v.last_seen_at,"ownershipConfirmedByThisResult":false,"reviewDecisionChanged":false}),
    )
}
fn collection_evidence(c: &Connection, project: i64, run_id: i64) -> Result<Value, String> {
    let (saved, responses, restricted, missing, transport, truncated): (i64,i64,i64,i64,i64,i64) = c.query_row(
        "SELECT COUNT(*),COALESCE(SUM(status_code>0),0),COALESCE(SUM(status_code IN (401,403)),0),COALESCE(SUM(status_code=404),0),COALESCE(SUM(kind=?3),0),COALESCE(SUM(truncated=1),0)
         FROM exposure_observations WHERE project_id=?1 AND run_id=?2",
        params![project,run_id,super::rules::KIND_TRANSPORT], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))
    ).map_err(|error| error.to_string())?;
    // The fixed projection carries trust metadata, never the diagnostic text or page body.
    // Old/malformed evidence is not upgraded into a certificate exception by inference.
    let (exceptions, expired, future, mismatch, issuer, invalid): (i64,i64,i64,i64,i64,i64) = c.query_row(
        "WITH evidence AS (
           SELECT CASE WHEN json_valid(evidence_json) THEN evidence_json ELSE '{}' END AS value
           FROM exposure_observations WHERE project_id=?1 AND run_id=?2
         ), exceptions AS (
           SELECT json_extract(value,'$.detail.tlsValidation.reasonCode') AS reason
           FROM evidence WHERE json_extract(value,'$.detail.tlsValidation.status')='certificate_exception'
             AND json_extract(value,'$.detail.tlsValidation.validationBypassed')=1
             AND json_extract(value,'$.detail.tlsValidation.scope')='authorized_public_target'
         ) SELECT COUNT(*),COALESCE(SUM(reason='expired'),0),COALESCE(SUM(reason='not_yet_valid'),0),
             COALESCE(SUM(reason='hostname_mismatch'),0),COALESCE(SUM(reason='untrusted_issuer'),0),
             COALESCE(SUM(reason='certificate_invalid'),0) FROM exceptions",
        params![project,run_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))
    ).map_err(|error| error.to_string())?;
    Ok(json!({"savedObservations":saved,"httpResponseObservations":responses,"restrictedResponseObservations":restricted,"notFoundResponseObservations":missing,"transportFailureObservations":transport,"truncatedObservations":truncated,
        "certificateExceptionObservations":exceptions,"certificateExceptionReasonCounts":{"expired":expired,"not_yet_valid":future,"hostname_mismatch":mismatch,"untrusted_issuer":issuer,"certificate_invalid":invalid},
        "tlsAnomalyDoesNotConfirmLeak":true,"certificateExceptionContentIsUnverified":true,
        "basis":"project_run_observation_ledger","bodyOmitted":true,"absenceDoesNotProveSafe":true,"httpSuccessDoesNotConfirmLeak":true}))
}
fn run(c: &Connection, v: ExposureRun) -> Result<Value, String> {
    let recovery = super::recovery::resume_view(c, v.id, v.project_id)?;
    Ok(
        json!({"collectionEvidence":collection_evidence(c,v.project_id,v.id)?,"resumable":recovery.resumable,"recovery":recovery,"id":v.id,"status":v.status,"eligibleAssets":v.eligible_assets,"scannedAssets":v.scanned_assets,"fetchedResources":v.fetched_resources,"findings":v.findings,"stage":text(&v.stage,120),"currentSource":text(&v.current_source,120),"hasError":!v.error.is_empty(),"error":if v.error.is_empty(){""}else{"存在采集错误；详细原因保留在本项目采集任务和数据源日志"},"createdAt":v.created_at,"startedAt":v.started_at,"completedAt":v.completed_at,"acceptedIsNotCompleted":true}),
    )
}
fn read(path: &Path, p: i64) -> Result<Connection, String> {
    if p <= 0 {
        return Err("exposure_agent_project_invalid".into());
    }
    let c = db::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    c.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    c.pragma_update(None, "query_only", true)
        .map_err(|e| e.to_string())?;
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [p],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("exposure_agent_project_unavailable".into());
    }
    Ok(c)
}
fn run_at(c: &Connection, p: i64, id: i64) -> Result<Value, String> {
    let value=c.query_row("SELECT id,project_id,status,eligible_assets,scanned_assets,fetched_resources,findings,stage,current_source,error,created_at,started_at,completed_at FROM exposure_runs WHERE project_id=?1 AND id=?2",params![p,id],super::exposure_run_from_row).optional().map_err(|e|e.to_string())?.ok_or("exposure_agent_run_unavailable")?;
    run(c, value)
}

pub(crate) fn dispatch_at(
    path: &Path,
    p: i64,
    action: &str,
    args: Value,
    operation: &str,
) -> Result<Value, String> {
    let a: Args =
        serde_json::from_value(args.clone()).map_err(|_| "exposure_agent_arguments_invalid")?;
    if [a.run_id, a.finding_id]
        .into_iter()
        .flatten()
        .any(|id| !(1..=9_007_199_254_740_991).contains(&id))
    {
        return Err("exposure_agent_arguments_invalid".into());
    }
    if a.operation_id.is_some() && action != "status" {
        return Err("exposure_agent_arguments_invalid".into());
    }
    let c = read(path, p)?;
    let read_action = matches!(action, "findings" | "finding" | "runs" | "status");
    let result = match action {
        "findings" => {
            if a.run_id.is_some() || a.finding_id.is_some() {
                return Err("exposure_agent_arguments_invalid".into());
            }
            let filter = json!([a.status, a.search]).to_string();
            let cursor = page_cursor(p, "findings", &filter, &a.cursor)?;
            let q = ExposureFindingPageInput {
                project_id: p,
                status: a.status,
                search: a.search,
                page_size: a.limit.unwrap_or(20),
                cursor,
                disposition: String::new(),
            }
            .normalize()
            .map_err(|e| e.message)?;
            if q.page_size > 20 {
                return Err("exposure_agent_arguments_invalid".into());
            }
            let v = repository::page_exposure_findings(&c, &q).map_err(|e| e.message)?;
            json!({"items":v.items.into_iter().map(|v|finding(&c,p,v)).collect::<Result<Vec<_>,_>>()?,"total":v.total,"hasMore":v.has_more,"nextCursor":next_cursor(p,"findings",&filter,&v.next_cursor),"pageSize":v.page_size,"evidenceOmitted":true})
        }
        "finding" => {
            let id = a
                .finding_id
                .filter(|n| *n > 0)
                .ok_or("exposure_agent_arguments_invalid")?;
            if a.run_id.is_some()
                || a.limit.is_some()
                || !a.cursor.is_empty()
                || !a.status.is_empty()
                || !a.search.is_empty()
            {
                return Err("exposure_agent_arguments_invalid".into());
            }
            finding(
                &c,
                p,
                repository::load_exposure_finding(&c, id, p).map_err(|e| e.message)?,
            )?
        }
        "runs" => {
            if a.finding_id.is_some()
                || a.run_id.is_some()
                || !a.status.is_empty()
                || !a.search.is_empty()
            {
                return Err("exposure_agent_arguments_invalid".into());
            }
            let limit = a.limit.unwrap_or(20);
            if !(10..=20).contains(&limit) {
                return Err("exposure_agent_arguments_invalid".into());
            }
            let v = repository::page_exposure_runs(
                &c,
                p,
                limit,
                &page_cursor(p, "runs", "", &a.cursor)?,
            )
            .map_err(|e| e.message)?;
            json!({"items":v.items.into_iter().map(|v|run(&c,v)).collect::<Result<Vec<_>,_>>()?,"total":v.total,"hasMore":v.has_more,"nextCursor":next_cursor(p,"runs","",&v.next_cursor)})
        }
        "status" => {
            if a.finding_id.is_some()
                || a.limit.is_some()
                || !a.cursor.is_empty()
                || !a.status.is_empty()
                || !a.search.is_empty()
            {
                return Err("exposure_agent_arguments_invalid".into());
            }
            if let Some(op) = a.operation_id {
                if a.run_id.is_some() || uuid::Uuid::parse_str(&op).is_err() {
                    return Err("exposure_agent_arguments_invalid".into());
                }
                let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='exposure_agent_operations')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
                let saved: Option<(String, Option<String>)> = if exists {
                    c.query_row("SELECT action,result_json FROM exposure_agent_operations WHERE operation_id=?1 AND project_id=?2",params![op,p],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?
                } else {
                    None
                };
                let (action, receipt) = saved
                    .map(|(kind, r)| (Some(kind), r))
                    .unwrap_or((None, None));
                let receipt: Option<Value> = receipt
                    .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
                    .transpose()?;
                let current = receipt
                    .as_ref()
                    .and_then(|v| v["runId"].as_i64())
                    .map(|id| run_at(&c, p, id))
                    .transpose()?;
                json!({"operationId":op,"action":action,"receipt":receipt,"run":current,"outcomeUnknown":current.is_none(),"missingReceiptDoesNotProveNotStarted":true})
            } else if let Some(id) = a.run_id {
                run_at(&c, p, id)?
            } else {
                let v = repository::page_exposure_runs(&c, p, 10, "").map_err(|e| e.message)?;
                json!({"latest":v.items.into_iter().next().map(|v|run(&c,v)).transpose()?,"totalRuns":v.total})
            }
        }
        "start" | "resume" | "cancel" => {
            if a.finding_id.is_some()
                || a.limit.is_some()
                || !a.cursor.is_empty()
                || !a.status.is_empty()
                || !a.search.is_empty()
                || action == "start" && a.run_id.is_some()
            {
                return Err("exposure_agent_arguments_invalid".into());
            }
            if action != "start" {
                run_at(
                    &c,
                    p,
                    a.run_id
                        .filter(|n| *n > 0)
                        .ok_or("exposure_agent_arguments_invalid")?,
                )?;
            }
            let active: String = c
                .query_row("SELECT status FROM projects WHERE id=?1", [p], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if active != "active" {
                return Err("exposure_agent_project_archived".into());
            }
            drop(c);
            operation_once(path, p, action, &args, operation, || {
                let id = match action {
                    "start" => super::start_exposure_scan_at(path, p)?,
                    "resume" => super::resume_exposure_scan_at(path, a.run_id.unwrap(), p)?,
                    _ => {
                        super::cancel_exposure_scan_at(path, p, a.run_id.unwrap())?;
                        a.run_id.unwrap()
                    }
                };
                Ok(
                    json!({"runId":id,"operationId":operation,"action":action,"accepted":true,"completed":false,"nextTool":"exposure_task_status"}),
                )
            })?
        }
        _ => return Err("exposure_agent_action_invalid".into()),
    };
    let mut result = result;
    result["projectId"] = json!(p);
    result["readOnly"] = json!(read_action);
    result["source"] = json!("project_exposure_ledger");
    if serde_json::to_vec(&result)
        .map_err(|e| e.to_string())?
        .len()
        > 64 * 1024
    {
        return Err("exposure_agent_result_too_large".into());
    }
    Ok(result)
}
fn operation_once<F: FnOnce() -> Result<Value, String>>(
    path: &Path,
    p: i64,
    action: &str,
    args: &Value,
    op: &str,
    execute: F,
) -> Result<Value, String> {
    if uuid::Uuid::parse_str(op).is_err() {
        return Err("exposure_agent_operation_invalid".into());
    }
    let mut c = db::open(path)?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS exposure_agent_operations(operation_id TEXT PRIMARY KEY,project_id INTEGER NOT NULL,action TEXT NOT NULL,args_json TEXT NOT NULL,result_json TEXT)").map_err(|e|e.to_string())?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let old:Option<(i64,String,String,Option<String>)>=tx.query_row("SELECT project_id,action,args_json,result_json FROM exposure_agent_operations WHERE operation_id=?1",[op],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
    if let Some((project, kind, saved, result)) = old {
        if project != p || kind != action || saved != args.to_string() {
            return Err("exposure_agent_operation_conflict".into());
        }
        return result
            .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
            .unwrap_or_else(|| Err("exposure_agent_operation_outcome_unknown".into()));
    }
    tx.execute(
        "INSERT INTO exposure_agent_operations VALUES(?1,?2,?3,?4,NULL)",
        params![op, p, action, args.to_string()],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    let value = execute()?;
    c.execute(
        "UPDATE exposure_agent_operations SET result_json=?1 WHERE operation_id=?2",
        params![value.to_string(), op],
    )
    .map_err(|e| e.to_string())?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: std::path::PathBuf,
        path: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("ovi-exposure-agent-{}", uuid::Uuid::new_v4()));
            let path = db::initialize(&root).unwrap();
            let c = db::open(&path).unwrap();
            c.execute_batch("INSERT INTO projects(id,name,status) VALUES(901,'temporary','active'),(902,'foreign','active')").unwrap();
            super::super::state::ensure_exposure_schema(&c).unwrap();
            for i in 0..25 {
                c.execute("INSERT INTO exposure_findings(project_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,status,fingerprint) VALUES(?1,'credential','token=TOP-SECRET-CREDENTIAL','passive_web','https://example.test/private/TOP-SECRET-CREDENTIAL?token=TOP-SECRET-CREDENTIAL','TOP-SECRET-CREDENTIAL','high',80,'new',?2)",params![if i==24{902}else{901},format!("fp-{i}")]).unwrap();
            }
            Self { root, path }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn call(f: &Fixture, action: &str, args: Value) -> Result<Value, String> {
        dispatch_at(
            &f.path,
            901,
            action,
            args,
            &uuid::Uuid::new_v4().to_string(),
        )
    }
    #[test]
    fn read_pages_are_project_scoped_redacted_complete_and_do_not_create_runs() {
        let f = Fixture::new();
        let c = db::open(&f.path).unwrap();
        let before: i64 = c
            .query_row("SELECT COUNT(*) FROM exposure_runs", [], |r| r.get(0))
            .unwrap();
        let mut ids = std::collections::HashSet::new();
        let mut cursor = String::new();
        loop {
            let v = call(&f, "findings", json!({"limit":10,"cursor":cursor})).unwrap();
            assert_eq!(v["total"], 24);
            assert!(v["readOnly"].as_bool().unwrap());
            let raw = v.to_string();
            assert!(!raw.contains("TOP-SECRET-CREDENTIAL"));
            assert!(!raw.contains("evidenceExcerpt"));
            assert!(!raw.contains("/private/"));
            for x in v["items"].as_array().unwrap() {
                assert!(ids.insert(x["id"].as_i64().unwrap()));
                assert_eq!(x["sourceOrigin"], "https://example.test");
            }
            if v["hasMore"] == false {
                break;
            }
            cursor = v["nextCursor"].as_str().unwrap().into();
        }
        assert_eq!(ids.len(), 24);
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM exposure_runs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            before
        );
        let tables: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='exposure_agent_operations'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
        let foreign: i64 = c
            .query_row(
                "SELECT id FROM exposure_findings WHERE project_id=902",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(call(&f, "finding", json!({"findingId":foreign})).is_err());
    }
    #[test]
    fn cursors_bind_project_kind_and_filter_and_unknown_receipts_are_not_fabricated() {
        let f = Fixture::new();
        let first = call(&f, "findings", json!({"limit":10})).unwrap();
        let cursor = first["nextCursor"].as_str().unwrap();
        assert!(dispatch_at(
            &f.path,
            902,
            "findings",
            json!({"limit":10,"cursor":cursor}),
            &uuid::Uuid::new_v4().to_string()
        )
        .unwrap_err()
        .contains("cursor_scope_changed"));
        assert!(call(
            &f,
            "findings",
            json!({"limit":10,"cursor":cursor,"status":"confirmed"})
        )
        .is_err());
        assert!(call(&f, "runs", json!({"limit":10,"cursor":cursor})).is_err());
        let missing = call(
            &f,
            "status",
            json!({"operationId":uuid::Uuid::new_v4().to_string()}),
        )
        .unwrap();
        assert_eq!(missing["outcomeUnknown"], true);
        assert_eq!(missing["receipt"], Value::Null);
        assert!(call(
            &f,
            "status",
            json!({"operationId":uuid::Uuid::new_v4().to_string(),"runId":1})
        )
        .is_err());
    }
    #[test]
    fn recovery_hint_is_read_only_and_never_reclaims_a_freshly_admitted_run() {
        let f = Fixture::new();
        let c = db::open(&f.path).unwrap();
        c.execute("INSERT INTO exposure_runs(project_id,status,created_at,started_at) VALUES(901,'running',datetime('now'),datetime('now'))",[]).unwrap();
        let id = c.last_insert_rowid();
        assert!(!super::super::recovery::stale_without_live_claims(&c, id, 90).unwrap());
        c.execute("UPDATE exposure_runs SET created_at=datetime('now','-10 minutes'),started_at=datetime('now','-10 minutes') WHERE id=?1",[id]).unwrap();
        assert!(super::super::recovery::stale_without_live_claims(&c, id, 90).unwrap());
        assert_eq!(
            c.query_row("SELECT status FROM exposure_runs WHERE id=?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "running"
        );
    }
    #[test]
    fn invalid_parameters_and_cross_project_runs_are_refused() {
        let f = Fixture::new();
        for args in [
            json!({"limit":500}),
            json!({"projectId":902}),
            json!({"limit":0}),
            json!({"cursor":"bad"}),
        ] {
            assert!(call(&f, "findings", args).is_err());
        }
        assert!(call(&f, "status", json!({"runId":i64::MAX})).is_err());
        assert!(call(&f, "resume", json!({"runId":1234567})).is_err());
        assert!(call(&f, "delete", json!({})).is_err());
    }
    #[test]
    fn durable_admission_reuses_receipts_but_never_replays_unknown_operations() {
        let f = Fixture::new();
        let op = uuid::Uuid::new_v4().to_string();
        let a = json!({});
        let v =
            operation_once(&f.path, 901, "start", &a, &op, || Ok(json!({"runId":123}))).unwrap();
        assert_eq!(
            operation_once(&f.path, 901, "start", &a, &op, || panic!(
                "must not execute again"
            ))
            .unwrap(),
            v
        );
        assert_eq!(
            operation_once(&f.path, 902, "start", &a, &op, || panic!("foreign")).unwrap_err(),
            "exposure_agent_operation_conflict"
        );
        let unknown = uuid::Uuid::new_v4().to_string();
        assert!(operation_once(&f.path, 901, "start", &a, &unknown, || Err(
            "lost after admission".into()
        ))
        .is_err());
        assert_eq!(
            operation_once(&f.path, 901, "start", &a, &unknown, || panic!(
                "unknown cannot replay"
            ))
            .unwrap_err(),
            "exposure_agent_operation_outcome_unknown"
        );
    }
    #[test]
    fn run_status_returns_saved_response_facts_without_exposing_bodies_or_other_projects() {
        let f = Fixture::new();
        let c = db::open(&f.path).unwrap();
        c.execute_batch("INSERT INTO exposure_runs(id,project_id,status) VALUES(9011,901,'completed'),(9021,902,'completed')").unwrap();
        for (project,run_id,status,url) in [(901,9011,403,"https://example.test/private/secret-one"),(901,9011,404,"https://example.test/private/secret-two"),(902,9021,200,"https://foreign.test/secret-three")] {
            let mut observation = super::super::observations::NewObservation::new("passive_web","resource",url);
            observation.status_code=status;
            observation.body=b"TOP-SECRET-CREDENTIAL".to_vec();
            super::super::observations::record(&c,run_id,project,None,&observation).unwrap();
        }
        let result=call(&f,"status",json!({"runId":9011})).unwrap();
        assert_eq!(result["collectionEvidence"]["savedObservations"],2);
        assert_eq!(result["collectionEvidence"]["restrictedResponseObservations"],1);
        assert_eq!(result["collectionEvidence"]["notFoundResponseObservations"],1);
        assert_eq!(result["collectionEvidence"]["httpSuccessDoesNotConfirmLeak"],true);
        let raw=result.to_string();
        assert!(!raw.contains("secret-one") && !raw.contains("TOP-SECRET-CREDENTIAL") && !raw.contains("foreign.test"));
        assert!(call(&f,"status",json!({"runId":9021})).is_err());
        assert_eq!(c.query_row("SELECT COUNT(*) FROM exposure_observations",[],|r|r.get::<_,i64>(0)).unwrap(),3);
    }

    #[test]
    fn certificate_exception_counts_are_project_scoped_bounded_and_not_a_leak_verdict() {
        let f=Fixture::new();let c=db::open(&f.path).unwrap();
        c.execute_batch("INSERT INTO exposure_runs(id,project_id,status) VALUES(9011,901,'completed'),(9021,902,'completed')").unwrap();
        for (index,(project,run_id,reason,bypassed)) in [(901,9011,"expired",true),(901,9011,"untrusted_issuer",true),(901,9011,"hostname_mismatch",false),(902,9021,"expired",true)].iter().enumerate() {
            let mut row=super::super::observations::NewObservation::new("passive_web","resource",&format!("https://example.test/private-{index}"));
            row.status_code=200;
            row.extra=json!({"tlsValidation":{"status":"certificate_exception","validationBypassed":bypassed,"reasonCode":reason,"scope":"authorized_public_target","diagnostic":"private diagnostic should not reach model"}});
            super::super::observations::record(&c,*run_id,*project,None,&row).unwrap();
        }
        // The indexed normalized-text expression already rejects malformed JSON writes.
        // Legacy valid JSON with an unexpected shape must still be readable as no exception.
        let malformed=super::super::observations::NewObservation::new("passive_web","resource","https://example.test/malformed");
        let(id,_)=super::super::observations::record(&c,9011,901,None,&malformed).unwrap();
        assert!(c.execute("UPDATE exposure_observations SET evidence_json='invalid legacy JSON' WHERE id=?1",[id]).is_err());
        c.execute("UPDATE exposure_observations SET evidence_json='\"legacy metadata without typed detail\"' WHERE id=?1",[id]).unwrap();
        let result=call(&f,"status",json!({"runId":9011})).unwrap();
        let facts=&result["collectionEvidence"];
        assert_eq!(facts["certificateExceptionObservations"],2);
        assert_eq!(facts["certificateExceptionReasonCounts"],json!({"expired":1,"not_yet_valid":0,"hostname_mismatch":0,"untrusted_issuer":1,"certificate_invalid":0}));
        assert_eq!(facts["tlsAnomalyDoesNotConfirmLeak"],true);
        assert_eq!(facts["certificateExceptionContentIsUnverified"],true);
        assert_eq!(result["findings"],0);
        assert!(!result.to_string().contains("private diagnostic"));
        assert!(!result.to_string().contains("private-"));
    }

    #[test]
    fn historical_findings_report_current_revocation_and_project_membership() {
        let f=Fixture::new();let c=db::open(&f.path).unwrap();
        super::super::fixture::seed_scope(&c,901,"https://owned.example.test/",true);
        let asset:i64=c.query_row("SELECT asset_id FROM project_assets WHERE project_id=901 LIMIT 1",[],|r|r.get(0)).unwrap();
        let id:i64=c.query_row("SELECT id FROM exposure_findings WHERE project_id=901 LIMIT 1",[],|r|r.get(0)).unwrap();
        c.execute("UPDATE exposure_findings SET asset_id=?2 WHERE id=?1",params![id,asset]).unwrap();
        assert_eq!(call(&f,"finding",json!({"findingId":id})).unwrap()["eligibleNow"],true);
        c.execute("UPDATE asset_ownership_decisions SET authorization_status='revoked' WHERE project_id=901",[]).unwrap();
        assert_eq!(call(&f,"finding",json!({"findingId":id})).unwrap()["eligibleNow"],false);
        c.execute("UPDATE asset_ownership_decisions SET authorization_status='allowed' WHERE project_id=901",[]).unwrap();
        c.execute("UPDATE project_assets SET is_deleted=1 WHERE project_id=901",[]).unwrap();
        assert_eq!(call(&f,"finding",json!({"findingId":id})).unwrap()["eligibleNow"],false);
        c.execute("UPDATE project_assets SET is_deleted=0 WHERE project_id=901",[]).unwrap();
        c.execute("UPDATE projects SET status='archived' WHERE id=901",[]).unwrap();
        assert_eq!(call(&f,"finding",json!({"findingId":id})).unwrap()["eligibleNow"],false);
    }

}
