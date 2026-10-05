//! Frozen, operator-confirmed queue preferences, not target execution results.
use super::*;
use crate::agent_runtime::{contract::COVERAGE_FAMILIES, store::stable_hash};

pub(super) fn explicit_priority_family(text: &str) -> Option<&'static str> {
    let lower = text
        .trim()
        .trim_end_matches(['。', '.'])
        .to_ascii_lowercase();
    let command = lower
        .strip_prefix("@coordinator ")
        .or_else(|| lower.strip_prefix("@协调 "))
        .or_else(|| lower.strip_prefix("@总控 "))
        .unwrap_or(&lower);
    let requested = command
        .strip_prefix("请优先检查")
        .or_else(|| command.strip_prefix("优先检查"))
        .or_else(|| command.strip_prefix("prioritize "))?
        .trim();
    // Exact grammar deliberately excludes negation, multiple clauses, URLs,
    // new capabilities and instructions directed to a specialist.
    let family = match requested {
        "信息泄露" => "information_disclosure",
        "错误处理" => "error_handling",
        "认证会话" => "authentication_session",
        "权限控制" => "authorization",
        "输入反射" => "input_reflection_xss",
        "隐藏接口" => "hidden_interface_discovery",
        "业务流程" => "business_flow",
        value => value,
    };
    COVERAGE_FAMILIES.into_iter().find(|known| *known == family)
}

fn matches_family(key: &str, family: &str) -> bool {
    key.strip_prefix("family:") == Some(family)
        || key
            .strip_prefix("contract:")
            .and_then(|value| value.split_once('|'))
            .is_some_and(|(category, _)| category == family)
}

fn promote(queue: &mut [String], family: &str) -> usize {
    let count = queue
        .iter()
        .filter(|key| matches_family(key, family))
        .count();
    // Stable partition: no work is removed or synthesized, including duplicates.
    queue.sort_by_key(|key| !matches_family(key, family));
    count
}

fn priority_family(draft: &HumanDirectiveDraft) -> Option<&str> {
    draft
        .priority_changes
        .first()
        .filter(|_| {
            draft.priority_changes.len() == 1
                && draft.intent == "priority_adjustment"
                && draft.recipient_role == "coordinator"
                && draft.side_effect_class == "read_only"
                && draft.requested_roles == ["coordinator"]
                && draft.proposed_scope_change.is_none()
        })
        .and_then(|action| action.strip_prefix("prioritize_family:"))
        .filter(|family| COVERAGE_FAMILIES.contains(family))
}

/// Historical consistency check, deliberately independent of the current lease
/// and current graph revision. It never repairs records or authorizes new work.
pub(crate) fn project_receipt(
    connection: &Connection,
    id: &str,
) -> Result<Option<serde_json::Value>, String> {
    let raw: String = connection.query_row(
        "SELECT json_object('scan',scan_id,'attempt',attempt_number,'root',root_run_id,'target',target_key,\
         'role',recipient_role,'text',text_redacted,'status',status,'draft',source_draft_id,\
         'revision',confirmed_revision,'hash',confirmed_hash,'thread',thread_key,\
         'claimRun',claim_run_id,'epoch',claim_lease_epoch,'token',claim_fencing_token,\
         'confirmed',confirmation_at,'accepted',accepted_at,'applied',applied_at,'finished',finished_at,\
         'payload',json(payload_json)) FROM agent_user_directives WHERE id=?1",
        [id], |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    let record: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let draft = load_draft(connection, record["draft"].as_str().unwrap_or_default())?;
    let action: Option<(String, String)> = connection
        .query_row(
            "SELECT family,receipt_json FROM agent_directive_queue_actions WHERE directive_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let has_priority = |value: &serde_json::Value| {
        value.as_array().is_some_and(|items| {
            items.iter().any(|v| {
                v.as_str()
                    .is_some_and(|s| s.starts_with("prioritize_family:"))
            })
        })
    };
    let priority = action.is_some()
        || has_priority(&record["payload"]["priorityChanges"])
        || draft.as_ref().is_some_and(|d| {
            d.priority_changes
                .iter()
                .any(|s| s.starts_with("prioritize_family:"))
        })
        || explicit_priority_family(record["text"].as_str().unwrap_or_default()).is_some();
    if !priority || (record["status"] != "completed" && action.is_none()) {
        return Ok(None);
    }
    let invalid = "directive_queue_receipt_unverified";
    let draft = draft.ok_or(invalid)?;
    let family = priority_family(&draft).ok_or(invalid)?;
    let (stored_family, raw_receipt) = action.ok_or(invalid)?;
    let receipt: serde_json::Value = serde_json::from_str(&raw_receipt).map_err(|_| invalid)?;
    let unbound = draft.root_run_id.is_empty()
        && draft.target_key.is_empty()
        && draft.bound_lease_epoch == 0
        && draft.bound_fencing_token.is_empty();
    let bound = record["root"] == draft.root_run_id
        && record["target"] == draft.target_key
        && record["epoch"] == draft.bound_lease_epoch
        && record["token"] == draft.bound_fencing_token;
    let expected_payload = confirmed_payload(&draft);
    let payload_matches = expected_payload
        .as_object()
        .expect("confirmed payload object")
        .iter()
        .all(|(key, value)| record["payload"].get(key) == Some(value));
    let valid_hash = |key: &str| {
        receipt[key].as_str().is_some_and(|hash| {
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    };
    if !draft_integrity_valid(&draft)
        || draft.status != "confirmed"
        || draft.confirmed_directive_id != id
        || draft.validation_result == "rejected"
        || record["status"] != "completed"
        || record["scan"] != draft.scan_id
        || record["attempt"] != draft.attempt_number
        || record["role"] != draft.recipient_role
        || record["revision"] != draft.revision
        || record["hash"] != draft.draft_hash
        || record["text"] != draft.safe_execution_text
        || record["thread"] != draft.thread_key
        || explicit_priority_family(&draft.safe_execution_text) != Some(family)
        || !(unbound || bound)
        || !payload_matches
        || record["root"] != record["claimRun"]
        || !record["root"].as_str().is_some_and(|s| !s.is_empty())
        || !record["target"].as_str().is_some_and(|s| !s.is_empty())
        || !record["token"].as_str().is_some_and(|s| !s.is_empty())
        || !record["epoch"].as_i64().is_some_and(|n| n > 0)
        || ["confirmed", "accepted", "applied", "finished"]
            .iter()
            .any(|key| !record[key].as_str().is_some_and(|s| !s.is_empty()))
        || stored_family != family
        || receipt["kind"] != "prioritize_family"
        || receipt["family"] != family
        || receipt["leaseEpoch"] != record["epoch"]
        || receipt["targetRequests"] != 0
        || receipt["modelRequests"] != 0
        || receipt["coverageVerified"] != false
        || !receipt["matchedItems"].as_u64().is_some_and(|n| n > 0)
        || !receipt["changedOrder"].is_boolean()
        || !valid_hash("beforeQueueHash")
        || !valid_hash("afterQueueHash")
        || receipt["changedOrder"] != (receipt["beforeQueueHash"] != receipt["afterQueueHash"])
    {
        return Err(invalid.into());
    }
    let events: (Option<i64>, Option<i64>) = connection.query_row(
        "SELECT MAX(CASE WHEN json_extract(payload_json,'$.status')='applied' THEN sequence END),\
         MAX(CASE WHEN json_extract(payload_json,'$.status')='completed' THEN sequence END) \
         FROM agent_collaboration_events WHERE entity_id=?1 AND entity_type='user_directive' \
         AND event_type='user_directive' AND scan_id=?2 AND attempt_number=?3 \
         AND json_extract(payload_json,'$.sourceDraftId')=?4",
        params![id, draft.scan_id, draft.attempt_number, draft.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|e| e.to_string())?;
    if !matches!(events, (Some(applied), Some(completed)) if applied < completed) {
        return Err(invalid.into());
    }
    Ok(Some(receipt))
}

/// Commit rules and receipts atomically before publishing a new in-memory queue.
/// Rules replay each round (also after checkpoint loss), scoped to this root and
/// attempt. A new lease can replay completed actions but cannot adopt pending
/// instructions from an old fence. Newer committed preferences take precedence.
// Storage-only replay wrapper; production always supplies the original parent gate.
#[cfg(test)]
pub fn apply_queue_actions(
    connection: &Connection,
    lease: &CoordinatorLease,
    queue: &[String],
) -> Result<(Vec<UserDirective>, Vec<String>), String> {
    apply_for_original_parent(connection, lease, queue, &|_| Ok(()))
}

pub(crate) fn apply_for_original_parent(
    connection: &Connection,
    lease: &CoordinatorLease,
    queue: &[String],
    check_original_parent: &dyn Fn(&Connection) -> Result<(), String>,
) -> Result<(Vec<UserDirective>, Vec<String>), String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
    check_original_parent(&transaction)?;
    let items = prepare_model_context_in_transaction(&transaction, lease)?;
    let mut ordered = queue.to_vec();
    {
        let mut query = transaction.prepare(
            "SELECT d.id FROM agent_user_directives d LEFT JOIN agent_directive_queue_actions a ON d.id=a.directive_id \
             WHERE d.scan_id=?1 AND d.attempt_number=?2 AND d.target_key=?3 AND d.root_run_id=?4 \
             AND (d.status='completed' OR a.directive_id IS NOT NULL) ORDER BY a.sequence,d.rowid",
        ).map_err(|error| error.to_string())?;
        let rules = query
            .query_map(
                params![
                    lease.scan_id,
                    lease.attempt_number,
                    lease.target_key,
                    lease.root_run_id
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| error.to_string())?;
        let mut preferences = Vec::new();
        for rule in rules {
            let id = rule.map_err(|error| error.to_string())?;
            if let Some(receipt) = project_receipt(&transaction, &id)? {
                let family = receipt["family"]
                    .as_str()
                    .ok_or("directive_queue_rule_invalid")?
                    .to_owned();
                preferences.retain(|previous| previous != &family);
                preferences.push(family);
            }
        }
        // Validate every persisted rule, but sort at most once per known family.
        for family in preferences {
            promote(&mut ordered, &family);
        }
    }
    let mut remaining = Vec::new();
    for item in items {
        let source: String = transaction
            .query_row(
                "SELECT source_draft_id FROM agent_user_directives WHERE id=?1",
                [&item.id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        // The shared inbox validation just verified the hash, confirmation,
        // text, thread, evidence refs and exact fence under this write lock.
        let draft = load_draft(&transaction, &source)?.ok_or("directive_draft_missing")?;
        let Some(family) = priority_family(&draft) else {
            remaining.push(item);
            continue;
        };
        let before = ordered.clone();
        let matched = promote(&mut ordered, family);
        if matched == 0 {
            transition_directive_in_transaction(
                &transaction,
                lease,
                &item.id,
                "accepted",
                "deferred",
            )?;
            transaction.execute(
                "UPDATE agent_user_directives SET rejection_code='priority_no_matching_pending_work' WHERE id=?1", [&item.id],
            ).map_err(|error| error.to_string())?;
            continue;
        }
        let receipt = json!({
            "kind":"prioritize_family", "family":family, "matchedItems":matched,
            "changedOrder":before != ordered,
            "beforeQueueHash":stable_hash(&serde_json::to_string(&before).map_err(|e|e.to_string())?),
            "afterQueueHash":stable_hash(&serde_json::to_string(&ordered).map_err(|e|e.to_string())?),
            "targetRequests":0, "modelRequests":0, "coverageVerified":false,
            "leaseEpoch":lease.lease_epoch,
        });
        transaction.execute(
            "INSERT INTO agent_directive_queue_actions(directive_id,family,receipt_json) VALUES(?1,?2,?3)",
            params![item.id,family,receipt.to_string()],
        ).map_err(|error| error.to_string())?;
        transition_directive_in_transaction(&transaction, lease, &item.id, "accepted", "applied")?;
        transition_directive_in_transaction(&transaction, lease, &item.id, "applied", "completed")?;
        if project_receipt(&transaction, &item.id)? != Some(receipt) {
            return Err("directive_queue_receipt_unverified".into());
        }
    }
    // Receipt/status/event writes and the published queue share the same gate.
    // Cancellation or withdrawal inside a trigger rolls every effect back.
    check_original_parent(&transaction)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok((remaining, ordered))
}
