//! Capture the entire original durable call before any financial write.
use super::*;

pub(super) struct Dispatch<'a> {
    pub family: &'static str,
    pub lease: &'a CoordinatorLease,
    pub child: &'a ScheduledChild,
    pub round: i64,
    pub request_hash: &'a str,
    pub estimate: Option<i64>,
}

impl Dispatch<'_> {
    pub fn snapshot(&self, db: &Connection) -> Result<String, String> {
        let role_bound: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND role=?2)",
                params![self.child.run_id, self.child.role.as_str()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !role_bound {
            return Err("model_cost_original_role_conflict".into());
        }
        let sql = match self.family {
            "specialist" => "SELECT json_array(assignment_id,child_run_id,root_run_id,role,
                lease_epoch,fencing_token,1,request_hash,NULL,request_json,state,response_json,
                usage_json,response_hash,event_sequence,failure_code,created_at,finished_at)
                FROM agent_specialist_calls WHERE assignment_id=?1 AND 1=?2 AND child_run_id=?3",
            "source-round" => "SELECT json_array(assignment_id,child_run_id,root_run_id,role,
                lease_epoch,fencing_token,round_number,request_hash,reserved_tokens,request_json,state,
                response_json,usage_json,response_hash,event_sequence,failure_code,created_at,finished_at)
                FROM agent_source_model_rounds WHERE assignment_id=?1 AND round_number=?2 AND child_run_id=?3",
            _ => return Err("model_cost_dispatch_family_invalid".into()),
        };
        let snapshot: String = db
            .query_row(
                sql,
                params![self.child.assignment_id, self.round, self.child.run_id],
                |r| r.get(0),
            )
            .map_err(|_| "model_cost_original_dispatch_missing")?;
        let v: Value =
            serde_json::from_str(&snapshot).map_err(|_| "model_cost_dispatch_invalid")?;
        if v[0] != self.child.assignment_id
            || v[1] != self.child.run_id
            || v[2] != self.lease.root_run_id
            || v[3] != self.child.role.as_str()
            || v[4] != self.lease.lease_epoch
            || v[5] != self.lease.fencing_token
            || v[6] != self.round
            || v[7] != self.request_hash
            || v[8] != json!(self.estimate)
            || v[9]
                .as_str()
                .is_none_or(|s| store::stable_hash(s) != self.request_hash)
            || v[10] != "executing"
            || v[11] != "{}"
            || v[12] != "{}"
            || v[13] != ""
            || v[14] != 0
            || v[15] != ""
            || v[17] != ""
            || v[16].as_str().is_none_or(str::is_empty)
            || self.round < 1
            || self.round > 256
            || self.estimate.is_some_and(|n| n < 1)
        {
            return Err("model_cost_original_dispatch_conflict".into());
        }
        Ok(snapshot)
    }

    pub fn source(&self) -> String {
        if self.family == "specialist" {
            format!(
                "specialist:{}:{}",
                self.child.assignment_id, self.request_hash
            )
        } else {
            format!(
                "source-round:{}:{}:{}",
                self.child.assignment_id, self.round, self.request_hash
            )
        }
    }
}
