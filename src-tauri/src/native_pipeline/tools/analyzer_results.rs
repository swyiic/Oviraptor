//! Read-only access to accepted analyzer results for this source assignment.

use super::repository::MAX_RESULTS;
use super::{BrokerDenial, SourceBroker};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};

impl SourceBroker {
    pub(super) fn list_results(
        &self,
        connection: &Connection,
        map: &serde_json::Map<String, JsonValue>,
    ) -> Result<JsonValue, BrokerDenial> {
        if let Some(view) = &self.analysis_view {
            let results = super::super::results::AnalysisResults::load(connection, view)
                .map_err(|e| BrokerDenial::new("source_results_unavailable", e))?;
            let engine = map.get("engine").and_then(JsonValue::as_str).unwrap_or("");
            let limit = Self::limit_of(map, MAX_RESULTS, MAX_RESULTS);
            let records = results
                .candidates(connection)
                .map_err(|e| BrokerDenial::new("source_results_unavailable", e))?;
            let matching: Vec<_> = records
                .iter()
                .filter(|candidate| {
                    engine.is_empty() || candidate.sources.iter().any(|r| r.engine == engine)
                })
                .collect();
            let candidates:Vec<_>=matching.iter().take(limit).map(|candidate| {
                let r=candidate.sources.iter().copied().find(|r|engine.is_empty() || r.engine==engine)
                    .expect("matching candidates have a selected origin");
                let v=&candidate.envelope;
                json!({
                "key":r.record.logical_key,"revisionHash":r.record.revision_hash,"engine":r.engine,
                "acceptedBundleId":r.bundle_id,"acceptedArtifactSha256":r.record.source_artifact_id,
                "acceptedSources":candidate.sources,
                "title":v.pointer("/payload/title").and_then(JsonValue::as_str).unwrap_or_default(),
                "ruleId":v.pointer("/payload/rule_id").and_then(JsonValue::as_str).unwrap_or_default(),
                "severity":v.pointer("/payload/severity").and_then(JsonValue::as_str).unwrap_or_default(),
                "endpoint":v.pointer("/payload/path").and_then(JsonValue::as_str).unwrap_or_default(),
                "reviewState":"candidate"
            })}).collect();
            return Ok(
                json!({"runs":results.runs.iter().filter(|a|engine.is_empty()||a["engine"].as_str()==Some(engine)).collect::<Vec<_>>(),
                "candidates":candidates,"scanId":self.scan_id,"attemptNumber":self.attempt_number,
                "analysisManifestDigest":results.analysis_digest,"analysisResultsDigest":results.digest(),
                "gaps":results.gaps,"truncated":matching.len()>limit}),
            );
        }
        let engine = map.get("engine").and_then(JsonValue::as_str).unwrap_or("");
        let limit = Self::limit_of(map, MAX_RESULTS, MAX_RESULTS);
        let mut runs = Vec::new();
        {
            let mut statement = connection
                .prepare(
                    "SELECT engine,status,gap_code,version,rule_pack_digest,evidence_json
                     FROM analyzer_runs WHERE scan_id=?1 AND attempt_number=?2
                     ORDER BY engine",
                )
                .map_err(|error| {
                    BrokerDenial::new("internal_error", format!("无法读取分析器记录：{error}"))
                })?;
            let rows = statement
                .query_map(
                    rusqlite::params![self.scan_id, self.attempt_number],
                    |row| {
                        Ok(json!({
                            "engine": row.get::<_, String>(0)?,
                            "status": row.get::<_, String>(1)?,
                            "gap": row.get::<_, String>(2)?,
                            "version": row.get::<_, String>(3)?,
                            "rulePackDigest": row.get::<_, String>(4)?,
                            "provenance": row.get::<_, String>(5)?,
                        }))
                    },
                )
                .map_err(|error| {
                    BrokerDenial::new("internal_error", format!("无法读取分析器记录：{error}"))
                })?;
            for row in rows.flatten() {
                if engine.is_empty()
                    || row.get("engine").and_then(JsonValue::as_str) == Some(engine)
                {
                    runs.push(row);
                }
            }
        }
        let mut candidates = Vec::new();
        {
            let mut statement = connection
                .prepare(
                    "SELECT r.logical_key,r.revision_hash,r.envelope_json
                     FROM import_projection_memberships m
                     JOIN import_record_revisions r ON r.id=m.revision_id
                     WHERE m.scope_key=?1 AND m.current=1 AND m.tombstone=0
                       AND r.record_kind='finding_candidate'
                     ORDER BY r.logical_key LIMIT ?2",
                )
                .map_err(|error| {
                    BrokerDenial::new("internal_error", format!("无法读取导入结果：{error}"))
                })?;
            let rows = statement
                .query_map(
                    rusqlite::params![self.result_scope_key, limit as i64],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(|error| {
                    BrokerDenial::new("internal_error", format!("无法读取导入结果：{error}"))
                })?;
            for (key, revision, envelope) in rows.flatten() {
                let Ok(parsed) = serde_json::from_str::<JsonValue>(&envelope) else {
                    continue;
                };
                if !engine.is_empty() {
                    let named = parsed
                        .pointer("/payload/rule_id")
                        .or_else(|| parsed.pointer("/payload/symbol"))
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    let adapter = parsed
                        .pointer("/provenance/import_adapter")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if !named.contains(engine) && !adapter.contains(engine) {
                        continue;
                    }
                }
                candidates.push(json!({
                    "key": key,
                    "revisionHash": revision,
                    "title": parsed.pointer("/payload/title").and_then(JsonValue::as_str).unwrap_or_default(),
                    "ruleId": parsed.pointer("/payload/rule_id").and_then(JsonValue::as_str).unwrap_or_default(),
                    "severity": parsed.pointer("/payload/severity").and_then(JsonValue::as_str).unwrap_or_default(),
                    "endpoint": parsed.pointer("/payload/endpoint").and_then(JsonValue::as_str).unwrap_or_default(),
                    "reviewState": "candidate",
                }));
            }
        }
        Ok(json!({
            "runs": runs,
            "candidates": candidates,
            "scopeKey": self.result_scope_key,
            "note": "规则命中和 security hotspot 都只是候选，确认权在评审面",
        }))
    }

    pub(super) fn get_result(
        &self,
        connection: &Connection,
        map: &serde_json::Map<String, JsonValue>,
    ) -> Result<JsonValue, BrokerDenial> {
        if let Some(view) = &self.analysis_view {
            let key = map
                .get("key")
                .and_then(JsonValue::as_str)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| {
                    BrokerDenial::new("invalid_arguments", "analyzer.get_result 需要 key")
                })?;
            let results = super::super::results::AnalysisResults::load(connection, view)
                .map_err(|e| BrokerDenial::new("source_results_unavailable", e))?;
            let matching: Vec<_> = results
                .candidates(connection)
                .map_err(|e| BrokerDenial::new("source_results_unavailable", e))?
                .into_iter()
                .filter(|candidate| candidate.receipt.record.logical_key == key)
                .collect();
            if matching.len() != 1 {
                return Err(BrokerDenial::new(
                    "result_not_found",
                    "本轮分析没有唯一对应的已接收结果",
                ));
            }
            let candidate = &matching[0];
            let record = candidate.receipt;
            return Ok(
                json!({"key":key,"revisionHash":record.record.revision_hash,"engine":record.engine,
                "acceptedBundleId":record.bundle_id,"acceptedArtifactSha256":record.record.source_artifact_id,
                "acceptedSources":candidate.sources,
                "reviewState":"candidate","envelope":candidate.envelope,"analysisManifestDigest":results.analysis_digest,
                "analysisResultsDigest":results.digest(),"scanId":self.scan_id,"attemptNumber":self.attempt_number}),
            );
        }
        let key = map
            .get("key")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                BrokerDenial::new("invalid_arguments", "analyzer.get_result 需要 key")
            })?;
        let envelope: Option<String> = connection
            .query_row(
                "SELECT r.envelope_json
                 FROM import_projection_memberships m
                 JOIN import_record_revisions r ON r.id=m.revision_id
                 WHERE m.scope_key=?1 AND r.logical_key=?2 AND m.current=1 AND m.tombstone=0
                 ORDER BY r.id DESC LIMIT 1",
                params![self.result_scope_key, key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| {
                BrokerDenial::new("internal_error", format!("无法读取记录 {key}：{error}"))
            })?;
        let Some(text) = envelope else {
            return Err(BrokerDenial::new(
                "result_not_found",
                format!("{key} 不在本次扫描的当前结果里"),
            ));
        };
        let parsed: JsonValue = serde_json::from_str(&text).map_err(|error| {
            BrokerDenial::new("internal_error", format!("记录信封不可解析：{error}"))
        })?;
        Ok(json!({
            "key": key,
            "reviewState": "candidate",
            "envelope": parsed,
        }))
    }
}
