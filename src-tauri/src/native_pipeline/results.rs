//! Immutable acceptance of actual analyzer revisions. Historical/current importer
//! memberships are deliberately not an authority for a live source attempt.
use super::analysis_view::SourceAnalysisView;
use crate::artifact_import::{
    canonical::sha256_hex, BundleStatus, ImportSummary, ImportedRecordReceipt,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptedArtifact {
    pub engine: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptedResult {
    pub bundle_id: String,
    pub engine: String,
    pub record: ImportedRecordReceipt,
}

/// A single accepted revision may have several actual analyzer origins. This
/// is a read projection, not a rewrite or an inferred upgrade of old receipts.
pub struct AcceptedCandidate<'a> {
    pub receipt: &'a AcceptedResult,
    pub envelope: Value,
    pub sources: Vec<&'a AcceptedResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalysisResults {
    pub schema_version: u32,
    pub scan_id: String,
    pub attempt_number: i64,
    pub analysis_digest: String,
    pub artifacts: Vec<AcceptedArtifact>,
    pub runs: Vec<Value>,
    pub records: Vec<AcceptedResult>,
    pub gaps: Vec<String>,
}

impl AnalysisResults {
    pub fn capture(
        connection: &Connection,
        view: &SourceAnalysisView,
        artifacts: Vec<AcceptedArtifact>,
        summary: &ImportSummary,
        outcomes: &[super::analyzer::AnalyzerOutcome],
        gaps: &[String],
    ) -> Result<Self, String> {
        let mut result = Self {
            schema_version: 1,
            scan_id: view.manifest.scan_id.clone(),
            attempt_number: view.manifest.attempt_number,
            analysis_digest: view.manifest.digest(),
            artifacts,
            runs: outcomes
                .iter()
                .map(|outcome| {
                    let mut value = outcome.evidence_json();
                    if let Some(map) = value.as_object_mut() {
                        // Reuse of the same invocation is not a new analysis identity.
                        map.remove("reused");
                        map.remove("durationMillis");
                        map.insert("version".into(), Value::String(outcome.version.clone()));
                        map.insert(
                            "gap".into(),
                            Value::String(if outcome.produced_results() {
                                String::new()
                            } else {
                                outcome.gap_reason()
                            }),
                        );
                    }
                    value
                })
                .collect(),
            records: vec![],
            gaps: gaps.to_vec(),
        };
        let mut accepted_artifacts = BTreeSet::new();
        for bundle in &summary.outcomes {
            if bundle.status == BundleStatus::Failed {
                continue;
            }
            let Some(records) = &bundle.committed_records else {
                result
                    .gaps
                    .push("analysis_result_commit_receipt_unavailable".into());
                continue;
            };
            // Validate the importer consumed precisely the bytes selected after the
            // analyzer returned, not a replaced output or an unrelated bundle.
            let mut statement=connection.prepare("SELECT f.relative_path,f.content_hash FROM import_bundle_files f JOIN import_bundles b ON b.id=f.bundle_row_id WHERE b.bundle_id=?1 AND b.source_path=?2 ORDER BY f.relative_path").map_err(|e|e.to_string())?;
            let files = statement
                .query_map(params![bundle.bundle_id, bundle.source_path], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if files.is_empty()
                || files.iter().any(|(path, hash)| {
                    !result
                        .artifacts
                        .iter()
                        .any(|a| *path == format!("{}.sarif", a.engine) && *hash == a.sha256)
                })
            {
                return Err("analysis_result_import_artifact_mismatch".into());
            }
            for artifact in &result.artifacts {
                if files.iter().any(|(path, hash)| {
                    *path == format!("{}.sarif", artifact.engine) && *hash == artifact.sha256
                }) {
                    accepted_artifacts.insert((artifact.engine.clone(), artifact.sha256.clone()));
                }
            }
            for record in records {
                let artifacts: Vec<_> = result
                    .artifacts
                    .iter()
                    .filter(|a| a.sha256 == record.source_artifact_id)
                    .collect();
                if artifacts.is_empty() {
                    return Err("analysis_result_record_artifact_mismatch".into());
                }
                let artifacts: Vec<_> = artifacts
                    .into_iter()
                    .filter(|a| {
                        files.iter().any(|(path, hash)| {
                            *path == format!("{}.sarif", a.engine) && *hash == a.sha256
                        })
                    })
                    .collect();
                if artifacts.is_empty() {
                    return Err("analysis_result_bundle_artifact_mismatch".into());
                }
                let envelope = record.read(connection)?;
                if record.record_kind == "finding_candidate" && !selected_locations(view, &envelope)
                {
                    result.gaps.push(format!(
                        "analysis_result_location_outside_view:{}",
                        record.logical_key
                    ));
                    continue;
                }
                // Equal bytes prove equal record content, not a single engine.
                // Keep every matching filename/hash accepted by THIS import.
                for artifact in artifacts {
                    result.records.push(AcceptedResult {
                        bundle_id: bundle.bundle_id.clone(),
                        engine: artifact.engine.clone(),
                        record: record.clone(),
                    });
                }
            }
        }
        for artifact in &result.artifacts {
            if !accepted_artifacts.contains(&(artifact.engine.clone(), artifact.sha256.clone())) {
                result.gaps.push(format!(
                    "analysis_result_artifact_not_accepted:{}",
                    artifact.engine
                ));
            }
        }
        result.records.sort_by(|a, b| {
            (
                &a.record.logical_key,
                &a.record.revision_hash,
                &a.engine,
                &a.bundle_id,
                &a.record.source_artifact_id,
            )
                .cmp(&(
                    &b.record.logical_key,
                    &b.record.revision_hash,
                    &b.engine,
                    &b.bundle_id,
                    &b.record.source_artifact_id,
                ))
        });
        result.records.dedup();
        result.gaps.sort();
        result.gaps.dedup();
        Ok(result)
    }

    pub fn digest(&self) -> String {
        sha256_hex(
            serde_json::to_string(self)
                .expect("serializable receipt")
                .as_bytes(),
        )
    }

    pub fn store(&self, connection: &Connection, view: &SourceAnalysisView) -> Result<(), String> {
        view.verify_receipt(connection)?;
        if self.scan_id != view.manifest.scan_id
            || self.attempt_number != view.manifest.attempt_number
            || self.analysis_digest != view.manifest.digest()
        {
            return Err("analysis_result_view_mismatch".into());
        }
        let exists:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM source_analysis_results WHERE scan_id=?1 AND attempt_number=?2)",params![self.scan_id,self.attempt_number],|r|r.get(0)).map_err(|e|e.to_string())?;
        if exists {
            if Self::load(connection, view)? != *self {
                return Err("analysis_results_already_frozen".into());
            }
            return Ok(());
        }
        let count=connection.execute("INSERT INTO source_analysis_results(scan_id,attempt_number,analysis_digest,receipt_json,receipt_digest) VALUES(?1,?2,?3,?4,?5)",
            params![self.scan_id,self.attempt_number,self.analysis_digest,serde_json::to_string(self).map_err(|e|e.to_string())?,self.digest()]).map_err(|e|e.to_string())?;
        if count != 1 || Self::load(connection, view)? != *self {
            return Err("analysis_result_receipt_not_committed".into());
        }
        Ok(())
    }

    pub fn load(connection: &Connection, view: &SourceAnalysisView) -> Result<Self, String> {
        view.verify_receipt(connection)?;
        let (analysis,raw,digest):(String,String,String)=connection.query_row(
            "SELECT analysis_digest,receipt_json,receipt_digest FROM source_analysis_results WHERE scan_id=?1 AND attempt_number=?2",
            params![view.manifest.scan_id,view.manifest.attempt_number],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
            .optional().map_err(|e|e.to_string())?.ok_or("analysis_result_receipt_unavailable")?;
        let value: Self = serde_json::from_str(&raw)
            .map_err(|e| format!("analysis_result_receipt_invalid:{e}"))?;
        if value.schema_version != 1
            || value.digest() != digest
            || analysis != view.manifest.digest()
            || value.analysis_digest != analysis
            || value.scan_id != view.manifest.scan_id
            || value.attempt_number != view.manifest.attempt_number
        {
            return Err("analysis_result_receipt_binding_changed".into());
        }
        for result in &value.records {
            let envelope = result.record.read(connection)?;
            if !value
                .artifacts
                .iter()
                .any(|a| a.engine == result.engine && a.sha256 == result.record.source_artifact_id)
                || (result.record.record_kind == "finding_candidate"
                    && !selected_locations(view, &envelope))
            {
                return Err("analysis_result_record_binding_changed".into());
            }
        }
        Ok(value)
    }

    pub fn candidates(
        &self,
        connection: &Connection,
    ) -> Result<Vec<AcceptedCandidate<'_>>, String> {
        let mut candidates = BTreeMap::<(&str, &str), AcceptedCandidate<'_>>::new();
        for source in self
            .records
            .iter()
            .filter(|r| r.record.record_kind == "finding_candidate")
        {
            let envelope = source.record.read(connection)?;
            let key = (
                source.record.logical_key.as_str(),
                source.record.revision_hash.as_str(),
            );
            if let Some(candidate) = candidates.get_mut(&key) {
                if candidate.receipt.record.revision_id != source.record.revision_id
                    || candidate.envelope != envelope
                {
                    return Err("analysis_result_candidate_revision_conflict".into());
                }
                if !candidate.sources.contains(&source) {
                    candidate.sources.push(source);
                }
            } else {
                candidates.insert(
                    key,
                    AcceptedCandidate {
                        receipt: source,
                        envelope,
                        sources: vec![source],
                    },
                );
            }
        }
        Ok(candidates.into_values().collect())
    }
}

fn selected_locations(view: &SourceAnalysisView, envelope: &Value) -> bool {
    let selected = |path: &str| view.manifest.files.iter().any(|f| f.path == path);
    envelope
        .pointer("/payload/path")
        .and_then(Value::as_str)
        .is_some_and(selected)
        && envelope
            .pointer("/payload/locations")
            .and_then(Value::as_array)
            .is_some_and(|locations| {
                !locations.is_empty()
                    && locations.iter().all(|location| {
                        location
                            .get("path")
                            .and_then(Value::as_str)
                            .is_some_and(selected)
                    })
            })
}
