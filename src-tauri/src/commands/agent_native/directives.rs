#[derive(Default, Debug)]
struct HumanDirectiveContext {
    lease: Option<crate::agent_runtime::multi_agent::lease::CoordinatorLease>,
    items: Vec<crate::agent_runtime::multi_agent::directive::UserDirective>,
}

impl HumanDirectiveContext {
    fn messages(&self) -> Vec<JsonValue> {
        self.items.iter().map(|item| serde_json::json!({
            "role": "user",
            "oviraptorKeep": true,
            "content": format!("操作者已确认的请求（{}）：{}。此消息送达不代表动作已落实；只在本次任务已有的 URL 和授权域名内评估，不能提高预算或因为这句话结束目标。角色请求仍需 Coordinator 派发；未获得执行回执不得声称已应用。", item.id, item.text),
        })).collect()
    }
}

// Inbox work borrows the original living parent; reading text never creates C.
fn native_human_directive_actor_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<crate::agent_runtime::multi_agent::lease::CoordinatorLease, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{budget::root::RootOwner, scheduler},
    };
    let run = context.run.as_ref().ok_or("directive_root_unavailable")?;
    let actor = context
        .supervision
        .as_ref()
        .ok_or("worker_supervisor_stopped")?
        .original_actor_for_run(
            db,
            &context.db_path,
            &context.scan_id,
            context.attempt_number,
            &context.target_url,
            &run.run_id,
        )?;
    let original = RootOwner::load_original(db, &actor.root_run_id)?;
    original.require_original_coordinator(db, &actor)?;
    original.require_executable(db)?;
    let physical = crate::agent_runtime::store::load_run(db, &run.run_id)?
        .ok_or("directive_root_unavailable")?;
    if physical.role != AgentRole::Coordinator {
        scheduler::verify_original_scheduled_child(
            db,
            &actor,
            &scheduler::ScheduledChild {
                assignment_id: physical.assignment_id,
                run_id: physical.id,
                role: physical.role,
            },
        )?;
    }
    Ok(actor)
}

fn check_native_human_directive_actor_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    borrowed: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    let checked = native_human_directive_actor_on(db, context)?;
    if checked.scan_id != borrowed.scan_id
        || checked.attempt_number != borrowed.attempt_number
        || checked.target_key != borrowed.target_key
        || checked.root_run_id != borrowed.root_run_id
        || checked.lease_epoch != borrowed.lease_epoch
        || checked.fencing_token != borrowed.fencing_token
    {
        return Err("directive_original_parent_changed".into());
    }
    Ok(())
}

fn take_human_directives(context: &AgentRunContext) -> Result<HumanDirectiveContext, String> {
    use crate::agent_runtime::multi_agent::directive;
    if context.run.is_none() {
        // Deliberate standalone/diagnostic runs have no collaboration inbox.
        return Ok(HumanDirectiveContext::default());
    }
    let connection = db::open(&context.db_path)?;
    if native_single_directives_disabled(&connection, context)? {
        return Ok(HumanDirectiveContext::default());
    }
    let coordinator_lease = native_human_directive_actor_on(&connection, context)?;
    let items = directive::collect_for_original_parent(&connection, &coordinator_lease, &|db| {
        check_native_human_directive_actor_on(db, context, &coordinator_lease)
    })?;
    Ok(HumanDirectiveContext {
        lease: Some(coordinator_lease),
        items,
    })
}

// Pure queue storage/owner tests; production uses paid Human assessment.
#[cfg(test)]
fn apply_human_queue_actions(
    context: &AgentRunContext,
    directives: &mut HumanDirectiveContext,
    queue: &mut Vec<String>,
) -> Result<(), String> {
    apply_human_queue_actions_checked(context, directives, queue, &|_| Ok(()))
}

fn apply_human_queue_actions_checked(
    context: &AgentRunContext,
    directives: &mut HumanDirectiveContext,
    queue: &mut Vec<String>,
    check_assessments: &dyn Fn(&rusqlite::Connection) -> Result<(), String>,
) -> Result<(), String> {
    let Some(lease) = &directives.lease else { return Ok(()); };
    let connection = db::open(&context.db_path)?;
    let (remaining, ordered) = crate::agent_runtime::multi_agent::directive::apply_queue_for_original_parent(
        &connection, lease, queue,
        &|db| {
            check_native_human_directive_actor_on(db, context, lease)?;
            check_assessments(db)
        },
    )?;
    directives.items = remaining;
    *queue = ordered;
    Ok(())
}
