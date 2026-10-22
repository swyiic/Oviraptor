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

#[cfg(test)]
mod tests {
    use super::*;
    struct Temporary(std::path::PathBuf);
    impl Temporary { fn path(&self) -> &std::path::Path { &self.0 } }
    impl Drop for Temporary { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
    fn fixture() -> (Temporary, Manager) {
        let dir = Temporary(std::env::temp_dir().join(format!("ovi-chat-owner-{}",uuid::Uuid::new_v4())));
        std::fs::create_dir_all(dir.path()).unwrap();
        let path = crate::db::initialize(dir.path()).unwrap();
        let c = db::open(&path).unwrap();
        c.execute("INSERT OR IGNORE INTO projects(id,name,status) VALUES(91,'Chat project','active'),(92,'Other','active')", []).unwrap();
        let enrichment = enrichment::Manager::new(&path).unwrap();
        let manager = Manager::new(&path, enrichment).unwrap();
        (dir, manager)
    }
    #[test]
    fn chat_owner_rejects_project_injection_and_stale_review_before_execution() {
        let (_dir, m) = fixture();
        assert_eq!(dispatch(&m, 91, "status", json!({"projectId":92})).unwrap_err(), "discovery_arguments_invalid");
        assert_eq!(dispatch(&m, 99999, "status", json!({})).unwrap_err(), "discovery_project_unavailable");
        assert_eq!(dispatch(&m, 91, "confirm", json!({"runId":"invented","proposalToken":"invented","hosts":["outside.test"],"expectedRoots":[],"userConfirmed":true})).unwrap_err(), "discovery_review_changed");
        assert_eq!(roots(&db::open(&m.path).unwrap(),91).unwrap(), Vec::<String>::new());
        // This fixture never starts enrichment. Its global owner registry may
        // contain other parallel tests' tasks; do not stop those owners.
        m.shutdown();
    }
    #[test]
    fn empty_project_stops_before_model_or_free_target_work_and_reports_same_saved_state() {
        let (_dir, m) = fixture();
        let first = dispatch(&m,91,"start",json!({"organization":"明确目标单位","includeIsolated":false})).unwrap();
        assert_eq!(first["phase"], "empty"); assert_eq!(first["enabled"],false);
        assert_eq!(first["hunterStarted"],false); assert_eq!(first["projectId"],91);
        let saved = dispatch(&m,91,"status",json!({})).unwrap();
        assert_eq!(saved["organization"], "明确目标单位"); assert_eq!(saved["includeIsolated"],false);
        let other = dispatch(&m,92,"status",json!({})).unwrap();
        assert_eq!(other["organization"], "");
        let c=db::open(&m.path).unwrap();
        assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_seed_review_model_receipts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_enrichment_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        m.shutdown();
    }
    #[test]
    fn human_seed_preparation_is_project_bound_and_only_builds_derived_local_indexes() {
        let (_dir, m) = fixture();
        let c = db::open(&m.path).unwrap();
        c.execute("INSERT INTO assets(asset_key,company,host,link,extra_json) VALUES('seed-prepare-fixture','untrusted collection label','seed.target.test','https://seed.target.test/','{}')", []).unwrap();
        let asset = c.last_insert_rowid();
        c.execute("INSERT INTO project_assets(project_id,asset_id) VALUES(91,?1)", [asset]).unwrap();
        crate::asset_discovery::ingest_observations(&c, 91, &[(asset,"seed.target.test".into(),"https://seed.target.test/".into(),"{}".into())]).unwrap();
        assert_eq!(dispatch(&m, 91, "prepare_seeds", json!({"projectId":92})).unwrap_err(), "discovery_arguments_invalid");
        assert_eq!(dispatch(&m, 91, "prepare_seeds", json!({"organization":"unrequested target"})).unwrap_err(), "discovery_arguments_invalid");
        assert_eq!(dispatch(&m, 99999, "prepare_seeds", json!({})).unwrap_err(), "discovery_project_unavailable");
        let prepared = dispatch(&m, 91, "prepare_seeds", json!({})).unwrap();
        assert_eq!(prepared["projectId"], 91);
        assert_eq!(prepared["ready"], true);
        assert_eq!(prepared["hunterStarted"], false);
        assert!(prepared["items"].as_array().unwrap().iter().any(|seed| seed["value"] == "seed.target.test"));
        assert_eq!(c.query_row("SELECT COUNT(*) FROM asset_project_seed_state WHERE project_id=92", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(c.query_row("SELECT extra_json FROM assets WHERE id=?1", [asset], |r| r.get::<_,String>(0)).unwrap(), "{}");
        assert!(roots(&c,91).unwrap().is_empty());
        for table in ["asset_seed_reviews","asset_seed_review_model_receipts","asset_enrichment_runs"] {
            assert_eq!(c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_,i64>(0)).unwrap(), 0);
        }
        m.shutdown();
    }
}
