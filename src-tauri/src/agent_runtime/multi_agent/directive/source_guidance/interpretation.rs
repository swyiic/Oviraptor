//! Source-only interpretation of confirmed operator analysis preferences.
use super::*;

/// An explicit addressed focus is not a request to create another specialist.
/// Apply this interpretation only to persisted source roots; existing Web
/// proposals and historical draft hashes retain their original meaning.
pub(crate) fn interpret_focus(
    db: &Connection,
    root: &str,
    target: &str,
    thread: &str,
    text: &str,
    draft: &mut DraftInterpretation,
) -> Result<(), String> {
    let redacted = crate::agent_runtime::secrets::redact_text_with(text, None);
    let text = redacted.as_str();
    if !["team", target, &format!("coordinator:{root}")].contains(&thread)
        || draft.status != "drafted"
        || draft.coordinator_decision != "accept"
        || draft.side_effect_class != "read_only"
        || draft.proposed_scope_change.is_some()
        || !draft.requested_contracts.is_empty()
        || draft.priority_changes != ["evaluate_requested_priority_change"]
        || draft.requested_roles.len() != 1
    {
        return Ok(());
    }
    let source: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=id
         AND role='coordinator' AND target_url=?2 AND target_url LIKE 'source:%'
         AND json_extract(plan_json,'$.surface')='source')",
            params![root, target],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !source {
        return Ok(());
    }
    if draft.intent == "priority_adjustment" && draft.requested_roles == ["coordinator"] {
        draft
            .reason_codes
            .push("source_guidance_tool_round_eligible".into());
        draft.estimated_requests = 0;
        draft.safe_execution_text = format!("仅作为源码下一未冻结阶段或工具轮次的分析关注点；不新增任务、裁决、工具或范围权限：{text}");
        return Ok(());
    }
    if draft.intent != "agent_proposal_request" {
        return Ok(());
    }
    let Some((mention, focus)) = text.split_once(char::is_whitespace) else {
        return Ok(());
    };
    let role = match mention.to_ascii_lowercase().as_str() {
        "@repo_mapper" | "@仓库梳理" => "repo_mapper",
        "@source_analyst" | "@源码分析" => "source_analyst",
        "@reviewer" | "@检查" | "@审查" => "evidence_reviewer",
        _ => return Ok(()),
    };
    let focus = focus.trim().to_ascii_lowercase();
    if draft.requested_roles != [role]
        || ![
            "请关注",
            "请解释",
            "请分析",
            "关注：",
            "解释：",
            "focus on ",
            "explain ",
        ]
        .iter()
        .any(|prefix| {
            focus
                .strip_prefix(prefix)
                .is_some_and(|s| !s.trim().is_empty())
        })
    {
        return Ok(());
    }
    draft.intent = "source_analysis_focus".into();
    draft.estimated_requests = 0;
    draft.estimated_tokens = (128 + text.chars().count() as i64 / 4).clamp(128, 800);
    draft
        .reason_codes
        .push("source_focus_next_unfrozen_role_phase".into());
    draft
        .reason_codes
        .push("source_guidance_tool_round_eligible".into());
    draft.safe_execution_text = format!(
        "仅作为指定源码角色下一未冻结阶段（含工具阶段后续轮次）的分析关注点；不新增任务、不指定裁决、不授予工具或范围权限；没有适用阶段则不送达：{text}"
    );
    Ok(())
}
