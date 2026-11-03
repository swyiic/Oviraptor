//! Exact original financial write set including physical row identities.
use super::Tick;
use crate::agent_runtime::model::gateway::ModelResponse;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
impl Tick {
    pub(super) fn cost_unknown(&self, db: &Connection, r: &ModelResponse) -> Result<bool, String> {
        if self.request_fact["version"] == 3 {
            return self
                .call
                .original_actual_cost_known(db, r)
                .map(|known| !known);
        }
        let limit:Option<i64>=db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='model_input_tokens'",[&self.call.owner.root],|r|r.get(0)).map_err(|e|e.to_string())?;
        let u = &r.usage;
        Ok(!(r.usage_reported
            && [
                u.input_tokens,
                u.cached_input_tokens,
                u.output_tokens,
                u.model_requests,
            ]
            .iter()
            .all(|n| *n >= 0)
            && u.model_requests == 1
            && u.cached_input_tokens <= u.input_tokens
            && u.input_tokens.checked_add(u.output_tokens) == Some(u.total_tokens)
            && u.total_tokens >= 0
            && (limit.is_none() || u.total_tokens <= self.call.tokens)))
    }
    pub(super) fn cost_rows(
        &self,
        db: &Connection,
        response: Option<&ModelResponse>,
    ) -> Result<Value, String> {
        use std::collections::BTreeMap;
        let prefix = format!("root:{}:root-model:{}:", self.call.owner.id, self.call.id);
        let mut expected = BTreeMap::new();
        let values = response.map(|r| {
            [
                r.usage.input_tokens,
                r.usage.cached_input_tokens,
                r.usage.output_tokens,
                r.usage.model_requests,
            ]
        });
        let known = match response {
            Some(r) => !self.cost_unknown(db, r)?,
            None => false,
        };
        let finite_actual = self.request_fact["version"] == 3
            && db.query_row("SELECT hard_limit IS NOT NULL FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='model_input_tokens'", [&self.call.owner.root], |r| r.get::<_, bool>(0)).map_err(|e|e.to_string())?;
        for (index, dimension) in crate::agent_runtime::multi_agent::budget::DIMENSIONS[..4]
            .iter()
            .enumerate()
        {
            let reserve = if index == 3 { 1 } else { self.call.tokens };
            expected.insert(
                format!("{prefix}{dimension}:reserve"),
                (dimension.to_string(), "reserve".to_string(), reserve),
            );
            if let Some(values) = values {
                let held = if known && index < 3 {
                    reserve.max(values[index])
                } else {
                    reserve
                };
                if held > reserve {
                    expected.insert(
                        format!(
                            "{prefix}{dimension}:{}",
                            if finite_actual { "actual" } else { "unlimited" }
                        ),
                        (dimension.to_string(), "reserve".into(), held - reserve),
                    );
                }
                let (kind, amount) = if index == 3 {
                    ("consume", 1)
                } else if known {
                    ("consume", values[index])
                } else {
                    ("forfeit", held)
                };
                if amount > 0 {
                    expected.insert(
                        format!("{prefix}{dimension}:terminal"),
                        (dimension.to_string(), kind.into(), amount),
                    );
                }
                if (index == 3 || known) && held > amount {
                    expected.insert(
                        format!("{prefix}{dimension}:release"),
                        (dimension.to_string(), "release".into(), held - amount),
                    );
                }
            }
        }
        let mut stmt=db.prepare("SELECT rowid,entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at
            FROM agent_budget_entries WHERE (source_id=?1 OR idempotency_key LIKE ?2)
              AND kind<>'reconcile'
              AND (?3=0 OR substr(idempotency_key,-8)=':reserve') ORDER BY entry_id").map_err(|e|e.to_string())?;
        let rows=stmt.query_map(params![self.call.id,format!("{prefix}%"),response.is_none()],|r|Ok(json!({
            "rowid":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"root":r.get::<_,String>(2)?,"assignment":r.get::<_,String>(3)?,
            "owner":r.get::<_,String>(4)?,"dimension":r.get::<_,String>(5)?,"kind":r.get::<_,String>(6)?,"amount":r.get::<_,i64>(7)?,
            "key":r.get::<_,String>(8)?,"source":r.get::<_,String>(9)?,"created":r.get::<_,String>(10)?,
        }))).map_err(|e|e.to_string())?.collect::<Result<Vec<Value>,_>>().map_err(|e|e.to_string())?;
        for row in &rows {
            let key = row["key"]
                .as_str()
                .ok_or("root_tick_original_cost_changed")?;
            let tuple = expected
                .remove(key)
                .ok_or("root_tick_original_cost_changed")?;
            if row["root"] != self.call.owner.root
                || row["assignment"] != ""
                || row["owner"] != self.call.owner.id
                || row["source"] != self.call.id
                || row["dimension"] != tuple.0
                || row["kind"] != tuple.1
                || row["amount"] != tuple.2
                || row["rowid"].as_i64().is_none_or(|n| n <= 0)
                || row["id"]
                    .as_str()
                    .is_none_or(|s| uuid::Uuid::parse_str(s).is_err())
            {
                return Err("root_tick_original_cost_changed".into());
            }
        }
        if !expected.is_empty() {
            return Err("root_tick_original_cost_missing".into());
        }
        Ok(Value::Array(rows))
    }
}

// B3.3c step ① safety net. Characterizes the ROOT model settlement cost path
// (`cost_rows`) at the reserve stage and — load-bearing for Option b — pins that
// it is strictly scoped to its own `(owner, call)` keys, so execution-scope
// entries added on the SAME root (a separate assignment) are invisible to it and
// cannot be double-counted into the model settlement. Reserve-only (response=None)
// is the tractable first slice; terminal/invoice cases are a follow-up brick.
#[cfg(test)]
mod cost_rows_characterization {
    use super::Tick;
    use super::super::RootModelCall;
    use super::super::super::RootOwner;
    use crate::agent_runtime::model::gateway::ModelResponse;
    use crate::agent_runtime::store::UsageDelta;
    use rusqlite::{params, Connection};

    const DIMS: [&str; 4] = [
        "model_input_tokens",
        "model_cached_tokens",
        "model_output_tokens",
        "model_requests",
    ];

    // Self-contained schema with exactly the columns `cost_rows` reads (entry_id,
    // root_run_id, assignment_id, lease_attempt_id, dimension, kind, amount,
    // idempotency_key, source_id, created_at) + the limits row it queries for
    // `finite_actual`. No triggers/FK/CHECK needed to exercise the read path.
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE agent_budget_limits(root_run_id TEXT, dimension TEXT, hard_limit INTEGER);
             CREATE TABLE agent_budget_entries(
               entry_id TEXT PRIMARY KEY, root_run_id TEXT, assignment_id TEXT, lease_attempt_id TEXT,
               dimension TEXT, kind TEXT, amount INTEGER, idempotency_key TEXT, source_id TEXT,
               created_at TEXT DEFAULT (datetime('now','localtime')));
             INSERT INTO agent_budget_limits(root_run_id,dimension,hard_limit) VALUES('root','model_input_tokens',NULL);",
        )
        .unwrap();
        c
    }

    // One root-model reserve row exactly as the claim path writes it:
    // key `root:{owner}:root-model:{call}:{dim}:reserve`, source=call, assignment=''.
    fn reserve_row(c: &Connection, owner: &str, call: &str, dim: &str, amount: i64) {
        c.execute(
            "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id) \
             VALUES(?1,'root','',?2,?3,'reserve',?4,?5,?6)",
            params![
                uuid::Uuid::new_v4().to_string(), owner, dim, amount,
                format!("root:{owner}:root-model:{call}:{dim}:reserve"), call
            ],
        )
        .unwrap();
    }

    fn seed_model_reserves(c: &Connection, owner: &str, call: &str, tokens: i64) {
        for dim in &DIMS[..3] {
            reserve_row(c, owner, call, dim, tokens);
        }
        reserve_row(c, owner, call, "model_requests", 1);
    }

    fn tick(owner: &str, call: &str, tokens: i64) -> Tick {
        Tick::for_test(
            RootModelCall::for_test(RootOwner::for_test("root", owner), call, tokens),
            3,
        )
    }

    #[test]
    fn reserve_only_verifies_the_four_model_reserves() {
        let c = db();
        seed_model_reserves(&c, "own-1", "call-1", 7000);
        let rows = tick("own-1", "call-1", 7000).cost_rows(&c, None).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 4);
    }

    #[test]
    fn scoped_to_its_own_call_ignoring_other_scopes() {
        let c = db();
        seed_model_reserves(&c, "own-1", "call-1", 7000);
        // A DIFFERENT root-model call's reserve on the same root+owner: ends in
        // ':reserve' but a different call prefix + source. Must be ignored.
        reserve_row(&c, "own-1", "call-2", "model_input_tokens", 500);
        // An EXECUTION-scope reserve (a separate assignment) in `model::reserve`
        // format: key `reserve:{asg}:{dim}`, source `assignment:{asg}`. This is the
        // Option-b co-resident scope on the same root; it must be invisible here.
        c.execute(
            "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id) \
             VALUES(?1,'root','exec-asg-1','own-1','model_input_tokens','reserve',999,'reserve:exec-asg-1:model_input_tokens','assignment:exec-asg-1')",
            params![uuid::Uuid::new_v4().to_string()],
        )
        .unwrap();
        let rows = tick("own-1", "call-1", 7000).cost_rows(&c, None).unwrap();
        assert_eq!(
            rows.as_array().unwrap().len(),
            4,
            "cost_rows must see only call-1's four reserves, not the other call or the exec scope"
        );
    }

    #[test]
    fn rejects_a_missing_reserve() {
        let c = db();
        // model_output_tokens omitted.
        reserve_row(&c, "own-1", "call-1", "model_input_tokens", 7000);
        reserve_row(&c, "own-1", "call-1", "model_cached_tokens", 7000);
        reserve_row(&c, "own-1", "call-1", "model_requests", 1);
        assert_eq!(
            tick("own-1", "call-1", 7000).cost_rows(&c, None).unwrap_err(),
            "root_tick_original_cost_missing"
        );
    }

    #[test]
    fn rejects_an_altered_reserve_amount() {
        let c = db();
        // Wrong amount on model_input_tokens (6999 vs the expected 7000).
        reserve_row(&c, "own-1", "call-1", "model_input_tokens", 6999);
        reserve_row(&c, "own-1", "call-1", "model_cached_tokens", 7000);
        reserve_row(&c, "own-1", "call-1", "model_output_tokens", 7000);
        reserve_row(&c, "own-1", "call-1", "model_requests", 1);
        assert_eq!(
            tick("own-1", "call-1", 7000).cost_rows(&c, None).unwrap_err(),
            "root_tick_original_cost_changed"
        );
    }

    // --- terminal/invoice settlement (response = Some), version 1 (keeps
    // cost_unknown hermetic; version 3 defers to the received journal). ---

    // One cost row at the exact key cost_rows computes: `{dim}:reserve`,
    // `{dim}:terminal` (consume|forfeit) or `{dim}:release`; source=call, ''.
    fn cost_row(c: &Connection, owner: &str, call: &str, dim: &str, suffix: &str, kind: &str, amount: i64) {
        c.execute(
            "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id) \
             VALUES(?1,'root','',?2,?3,?4,?5,?6,?7)",
            params![
                uuid::Uuid::new_v4().to_string(), owner, dim, kind, amount,
                format!("root:{owner}:root-model:{call}:{dim}:{suffix}"), call
            ],
        )
        .unwrap();
    }
    fn response(reported: bool, input: i64, cached: i64, output: i64, total: i64, requests: i64) -> ModelResponse {
        ModelResponse {
            usage: UsageDelta {
                input_tokens: input,
                cached_input_tokens: cached,
                output_tokens: output,
                total_tokens: total,
                model_requests: requests,
            },
            usage_reported: reported,
            ..Default::default()
        }
    }
    fn tick_v1(owner: &str, call: &str, tokens: i64) -> Tick {
        Tick::for_test(
            RootModelCall::for_test(RootOwner::for_test("root", owner), call, tokens),
            1,
        )
    }

    // A reported invoice within the reservation: each token dimension consumes
    // its actual and RELEASES the remainder (settle-to-zero, no leak); the request
    // dimension consumes its 1. (reserve 7000/7000/7000/1; actual 2000/0/1000/1.)
    #[test]
    fn known_invoice_consumes_actual_and_releases_remainder() {
        let c = db();
        // input: reserve 7000, consume 2000, release 5000
        cost_row(&c, "own-1", "call-1", "model_input_tokens", "reserve", "reserve", 7000);
        cost_row(&c, "own-1", "call-1", "model_input_tokens", "terminal", "consume", 2000);
        cost_row(&c, "own-1", "call-1", "model_input_tokens", "release", "release", 5000);
        // cached: reserve 7000, release 7000 (consumed 0 ⇒ no terminal row)
        cost_row(&c, "own-1", "call-1", "model_cached_tokens", "reserve", "reserve", 7000);
        cost_row(&c, "own-1", "call-1", "model_cached_tokens", "release", "release", 7000);
        // output: reserve 7000, consume 1000, release 6000
        cost_row(&c, "own-1", "call-1", "model_output_tokens", "reserve", "reserve", 7000);
        cost_row(&c, "own-1", "call-1", "model_output_tokens", "terminal", "consume", 1000);
        cost_row(&c, "own-1", "call-1", "model_output_tokens", "release", "release", 6000);
        // requests: reserve 1, consume 1 (no release)
        cost_row(&c, "own-1", "call-1", "model_requests", "reserve", "reserve", 1);
        cost_row(&c, "own-1", "call-1", "model_requests", "terminal", "consume", 1);
        let r = response(true, 2000, 0, 1000, 3000, 1);
        let rows = tick_v1("own-1", "call-1", 7000).cost_rows(&c, Some(&r)).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 10);
    }

    // An UNREPORTED invoice: each token dimension FORFEITS its whole reservation
    // (indeterminate crash-spend, never silently released), the request dimension
    // still consumes its 1.
    #[test]
    fn unknown_invoice_forfeits_each_token_reservation() {
        let c = db();
        for dim in &DIMS[..3] {
            cost_row(&c, "own-1", "call-1", dim, "reserve", "reserve", 7000);
            cost_row(&c, "own-1", "call-1", dim, "terminal", "forfeit", 7000);
        }
        cost_row(&c, "own-1", "call-1", "model_requests", "reserve", "reserve", 1);
        cost_row(&c, "own-1", "call-1", "model_requests", "terminal", "consume", 1);
        // usage_reported=false ⇒ cost is unknown ⇒ forfeit path.
        let r = response(false, 2000, 0, 1000, 3000, 1);
        let rows = tick_v1("own-1", "call-1", 7000).cost_rows(&c, Some(&r)).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 8);
    }

    // A reported invoice whose actual EXCEEDS the estimate under an unbounded
    // dimension (hard_limit NULL): the held amount grows to the actual and an
    // extra ':unlimited' reserve row records the overrun, then it is fully
    // consumed (no release). Pins the overrun path so it can never silently
    // under-reserve or leak. (input actual 9000 > reserve 7000.)
    #[test]
    fn known_invoice_over_estimate_records_the_unlimited_overrun() {
        let c = db();
        // input: reserve 7000 + unlimited 2000 (held 9000) → consume 9000 (no release)
        cost_row(&c, "own-1", "call-1", "model_input_tokens", "reserve", "reserve", 7000);
        cost_row(&c, "own-1", "call-1", "model_input_tokens", "unlimited", "reserve", 2000);
        cost_row(&c, "own-1", "call-1", "model_input_tokens", "terminal", "consume", 9000);
        // cached: reserve 7000, release 7000
        cost_row(&c, "own-1", "call-1", "model_cached_tokens", "reserve", "reserve", 7000);
        cost_row(&c, "own-1", "call-1", "model_cached_tokens", "release", "release", 7000);
        // output: reserve 7000, consume 1000, release 6000
        cost_row(&c, "own-1", "call-1", "model_output_tokens", "reserve", "reserve", 7000);
        cost_row(&c, "own-1", "call-1", "model_output_tokens", "terminal", "consume", 1000);
        cost_row(&c, "own-1", "call-1", "model_output_tokens", "release", "release", 6000);
        // requests: reserve 1, consume 1
        cost_row(&c, "own-1", "call-1", "model_requests", "reserve", "reserve", 1);
        cost_row(&c, "own-1", "call-1", "model_requests", "terminal", "consume", 1);
        let r = response(true, 9000, 0, 1000, 10000, 1);
        let rows = tick_v1("own-1", "call-1", 7000).cost_rows(&c, Some(&r)).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 10);
    }
}
