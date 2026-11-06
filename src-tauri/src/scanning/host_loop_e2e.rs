//! Host <-> Nest end to end: the real desktop bridge and database on this side,
//! the real Host store in a separate Node process with its own database. Each
//! Host phase is a new process, so the second phase also covers a restart.
use serde_json::Value;
use std::{path::Path, process::Command};

fn host_phase(phase: &str, host_dir: &Path, origin: &str, token: &str) -> Value {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packages/agent-host/test/host-nest-loop.e2e.mjs");
    let output = Command::new("node")
        .arg(script)
        .env("HOST_DIR", host_dir)
        .env("PHASE", phase)
        .env("OVIRAPTOR_SCAN_BRIDGE_ORIGIN", origin)
        .env("OVIRAPTOR_SCAN_BRIDGE_TOKEN", token)
        .output()
        .expect("node must be installed to run the Host half");
    assert!(output.status.success(), "host phase {phase} failed: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn a_result_returns_once_to_the_bound_host_conversation_across_restarts() {
    let root = std::env::temp_dir().join(format!("oviraptor-e2e-{}", uuid::Uuid::new_v4()));
    let (desktop_dir, host_dir) = (root.join("desktop"), root.join("host"));
    std::fs::create_dir_all(&host_dir).unwrap();
    let db_path = crate::db::initialize(&desktop_dir).unwrap();
    {
        let c = crate::db::open(&db_path).unwrap();
        c.execute_batch(
            "INSERT INTO projects(id,name) VALUES(1,'one'),(2,'two');
             INSERT INTO sentinel_scans(id,project_id,status,attempt_count,scan_type) VALUES('scan-1',1,'completed',1,'web');
             INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(1,'scan-1','示例','https://a.example/','completed');
             INSERT INTO sentinel_validations(scan_id,url,finding_key,verdict) VALUES('scan-1','https://a.example/','k1','confirmed');",
        ).unwrap();
    }
    let manager = crate::asset_discovery::certificate_tasks::Manager::new(&db_path).unwrap();
    let bridge = crate::certificate_bridge::Bridge::start_for_database(manager, db_path.clone(), desktop_dir.clone()).unwrap();

    let first = host_phase("bind", &host_dir, &bridge.origin, &bridge.token);
    assert_eq!(first["forgedRejected"], true, "{first}");
    assert_eq!(first["crossProjectRejected"], true, "{first}");
    assert_eq!(first["messages"], 1, "the settled attempt reached the bound conversation: {first}");
    assert_eq!(first["otherMessages"], 0, "{first}");
    {
        let c = crate::db::open(&db_path).unwrap();
        let binding: String = c.query_row("SELECT conversation_id FROM nest_session_task_bindings WHERE task_id='scan-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(binding, format!("host:{}", first["conversationId"].as_str().unwrap()));
        let pending: i64 = c.query_row("SELECT COUNT(*) FROM nest_host_outbox WHERE state='pending'", [], |r| r.get(0)).unwrap();
        assert_eq!(pending, 0, "the Host acknowledged what it stored");
        // A new attempt settles while the Host is down.
        c.execute("UPDATE sentinel_scans SET attempt_count=2, status='paused' WHERE id='scan-1'", []).unwrap();
    }

    let second = host_phase("restart", &host_dir, &bridge.origin, &bridge.token);
    assert_eq!(second["conversationId"], first["conversationId"], "the binding survived the restart: {second}");
    assert_eq!(second["before"], 1, "{second}");
    assert_eq!(second["messages"], 2, "one new event, nothing replayed twice: {second}");
    assert!(second["lastText"].as_str().unwrap().contains("第 2 次尝试"), "{second}");
    drop(bridge);
    let _ = std::fs::remove_dir_all(&root);
}
