//! Trusted chat adapter to the SAME persisted discovery owner used by the UI.
//! Confirmation is a session-authenticated human action, never a model tool.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewRead {
    run_id: Option<String>,
    #[serde(default = "all_filter")]
    filter: String,
    #[serde(default = "first_page")]
    page: i64,
    #[serde(default)]
    search: String,
}
fn all_filter() -> String {
    "all".into()
}
fn first_page() -> i64 {
    1
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeedRead {
    seed_id: i64,
    #[serde(default = "first_page")]
    page: i64,
    #[serde(default = "seed_page_size")]
    page_size: i64,
}
fn seed_page_size() -> i64 {
    20
}

/// Agent-facing reprobe scope. Project binding is forced server-side from the
/// Host-bound project id `p`; the model never supplies a project id. This only
/// re-runs liveness over already-authorized members and never widens scope or
/// spends money.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ReprobeArg {
    Selection {
        #[serde(rename = "assetIds")]
        asset_ids: Vec<i64>,
        #[serde(rename = "expectedCount")]
        expected_count: i64,
    },
    Query {
        #[serde(default)]
        query: crate::asset_models::AssetQuery,
        #[serde(rename = "expectedCount")]
        expected_count: i64,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReprobeRequest {
    scope: ReprobeArg,
}

pub(crate) fn dispatch(
    manager: &Manager,
    p: i64,
    action: &str,
    mut args: Value,
) -> Result<Value, String> {
    if p < 1 || !args.is_object() {
        return Err("discovery_arguments_invalid".into());
    }
    let c = db::open(&manager.path)?;
    if !c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [p],
            |r| r.get::<_, bool>(0),
        )
        .map_err(err)?
    {
        return Err("discovery_project_unavailable".into());
    }
    if args.get("projectId").is_some() || args.get("action").is_some() {
        return Err("discovery_arguments_invalid".into());
    }
    let mut value = match action {
        "start" => {
            args["projectId"] = json!(p);
            args["action"] = json!("start");
            let input: Input =
                serde_json::from_value(args).map_err(|_| "discovery_arguments_invalid")?;
            manager.start(&input)?
        }
        "pause" | "status" | "proposal" => {
            if args.as_object().is_none_or(|o| !o.is_empty()) {
                return Err("discovery_arguments_invalid".into());
            }
            if action == "pause" {
                manager.pause(p)?
            } else {
                assist_status(&c, p)?
            }
        }
        "review" => {
            let input: ReviewRead =
                serde_json::from_value(args).map_err(|_| "discovery_arguments_invalid")?;
            view_run_on(
                &c,
                p,
                input.run_id.as_deref(),
                &input.filter,
                input.page,
                &input.search,
            )?
        }
        "seeds" => {
            args["projectId"] = json!(p);
            let query: crate::asset_discovery::seed_ledger::Query =
                serde_json::from_value(args).map_err(|_| "discovery_arguments_invalid")?;
            crate::asset_discovery::seed_page::read_on(&c, &query)?
        }
        "seed_detail" => {
            let query: SeedRead =
                serde_json::from_value(args).map_err(|_| "discovery_arguments_invalid")?;
            crate::asset_discovery::seed_ledger::detail(
                &c,
                p,
                query.seed_id,
                query.page,
                query.page_size,
            )?
        }
        "prepare_seeds" => {
            if args.as_object().is_none_or(|o| !o.is_empty()) {
                return Err("discovery_arguments_invalid".into());
            }
            // The Host admits this only through its witnessed human endpoint.
            // Preparation writes derived indexes, never starts discovery work.
            crate::asset_discovery::seed_page::prepare_on(&manager.path, p)?
        }
        "confirm" => {
            args["projectId"] = json!(p);
            // Binding also exists in confirm_on; require the active assist here
            // so an old conversation cannot grant an unrelated historical run.
            let current = assist_status(&c, p)?;
            if args["runId"] != current["runId"] {
                return Err("discovery_review_changed".into());
            }
            let input: super::scope::Selection =
                serde_json::from_value(args).map_err(|_| "discovery_arguments_invalid")?;
            super::scope::confirm_and_launch(&manager.path, &manager.enrichment, &input)?
        }
        "reprobe" => {
            // Re-run liveness over already-authorized members and write back the
            // result. Every refusal below happens before the job row commits and
            // before any worker thread spawns (start_job_in validates inside one
            // IMMEDIATE transaction), so a failed start definitively did not run:
            // mark it not_started so the agent never treats it as "maybe charged".
            reprobe(manager, p, args).map_err(|e| {
                if e.starts_with("reprobe_") {
                    e
                } else {
                    format!("reprobe_not_started: {e}")
                }
            })?
        }
        _ => return Err("discovery_action_invalid".into()),
    };
    value["projectId"] = json!(p);
    value["hunterStarted"] = json!(
        value["pipeline"]["phase"] == "hunter"
            || value["pipeline"]["hunter"]["pages"].as_i64().unwrap_or(0) > 0
    );
    Ok(value)
}

/// Start a second-pass liveness reprobe over already-authorized members.
/// Project binding is forced to the Host-bound `p`; the model never supplies a
/// project id. Reuses the exact UI job path (reprobe_members_on resolves the
/// strict membership, require_idle_project enforces one job per project), so
/// this never widens scope and never spends money.
fn reprobe(manager: &Manager, p: i64, args: Value) -> Result<Value, String> {
    let jobs = manager
        .jobs
        .as_ref()
        .ok_or("reprobe_executor_unavailable")?;
    let request: ReprobeRequest =
        serde_json::from_value(args).map_err(|_| "discovery_arguments_invalid")?;
    let scope = reprobe_scope_for(p, request.scope);
    let run = crate::jobs::start_job_in(
        jobs,
        crate::asset_models::StartJobInput {
            project_id: p,
            profile_id: 0,
            configuration_source: crate::asset_models::ConfigurationSource::CurrentDefaults,
            name: "Agent · 二次存活复测".into(),
            pipeline: "reprobe".into(),
            reprobe_scope: Some(scope),
            incremental: None,
        },
    )?;
    Ok(json!({ "runId": run, "pipeline": "reprobe", "started": true }))
}

/// Build the native reprobe scope with the project binding forced to the
/// Host-bound `p`. The model's input carries no project id (ReprobeArg denies
/// unknown fields); for a selection every member is stamped with `p`, and for a
/// query any inner project id is overwritten with `p`. Resolution and the strict
/// expectedCount guard still run later in `reprobe_members_on`.
fn reprobe_scope_for(p: i64, arg: ReprobeArg) -> crate::asset_models::ReprobeScope {
    match arg {
        ReprobeArg::Selection {
            asset_ids,
            expected_count,
        } => crate::asset_models::ReprobeScope::Selection {
            selections: asset_ids
                .iter()
                .map(|id| crate::asset_models::AssetSelection {
                    project_id: p,
                    asset_id: *id,
                })
                .collect(),
            expected_count,
        },
        ReprobeArg::Query {
            mut query,
            expected_count,
        } => {
            query.project_id = Some(p);
            crate::asset_models::ReprobeScope::Query {
                query,
                expected_count,
            }
        }
    }
}

#[cfg(test)]
mod reprobe_tests {
    use super::*;
    use crate::asset_models::ReprobeScope;

    #[test]
    fn selection_scope_forces_the_host_project_on_every_member() {
        // The model supplies only asset ids; project binding is the Host's, never
        // the model's, so a reprobe can never reach another project's members.
        let arg: ReprobeArg =
            serde_json::from_value(json!({"kind":"selection","assetIds":[11,22,33],"expectedCount":3}))
                .expect("selection arg parses");
        match reprobe_scope_for(7, arg) {
            ReprobeScope::Selection {
                selections,
                expected_count,
            } => {
                assert_eq!(expected_count, 3);
                assert_eq!(selections.len(), 3);
                assert!(selections.iter().all(|s| s.project_id == 7));
                assert_eq!(
                    selections.iter().map(|s| s.asset_id).collect::<Vec<_>>(),
                    vec![11, 22, 33]
                );
            }
            _ => panic!("expected a selection scope"),
        }
    }

    #[test]
    fn query_scope_overwrites_any_inner_project_id_with_the_host_project() {
        // Even if a crafted inner query names another project, the Host project
        // wins. reprobe_members_on then rejects a query whose project id differs.
        let arg: ReprobeArg = serde_json::from_value(
            json!({"kind":"query","query":{"projectId":999,"search":"example.com"},"expectedCount":5}),
        )
        .expect("query arg parses");
        match reprobe_scope_for(7, arg) {
            ReprobeScope::Query {
                query,
                expected_count,
            } => {
                assert_eq!(expected_count, 5);
                assert_eq!(query.project_id, Some(7));
                assert_eq!(query.search, "example.com");
            }
            _ => panic!("expected a query scope"),
        }
    }

    #[test]
    fn a_model_supplied_project_id_at_the_scope_level_is_rejected() {
        // deny_unknown_fields: the scope object has no project id field, so the
        // model cannot smuggle a binding past the forced one.
        let parsed: Result<ReprobeRequest, _> = serde_json::from_value(json!({
            "scope":{"kind":"selection","assetIds":[1],"expectedCount":1,"projectId":2}
        }));
        assert!(parsed.is_err(), "an extra projectId must be refused");
    }

    #[test]
    fn an_empty_query_is_allowed_and_still_bound_to_the_host_project() {
        // "reprobe everything in this project": query omitted entirely.
        let arg: ReprobeArg =
            serde_json::from_value(json!({"kind":"query","expectedCount":42})).expect("parses");
        match reprobe_scope_for(3, arg) {
            ReprobeScope::Query { query, .. } => assert_eq!(query.project_id, Some(3)),
            _ => panic!("expected a query scope"),
        }
    }
}

