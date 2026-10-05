//! A role is not a review subject. Inspect every Reviewer assignment before
//! selecting one, so filtering for coverage cannot hide unknown/extra work.
use super::{lease::CoordinatorLease, scheduler::ScheduledChild, source};
use rusqlite::{params, Connection};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Subject {
    Candidates,
    Coverage,
}

impl Subject {
    pub(crate) fn trigger(self) -> &'static str {
        match self {
            Self::Candidates => "source_candidates_ready",
            Self::Coverage => "source_coverage_ready",
        }
    }
    pub(crate) fn from_slice(slice: &Value) -> Result<Self, String> {
        match (slice["phase"].as_str(), slice["subject"].as_str()) {
            (Some("source_review"), Some("source_candidates")) => Ok(Self::Candidates),
            (Some("source_coverage_review"), Some("source_coverage")) => Ok(Self::Coverage),
            _ => Err("source_review_subject_invalid".into()),
        }
    }
}

pub(crate) fn inventory(db: &Connection, lease: &CoordinatorLease) -> Result<(), String> {
    let plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&lease.root_run_id],
            |r| r.get(0),
        )
        .map_err(|_| "source_review_root_missing")?;
    let plan: Value = serde_json::from_str(&plan).map_err(|_| "source_review_root_invalid")?;
    let contract = source::SourcePhaseContract::from_plan(&plan)?;
    let mut stmt = db.prepare("SELECT trigger_code,task_slice_json FROM agent_assignments WHERE coordinator_run_id=?1 AND role='evidence_reviewer'").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map([&lease.root_run_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut counts = [0, 0];
    for row in rows {
        let (trigger, text) = row.map_err(|e| e.to_string())?;
        let slice: Value =
            serde_json::from_str(&text).map_err(|_| "source_review_assignment_corrupt")?;
        let subject = Subject::from_slice(&slice)?;
        let index = match subject {
            Subject::Candidates if contract.candidate_review => 0,
            Subject::Coverage if contract.coverage_review => 1,
            _ => return Err("source_phase_role_not_in_frozen_plan".into()),
        };
        if trigger != subject.trigger()
            || slice["rootRunId"] != lease.root_run_id
            || slice["scanId"] != lease.scan_id
            || slice["attemptNumber"] != lease.attempt_number
            || slice["target"] != lease.target_key
            || slice["surface"] != "source"
        {
            return Err("source_review_assignment_binding_invalid".into());
        }
        counts[index] += 1;
        if counts[index] > 1 {
            return Err("source_review_progress_extra_assignments".into());
        }
    }
    Ok(())
}

pub(crate) fn for_child(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Subject, String> {
    inventory(db, lease)?;
    let text: String = db.query_row("SELECT task_slice_json FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND role='evidence_reviewer'",
        params![child.assignment_id,child.run_id,lease.root_run_id], |r| r.get(0)).map_err(|_| "source_review_assignment_binding_invalid")?;
    Subject::from_slice(
        &serde_json::from_str(&text).map_err(|_| "source_review_assignment_corrupt")?,
    )
}

pub(crate) fn validate_slice(
    db: &Connection,
    lease: &CoordinatorLease,
    slice: &Value,
    revision: i64,
    caps: &[String],
) -> Result<(), String> {
    match Subject::from_slice(slice)? {
        Subject::Candidates => {
            super::source_reviewer::validate_slice(db, lease, slice, revision, caps)
        }
        Subject::Coverage => {
            super::source_coverage_reviewer::validate_slice(db, lease, slice, revision, caps)
        }
    }
}

pub(crate) fn verify_assignment(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Value, String> {
    match for_child(db, lease, child)? {
        Subject::Candidates => super::source_reviewer::verify_assignment(db, lease, child),
        Subject::Coverage => super::source_coverage_reviewer::verify_assignment(db, lease, child),
    }
}

pub(crate) fn validate_request(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    request: &Value,
) -> Result<(), String> {
    match for_child(db, lease, child)? {
        Subject::Candidates => super::source_reviewer::validate_request(db, lease, child, request),
        Subject::Coverage => {
            super::source_coverage_reviewer::validate_request(db, lease, child, request)
        }
    }
}
