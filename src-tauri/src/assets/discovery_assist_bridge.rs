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
        _ => return Err("discovery_action_invalid".into()),
    };
    value["projectId"] = json!(p);
    value["hunterStarted"] = json!(
        value["pipeline"]["phase"] == "hunter"
            || value["pipeline"]["hunter"]["pages"].as_i64().unwrap_or(0) > 0
    );
    Ok(value)
}

