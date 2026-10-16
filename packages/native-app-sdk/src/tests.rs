use super::*;
use crate::{
    driver::{read_frame, run_loop, Driver, Frame},
    manifest::Manifest,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Cursor, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn operator_nullable_values_keep_null_semantics_and_nested_closed_bounds() {
    let schema = json!({"type":"object","properties":{
        "note":{"type":["string","null"],"minLength":2,"maxLength":4},
        "selection":{"type":["object","null"],"properties":{"id":{"type":["integer","null"],"minimum":1}},"additionalProperties":false},
        "ids":{"type":["array","null"],"items":{"type":["integer","null"],"minimum":1},"maxItems":3},
        "choice":{"type":["string","null"],"enum":["yes",null]}
    },"additionalProperties":false});
    for value in [
        json!({"note":null,"selection":null,"ids":null,"choice":null}),
        json!({"note":"yes","selection":{"id":null},"ids":[1,null],"choice":"yes"}),
    ] {
        crate::schema::arguments(&schema, &value, true).unwrap();
    }
    for value in [
        json!({"note":"x"}),
        json!({"selection":{"extra":1}}),
        json!({"selection":{"id":0}}),
        json!({"ids":[1,2,3,4]}),
        json!({"choice":"no"}),
    ] {
        assert_eq!(
            crate::schema::arguments(&schema, &value, true).unwrap_err(),
            "app_arguments_invalid"
        );
    }
}

#[test]
fn nullable_schemas_reject_open_or_general_unions_and_leave_model_contracts_unchanged() {
    for kind in [
        json!(["string", "number"]),
        json!(["null", "null"]),
        json!(["string", "null", "integer"]),
        json!(["unknown", "null"]),
        json!([]),
    ] {
        let schema = json!({"type":"object","properties":{"value":{"type":kind}},"additionalProperties":false});
        assert_eq!(
            crate::schema::validate(&schema, true, 0).unwrap_err(),
            "app_schema_invalid"
        );
    }
    let nullable = json!({"type":"object","properties":{"value":{"type":["integer","null"]}},"additionalProperties":false});
    assert_eq!(
        crate::schema::arguments(&nullable, &json!({"value":null}), false).unwrap_err(),
        "app_schema_invalid"
    );
    let open = json!({"type":"object","properties":{"value":{"type":["object","null"],"properties":{},"additionalProperties":true}},"additionalProperties":false});
    assert_eq!(
        crate::schema::validate(&open, true, 0).unwrap_err(),
        "app_schema_invalid"
    );
}

#[test]
fn artifact_delivery_accepts_only_the_explicit_project_read_contract() {
    let mut f = Fixture::new();
    let action = json!({"name":"read_artifact","description":"Read an owned artifact chunk","permission":"project.read","scope":"project","delivery":"artifact-read","inputSchema":crate::manifest::artifact_read_schema()});
    f.manifest["actions"].as_array_mut().unwrap().push(action);
    assert!(Manifest::parse(&serde_json::to_vec(&f.manifest).unwrap()).is_ok());
}

#[test]
fn artifact_delivery_rejects_global_mutating_open_schema_or_model_surfaces() {
    let f = Fixture::new();
    let action = json!({"name":"read_artifact","description":"Read an owned artifact chunk","permission":"project.read","scope":"project","delivery":"artifact-read","inputSchema":crate::manifest::artifact_read_schema()});
    for mode in ["global", "null", "unknown", "open", "tool"] {
        let mut manifest = f.manifest.clone();
        let mut invalid = action.clone();
        match mode {
            "global" => invalid["scope"] = json!("application"),
            "null" => invalid["delivery"] = Value::Null,
            "unknown" => invalid["delivery"] = json!("execute"),
            "open" => invalid["inputSchema"]["properties"]["path"] = json!({"type":"string"}),
            "tool" => {
                invalid.as_object_mut().unwrap().remove("scope");
            }
            _ => unreachable!(),
        }
        manifest[if mode == "tool" { "tools" } else { "actions" }]
            .as_array_mut()
            .unwrap()
            .push(invalid);
        assert_eq!(
            Manifest::parse(&serde_json::to_vec(&manifest).unwrap())
                .err()
                .unwrap(),
            "app_manifest_invalid",
            "{mode}"
        );
    }
}
struct Fixture {
    root: PathBuf,
    data: PathBuf,
    package: PathBuf,
    manifest: Value,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-native-sdk-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let data = root.join("data");
        let package = root.join("package");
        fs::create_dir(&data).unwrap();
        fs::create_dir(&package).unwrap();
        fs::create_dir(package.join("bin")).unwrap();
        fs::write(package.join("bin/app"), b"synthetic-executable").unwrap();
        let empty = json!({"type":"object","properties":{},"additionalProperties":false});
        let text = json!({"type":"object","properties":{"value":{"type":"string","maxLength":16384}},"required":["value"],"additionalProperties":false});
        let permissions = json!([{"id":"project.read","description":"Read selected project"},{"id":"application.admin","description":"Explicit application administration"}]);
        let tools=[("echo",text.clone()),("big_tool",empty.clone()),("return_scalar",empty.clone()),("return_null",empty.clone())].into_iter().map(|(name,input)|json!({"name":name,"description":"Fixture tool","inputSchema":input,"permission":"project.read"})).collect::<Vec<_>>();
        let actions=[("act_project","project",empty.clone()),("act_app","application",empty.clone()),("scalar","project",empty.clone()),("null_value","project",empty.clone()),("big_action","application",empty.clone()),("batch","project",json!({"type":"object","properties":{"values":{"type":"array","items":{"type":"integer"},"maxItems":16384}},"required":["values"],"additionalProperties":false}))].into_iter().map(|(name,scope,input)|json!({"name":name,"description":"Fixture action","inputSchema":input,"permission":if scope=="application"{"application.admin"}else{"project.read"},"scope":scope})).collect::<Vec<_>>();
        let platform = match std::env::consts::OS {
            "macos" => "darwin",
            "windows" => "win32",
            v => v,
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            v => v,
        };
        let manifest = json!({"schemaVersion":2,"id":"org.oviraptor.fixture","version":"2.0.0","hostApi":"^2.0.0","entry":"bin/app","dataSchemaVersion":2,"runtime":{"kind":"native","platform":platform,"arch":arch},"tools":tools,"actions":actions,"pages":[],"permissions":permissions,"files":[{"path":"bin/app","sha256":format!("{:x}",Sha256::digest(b"synthetic-executable"))}],"lifecycle":{"start":true,"stop":true}});
        let f = Self {
            root,
            data,
            package,
            manifest,
        };
        f.save();
        f
    }
    fn save(&self) {
        fs::write(
            self.package.join("manifest.json"),
            serde_json::to_vec_pretty(&self.manifest).unwrap(),
        )
        .unwrap();
    }
    fn driver(&self) -> Driver {
        let bytes = serde_json::to_vec_pretty(&self.manifest).unwrap();
        let m = Manifest::parse(&bytes).unwrap();
        let b = Bootstrap {
            data_dir: self.data.clone(),
            package_dir: self.package.clone(),
            app_id: m.id.clone(),
            data_schema_version: m.data_schema_version,
            events: EventSink::silent(),
        };
        Driver::new(b, m, format!("{:x}", Sha256::digest(bytes)), factory)
    }
    fn start_params(&self) -> Value {
        json!({"dataDir":self.data,"packageDir":self.package,"context":{"hostApiVersion":"2.0.0","appId":"org.oviraptor.fixture"}})
    }
    fn started(&self) -> Driver {
        let mut d = self.driver();
        assert!(
            request(&mut d, "describe", json!({"hostApiVersion":"2.0.0"}))
                .0
                .get("result")
                .is_some()
        );
        assert!(request(&mut d, "lifecycle/start", self.start_params())
            .0
            .get("result")
            .is_some());
        d
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct App {
    data: PathBuf,
    busy: u32,
    uncertain: u32,
    stop_busy: bool,
    large_status: bool,
}
impl NativeApplication for App {
    fn data_schema_version(&self) -> u64 {
        2
    }
    fn call(
        &mut self,
        surface: CallSurface,
        name: &str,
        args: Value,
        ctx: &NativeCallContext,
    ) -> Result<Value, String> {
        fs::write(self.data.join("called"), name).unwrap();
        match name {
            "big_tool" => Ok(json!({"value":"\"".repeat(20_000)})),
            "big_action" => Ok(json!("x".repeat(MAX_ACTION_FRAME_BYTES))),
            "scalar" | "return_scalar" => Ok(json!(7)),
            "null_value" | "return_null" => Ok(Value::Null),
            _ => Ok(
                json!({"surface":format!("{surface:?}"),"projectId":ctx.host_bound_project_id,"arguments":args}),
            ),
        }
    }
    fn status(&mut self) -> Result<Value, String> {
        if self.large_status {
            return Ok(json!({"busy":false,"detail":"x".repeat(MAX_STATUS_FRAME_BYTES)}));
        }
        let p = self.data.join("status-polls");
        let n = fs::read_to_string(&p)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        fs::write(p, n.to_string()).unwrap();
        if self.busy > 0 {
            self.busy -= 1;
            return Ok(json!({"busy":true}));
        }
        if self.uncertain > 0 {
            self.uncertain -= 1;
            return Ok(json!({"busy":false,"uncertain":true}));
        }
        Ok(json!({"busy":false,"uncertain":false}))
    }
    fn stop(&mut self) -> Result<Value, String> {
        fs::write(self.data.join("stopped"), b"true").unwrap();
        Ok(json!({"busy":self.stop_busy,"stopped":!self.stop_busy}))
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = fs::write(self.data.join("dropped"), b"true");
    }
}
fn factory(b: &Bootstrap) -> Result<Box<dyn NativeApplication>, String> {
    let p = b.data_dir.join("factory-count");
    let n = fs::read_to_string(&p)
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0)
        + 1;
    fs::write(p, n.to_string()).unwrap();
    let mode = fs::read_to_string(b.data_dir.join("mode")).unwrap_or_default();
    if mode == "factory_error" {
        return Err("app_factory_failed".into());
    }
    if mode == "factory_path_error" {
        return Err("sqlite /private/fixture token=synthetic-private-key".into());
    }
    if mode == "factory_uppercase_error" {
        return Err("ASSET_CAS_CONFLICT".into());
    }
    Ok(Box::new(App {
        data: b.data_dir.clone(),
        busy: if mode == "busy" { 2 } else { 0 },
        uncertain: if mode == "uncertain" { 1 } else { 0 },
        stop_busy: mode == "stop_busy",
        large_status: mode == "large_status",
    }))
}
fn request(d: &mut Driver, method: &str, params: Value) -> (Value, bool) {
    let (bytes, stop) = d.handle(
        &serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .unwrap(),
    );
    (serde_json::from_slice(&bytes).unwrap(), stop)
}
fn context(permission: &str) -> Value {
    json!({"hostBoundProjectId":91,"grantRevision":7,"operationId":"fixture-operation","appRevision":3,"permissions":["project.read","application.admin"],"permission":permission,"project":{"id":91,"name":"Fixture project","status":"active"}})
}
fn call(d: &mut Driver, method: &str, name: &str, args: Value, ctx: Value) -> Value {
    request(
        d,
        method,
        json!({"name":name,"arguments":args,"context":ctx}),
    )
    .0
}

#[test]
fn describe_exact_manifest_bytes_and_no_factory_before_legal_start() {
    let f = Fixture::new();
    let mut d = f.driver();
    let description = request(&mut d, "describe", json!({"hostApiVersion":"2.0.0"})).0;
    assert_eq!(description["result"]["protocolVersion"], 2);
    assert_eq!(
        description["result"]["manifestSha256"],
        format!(
            "{:x}",
            Sha256::digest(fs::read(f.package.join("manifest.json")).unwrap())
        )
    );
    assert!(!f.data.join("factory-count").exists());
    let mut bad = f.start_params();
    bad["dataDir"] = json!(f.root);
    assert!(request(&mut d, "lifecycle/start", bad)
        .0
        .get("error")
        .is_some());
    assert!(!f.data.join("factory-count").exists());
}
#[test]
fn start_without_version_handshake_and_calls_before_start_never_create_domain() {
    let f = Fixture::new();
    let mut d = f.driver();
    assert!(request(&mut d, "lifecycle/start", f.start_params())
        .0
        .get("error")
        .is_some());
    assert!(call(
        &mut d,
        "tool/call",
        "echo",
        json!({"value":"hi"}),
        context("project.read")
    )
    .get("error")
    .is_some());
    assert!(!f.data.join("factory-count").exists());
    assert!(
        request(&mut d, "describe", json!({"hostApiVersion":"1.0.0"}))
            .0
            .get("error")
            .is_some()
    );
}
#[test]
fn successful_start_is_idempotent_but_factory_failure_is_not_replayed() {
    let f = Fixture::new();
    let mut d = f.started();
    assert_eq!(
        request(&mut d, "lifecycle/start", f.start_params()).0["result"]["dataSchemaVersion"],
        2
    );
    assert_eq!(
        fs::read_to_string(f.data.join("factory-count")).unwrap(),
        "1"
    );
    let g = Fixture::new();
    fs::write(g.data.join("mode"), "factory_error").unwrap();
    let mut e = g.driver();
    request(&mut e, "describe", json!({"hostApiVersion":"2.0.0"}));
    for _ in 0..2 {
        assert!(request(&mut e, "lifecycle/start", g.start_params())
            .0
            .get("error")
            .is_some());
    }
    assert_eq!(
        fs::read_to_string(g.data.join("factory-count")).unwrap(),
        "1"
    );
}
#[test]
fn actual_domain_schema_mismatch_never_acknowledges_manifest_version() {
    let mut f = Fixture::new();
    f.manifest["dataSchemaVersion"] = json!(999);
    let mut d = f.driver();
    request(&mut d, "describe", json!({"hostApiVersion":"2.0.0"}));
    let result = request(&mut d, "lifecycle/start", f.start_params()).0;
    assert_eq!(result["error"]["data"]["code"], "app_data_schema_mismatch");
    assert!(call(
        &mut d,
        "tool/call",
        "return_null",
        json!({}),
        context("project.read")
    )
    .get("error")
    .is_some());
}

#[test]
fn domain_error_messages_are_not_exposed_but_uppercase_cas_codes_survive() {
    for (mode, code) in [
        ("factory_path_error", "app_operation_failed"),
        ("factory_uppercase_error", "ASSET_CAS_CONFLICT"),
    ] {
        let f = Fixture::new();
        fs::write(f.data.join("mode"), mode).unwrap();
        let mut d = f.driver();
        request(&mut d, "describe", json!({"hostApiVersion":"2.0.0"}));
        let value = request(&mut d, "lifecycle/start", f.start_params()).0;
        assert_eq!(value["error"]["data"]["code"], code);
        let wire = serde_json::to_string(&value).unwrap();
        assert!(!wire.contains("/private/fixture"));
        assert!(!wire.contains("synthetic-private-key"));
        assert!(wire.len() < 512);
    }
}
#[test]
fn context_rejects_cross_project_revisions_archived_unknown_and_wrong_permission() {
    let f = Fixture::new();
    let mut d = f.started();
    let mut samples = Vec::new();
    for (key, value) in [
        ("hostBoundProjectId", json!(92)),
        ("appRevision", json!(-1)),
        ("grantRevision", json!(-1)),
        ("permission", json!("application.admin")),
        ("operationId", json!("bad\nvalue")),
        ("unknown", json!(1)),
    ] {
        let mut c = context("project.read");
        c[key] = value;
        samples.push(c);
    }
    let mut archived = context("project.read");
    archived["project"]["status"] = json!("archived");
    samples.push(archived);
    let mut extra = context("project.read");
    extra["project"]["extra"] = json!(true);
    samples.push(extra);
    let mut ungranted = context("project.read");
    ungranted["permissions"] = json!([]);
    samples.push(ungranted);
    for c in samples {
        assert_eq!(
            call(&mut d, "tool/call", "echo", json!({"value":"hi"}), c)["error"]["data"]["code"],
            "app_context_invalid"
        );
    }
    assert!(!f.data.join("called").exists());
}
#[test]
fn tool_context_cannot_choose_action_surface_and_manifest_decides_scope() {
    let f = Fixture::new();
    let mut d = f.started();
    assert!(call(
        &mut d,
        "tool/call",
        "act_app",
        json!({}),
        context("application.admin")
    )
    .get("error")
    .is_some());
    assert_eq!(
        call(
            &mut d,
            "action/call",
            "act_project",
            json!({}),
            context("project.read")
        )["result"]["surface"],
        "ProjectAction"
    );
    assert_eq!(
        call(
            &mut d,
            "action/call",
            "act_app",
            json!({}),
            context("application.admin")
        )["result"]["surface"],
        "ApplicationAction"
    );
    let mut args = json!({"value":"x"});
    args["surface"] = json!("ApplicationAction");
    assert!(
        call(&mut d, "tool/call", "echo", args, context("project.read"))
            .get("error")
            .is_some()
    );
}
#[test]
fn action_numbers_and_null_remain_raw_while_tool_values_get_mcp_result() {
    let f = Fixture::new();
    let mut d = f.started();
    assert_eq!(
        call(
            &mut d,
            "action/call",
            "scalar",
            json!({}),
            context("project.read")
        )["result"],
        7
    );
    assert!(call(
        &mut d,
        "action/call",
        "null_value",
        json!({}),
        context("project.read")
    )["result"]
        .is_null());
    let t = call(
        &mut d,
        "tool/call",
        "return_scalar",
        json!({}),
        context("project.read"),
    );
    assert_eq!(t["result"]["content"][0]["text"], "7");
    assert!(t["result"].get("structuredContent").is_none());
    let obj = call(
        &mut d,
        "tool/call",
        "echo",
        json!({"value":"hi"}),
        context("project.read"),
    );
    assert_eq!(obj["result"]["structuredContent"]["projectId"], 91);
}
#[test]
fn full_tool_and_action_wire_budgets_include_wrapper_and_json_escaping() {
    let f = Fixture::new();
    let mut d = f.started();
    for (method, name, permission, max) in [
        (
            "tool/call",
            "big_tool",
            "project.read",
            MAX_TOOL_FRAME_BYTES,
        ),
        (
            "action/call",
            "big_action",
            "application.admin",
            MAX_ACTION_FRAME_BYTES,
        ),
    ] {
        let (bytes,_)=d.handle(&serde_json::to_vec(&json!({"jsonrpc":"2.0","id":"界".repeat(40),"method":method,"params":{"name":name,"arguments":{},"context":context(permission)}})).unwrap());
        assert!(bytes.len() <= max);
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap()["error"]["data"]["code"],
            "app_result_too_large"
        );
    }
}
#[test]
fn oversized_non_scalar_and_control_ids_are_small_null_id_errors() {
    let f = Fixture::new();
    let mut d = f.driver();
    for id in [
        json!("界".repeat(30000)),
        json!({"secret":"x".repeat(80000)}),
        json!([1]),
        json!(true),
        json!(1.5),
        json!(9007199254740992u64),
        json!("bad\nID"),
    ] {
        let (b,stop)=d.handle(&serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":"describe","params":{"hostApiVersion":"2.0.0"}})).unwrap());
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert!(v["id"].is_null());
        assert!(b.len() < 512);
        assert!(!stop);
    }
    assert!(!f.data.join("factory-count").exists());
}
#[test]
fn finite_schema_enums_numeric_bounds_and_unknown_fields_are_enforced() {
    let s = json!({"type":"object","properties":{"status":{"type":"string","enum":["known","unknown"]},"n":{"type":"integer","minimum":0,"maximum":3}},"required":["status","n"],"additionalProperties":false});
    assert!(schema::arguments(&s, &json!({"status":"known","n":3}), false).is_ok());
    for v in [
        json!({"status":"new","n":1}),
        json!({"status":"known","n":4}),
        json!({"status":"known","n":1.5}),
        json!({"status":"known","n":1,"projectId":92}),
        json!({"n":1}),
    ] {
        assert!(schema::arguments(&s, &v, false).is_err());
    }
    let mut bad = s.clone();
    bad["anyOf"] = json!([]);
    assert!(schema::validate(&bad, false, 0).is_err());
}
#[test]
fn tool_and_action_schema_limits_are_separate_and_finite() {
    let f = Fixture::new();
    let s = &f.manifest["actions"][5]["inputSchema"];
    assert!(schema::validate(s, true, 0).is_ok());
    assert!(schema::validate(s, false, 0).is_err());
    assert!(schema::arguments(s, &json!({"values":vec![1;16384]}), true).is_ok());
    assert!(schema::arguments(s, &json!({"values":vec![1;16385]}), true).is_err());
    let text = json!({"type":"object","properties":{"text":{"type":"string","maxLength":1048576}},"additionalProperties":false});
    assert!(schema::arguments(&text, &json!({"text":"x".repeat(1048500)}), true).is_ok());
    assert!(schema::arguments(&text, &json!({"text":"界".repeat(400000)}), true).is_err());
    assert!(schema::validate(&text, false, 0).is_err());
}
#[test]
fn deep_schema_and_unbounded_or_dangerous_properties_are_rejected() {
    let mut s = json!({"type":"boolean"});
    for _ in 0..10 {
        s = json!({"type":"array","items":s,"maxItems":1});
    }
    assert!(schema::validate(&s, true, 0).is_err());
    for s in [
        json!({"type":"array","items":{"type":"string"}}),
        json!({"type":"object","properties":{"constructor":{"type":"boolean"}},"additionalProperties":false}),
        json!({"type":"object","properties":{},"additionalProperties":true}),
    ] {
        assert!(schema::validate(&s, true, 0).is_err());
    }
}
#[test]
fn busy_and_uncertain_stop_do_not_call_stop_or_drop_application() {
    for mode in ["busy", "uncertain"] {
        let f = Fixture::new();
        fs::write(f.data.join("mode"), mode).unwrap();
        let mut d = f.started();
        let (v, exit) = request(&mut d, "lifecycle/stop", json!({}));
        assert!(v.get("error").is_some());
        assert!(!exit);
        assert!(!f.data.join("stopped").exists());
        assert!(!f.data.join("dropped").exists());
    }
}

#[test]
fn status_requires_started_state_and_empty_params_without_creating_domain() {
    let f = Fixture::new();
    let mut d = f.driver();
    assert!(request(&mut d, "lifecycle/status", json!({}))
        .0
        .get("error")
        .is_some());
    assert!(!f.data.join("factory-count").exists());
    let mut d = f.started();
    assert!(request(&mut d, "lifecycle/status", json!({"projectId":92}))
        .0
        .get("error")
        .is_some());
    assert!(!f.data.join("status-polls").exists());
}

#[test]
fn status_preserves_real_background_busy_and_uncertainty_without_inventing_idle() {
    for mode in ["busy", "uncertain"] {
        let f = Fixture::new();
        fs::write(f.data.join("mode"), mode).unwrap();
        let mut d = f.started();
        let (v, exit) = request(&mut d, "lifecycle/status", json!({}));
        assert!(!exit);
        if mode == "busy" {
            assert_eq!(v["result"]["busy"], true);
        } else {
            assert_eq!(v["result"]["busy"], false);
            assert_eq!(v["result"]["uncertain"], true);
        }
        assert!(!f.data.join("called").exists());
        assert!(!f.data.join("stopped").exists());
        assert!(!f.data.join("dropped").exists());
    }
}

#[test]
fn status_complete_wire_is_bounded_at_four_kib_without_truncating_domain_data() {
    let f = Fixture::new();
    fs::write(f.data.join("mode"), "large_status").unwrap();
    let mut d = f.started();
    let (bytes, exit) = d.handle(
        &serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":"界".repeat(40),"method":"lifecycle/status","params":{}}),
        )
        .unwrap(),
    );
    assert!(!exit);
    assert!(bytes.len() <= MAX_STATUS_FRAME_BYTES);
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["error"]["data"]["code"],
        "app_result_too_large"
    );
    assert!(!f.data.join("stopped").exists());
}
#[test]
fn stop_acknowledgement_is_required_before_driver_exit() {
    let f = Fixture::new();
    let mut d = f.started();
    let (v, exit) = request(&mut d, "lifecycle/stop", json!({}));
    assert_eq!(v["result"]["busy"], false);
    assert!(exit);
    let g = Fixture::new();
    fs::write(g.data.join("mode"), "stop_busy").unwrap();
    let mut e = g.started();
    let (v, exit) = request(&mut e, "lifecycle/stop", json!({}));
    assert!(v.get("error").is_some());
    assert!(!exit);
}
#[test]
fn eof_retains_busy_owner_until_actual_idle_without_stop_or_cancel() {
    let f = Fixture::new();
    fs::write(f.data.join("mode"), "busy").unwrap();
    let mut d = f.started();
    let mut pauses = 0;
    d.wait_idle(|| {
        pauses += 1;
        assert!(!f.data.join("dropped").exists());
    });
    assert_eq!(pauses, 2);
    assert_eq!(
        fs::read_to_string(f.data.join("status-polls")).unwrap(),
        "3"
    );
    assert!(!f.data.join("stopped").exists());
    assert!(!f.data.join("dropped").exists());
}
#[test]
fn output_failure_waits_for_idle_instead_of_dropping_live_owner() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "synthetic"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let f = Fixture::new();
    fs::write(f.data.join("mode"), "busy").unwrap();
    let mut d = f.started();
    let result = run_loop(
        &mut Cursor::new(b"bad json\n"),
        &mut d,
        &EventSink::writer(Box::new(Broken)),
    );
    assert_eq!(result.unwrap_err(), "app_transport_output_failed");
    assert_eq!(
        fs::read_to_string(f.data.join("status-polls")).unwrap(),
        "3"
    );
    assert!(!f.data.join("dropped").exists());
    assert!(!f.data.join("stopped").exists());
}
#[test]
fn overflow_is_drained_boundedly_and_next_line_survives() {
    let mut data = vec![b'x'; MAX_INPUT_FRAME_BYTES + 100];
    data.extend_from_slice(b"\n{}\n");
    let mut input = io::BufReader::with_capacity(37, Cursor::new(data));
    assert!(matches!(read_frame(&mut input).unwrap(), Frame::TooLarge));
    match read_frame(&mut input).unwrap() {
        Frame::Line(v) => assert_eq!(v, b"{}"),
        _ => panic!("next frame lost"),
    };
    assert!(matches!(read_frame(&mut input).unwrap(), Frame::Eof));
}
#[test]
fn partial_eof_and_invalid_utf8_never_invoke_factory() {
    let f = Fixture::new();
    let mut d = f.driver();
    assert!(matches!(
        read_frame(&mut Cursor::new(b"{\"id\":1}")).unwrap(),
        Frame::Partial
    ));
    let (b, _) = d.handle(&[0xff, 0xfe]);
    assert!(serde_json::from_slice::<Value>(&b)
        .unwrap()
        .get("error")
        .is_some());
    assert!(!f.data.join("factory-count").exists());
}
#[test]
fn manifest_identity_hashes_exact_bytes_and_rejects_tamper_extra_or_unsafe_entry() {
    let mut f = Fixture::new();
    let (_, hash) = manifest::load(&f.package).unwrap();
    assert_eq!(
        hash,
        format!(
            "{:x}",
            Sha256::digest(fs::read(f.package.join("manifest.json")).unwrap())
        )
    );
    fs::write(f.package.join("bin/app"), "tampered").unwrap();
    assert!(manifest::load(&f.package).is_err());
    fs::write(f.package.join("bin/app"), b"synthetic-executable").unwrap();
    fs::write(f.package.join("extra"), b"extra").unwrap();
    assert!(manifest::load(&f.package).is_err());
    fs::remove_file(f.package.join("extra")).unwrap();
    f.manifest["entry"] = json!("../app");
    assert!(Manifest::parse(&serde_json::to_vec(&f.manifest).unwrap()).is_err());
}
#[test]
fn manifest_v1_wrong_api_tool_scope_unknown_fields_and_duplicate_ids_are_refused() {
    let f = Fixture::new();
    for (key, val) in [
        ("schemaVersion", json!(1)),
        ("hostApi", json!("^1.0.0")),
        ("unknown", json!(true)),
    ] {
        let mut v = f.manifest.clone();
        v[key] = val;
        assert!(Manifest::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    let mut v = f.manifest.clone();
    v["tools"][0]["scope"] = json!("application");
    assert!(Manifest::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v = f.manifest.clone();
    v["actions"][1]["name"] = v["actions"][0]["name"].clone();
    assert!(Manifest::parse(&serde_json::to_vec(&v).unwrap()).is_err());
}
#[cfg(unix)]
#[test]
fn package_symlinks_and_hardlinks_are_rejected_before_factory() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::remove_file(f.package.join("bin/app")).unwrap();
    symlink(f.root.join("missing"), f.package.join("bin/app")).unwrap();
    assert!(manifest::load(&f.package).is_err());
    fs::remove_file(f.package.join("bin/app")).unwrap();
    fs::write(f.root.join("source"), b"synthetic-executable").unwrap();
    fs::hard_link(f.root.join("source"), f.package.join("bin/app")).unwrap();
    assert!(manifest::load(&f.package).is_err());
}
#[test]
fn event_frames_are_atomic_bounded_and_clone_send_sync() {
    #[derive(Clone)]
    struct Shared(Arc<Mutex<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let events = EventSink::writer(Box::new(Shared(buffer.clone())));
    let threads = (0..8)
        .map(|n| {
            let events = events.clone();
            std::thread::spawn(move || {
                for i in 0..10 {
                    events
                        .notify("scan.changed", &json!({"n":n,"i":i}))
                        .unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    for t in threads {
        t.join().unwrap();
    }
    let data = buffer.lock().unwrap();
    let lines = data
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 80);
    for line in lines {
        assert!(line.len() + 1 <= MAX_TOOL_FRAME_BYTES);
        let v: Value = serde_json::from_slice(line).unwrap();
        assert_eq!(v["method"], "application/event");
        assert!(v.get("id").is_none());
    }
    assert!(events.notify("bad\nname", &json!({})).is_err());
    assert!(events
        .notify("valid", &json!("x".repeat(MAX_TOOL_FRAME_BYTES)))
        .is_err());
    EventSink::silent().notify("valid", &Value::Null).unwrap();
}

#[test]
fn complete_closed_operator_settings_support_76_fields_without_expanding_tools() {
    let props: serde_json::Map<String, Value> = (0..76)
        .map(|i| (format!("field_{i}"), json!({"type":"string"})))
        .collect();
    let required: Vec<_> = props.keys().cloned().collect();
    let value: serde_json::Map<String, Value> = props
        .keys()
        .map(|key| (key.clone(), json!("saved")))
        .collect();
    let schema = json!({"type":"object","properties":props,"required":required,"additionalProperties":false});
    crate::schema::arguments(&schema, &Value::Object(value), true).unwrap();
    assert_eq!(
        crate::schema::validate(&schema, false, 0).unwrap_err(),
        "app_schema_invalid"
    );
    let props: serde_json::Map<String, Value> = (0..257)
        .map(|i| (format!("field_{i}"), json!({"type":"boolean"})))
        .collect();
    let schema = json!({"type":"object","properties":props,"additionalProperties":false});
    assert_eq!(
        crate::schema::validate(&schema, true, 0).unwrap_err(),
        "app_schema_invalid"
    );
}
