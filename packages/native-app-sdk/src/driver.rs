use crate::{
    manifest::{self, Manifest},
    schema, Bootstrap, CallSurface, NativeApplication, NativeCallContext, MAX_ACTION_FRAME_BYTES,
    MAX_CONTEXT_BYTES, MAX_INPUT_FRAME_BYTES, MAX_SAFE_INTEGER, MAX_TOOL_FRAME_BYTES,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

type Factory = fn(&Bootstrap) -> Result<Box<dyn NativeApplication>, String>;
struct Output {
    writer: Box<dyn Write + Send>,
    failed: bool,
}

/// Cloneable notification-only writer. Events carry no call result or authority.
#[derive(Clone)]
pub struct EventSink {
    output: Option<Arc<Mutex<Output>>>,
}
impl EventSink {
    pub fn silent() -> Self {
        Self { output: None }
    }
    pub(crate) fn writer(writer: Box<dyn Write + Send>) -> Self {
        Self {
            output: Some(Arc::new(Mutex::new(Output {
                writer,
                failed: false,
            }))),
        }
    }
    fn send(&self, frame: &[u8]) -> Result<(), String> {
        if let Some(output) = &self.output {
            let mut output = output.lock().map_err(|_| "app_transport_output_failed")?;
            if output.failed {
                return Err("app_transport_output_failed".into());
            }
            if output
                .writer
                .write_all(frame)
                .and_then(|_| output.writer.flush())
                .is_err()
            {
                output.failed = true;
                return Err("app_transport_output_failed".into());
            }
        }
        Ok(())
    }
    pub fn notify(&self, name: &str, payload: &Value) -> Result<(), String> {
        if name.is_empty()
            || name.len() > 96
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return Err("app_event_invalid".into());
        }
        let v = json!({"jsonrpc":"2.0","method":"application/event","params":{"name":name,"payload":payload}});
        self.send(&encode(&v, MAX_TOOL_FRAME_BYTES)?)
    }
}

fn encode(value: &Value, limit: usize) -> Result<Vec<u8>, String> {
    let mut out = schema::bytes(value, limit.saturating_sub(1))?;
    out.push(b'\n');
    Ok(out)
}
fn valid_id(id: &Value) -> bool {
    id.as_i64()
        .is_some_and(|n| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&n))
        || id.as_str().is_some_and(|s| crate::bounded_text(s, 128))
}
fn safe_code(code: &str) -> &str {
    if !code.is_empty()
        && code.len() <= 96
        && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        code
    } else {
        "app_operation_failed"
    }
}
fn error(id: Value, code: &str) -> Value {
    let code = safe_code(code);
    json!({"jsonrpc":"2.0","id":if valid_id(&id){id}else{Value::Null},"error":{"code":-32602,"message":code,"data":{"code":code}}})
}
fn reply(id: Value, result: Result<Value, String>, limit: usize) -> (Vec<u8>, bool) {
    let success = result.is_ok();
    let v = match result {
        Ok(value) => json!({"jsonrpc":"2.0","id":id,"result":value}),
        Err(code) => error(id.clone(), &code),
    };
    match encode(&v, limit) {
        Ok(bytes) => (bytes, success),
        Err(_) => (
            encode(&error(id, "app_result_too_large"), MAX_TOOL_FRAME_BYTES)
                .expect("fixed error envelope bounded"),
            false,
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    jsonrpc: String,
    id: Value,
    method: String,
    params: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    name: String,
    arguments: Value,
    context: Value,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    New,
    Described,
    Started,
    StartFailed,
    Stopped,
}

pub(crate) struct Driver {
    bootstrap: Bootstrap,
    manifest: Manifest,
    manifest_hash: String,
    factory: Factory,
    application: Option<Box<dyn NativeApplication>>,
    state: State,
}
impl Driver {
    pub(crate) fn new(
        bootstrap: Bootstrap,
        manifest: Manifest,
        manifest_hash: String,
        factory: Factory,
    ) -> Self {
        Self {
            bootstrap,
            manifest,
            manifest_hash,
            factory,
            application: None,
            state: State::New,
        }
    }
    fn dispatch(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "describe" => {
                if params != json!({"hostApiVersion":"2.0.0"}) {
                    return Err("app_protocol_invalid".into());
                }
                if self.state == State::New {
                    self.state = State::Described;
                }
                Ok(self.manifest.description(&self.manifest_hash))
            }
            "lifecycle/start" => {
                if !schema::object_keys(
                    &params,
                    &["dataDir", "packageDir", "context"],
                    &["dataDir", "packageDir", "context"],
                ) || params["dataDir"].as_str().map(Path::new)
                    != Some(self.bootstrap.data_dir.as_path())
                    || params["packageDir"].as_str().map(Path::new)
                        != Some(self.bootstrap.package_dir.as_path())
                    || params["context"]
                        != json!({"hostApiVersion":"2.0.0","appId":self.bootstrap.app_id})
                {
                    return Err("app_context_invalid".into());
                }
                if self.state == State::Started {
                    return Ok(json!({"dataSchemaVersion":self.manifest.data_schema_version}));
                }
                if self.state != State::Described || !self.manifest.lifecycle_allowed("start") {
                    return Err("app_lifecycle_invalid".into());
                }
                // A failed factory may have begun a migration. Never replay it in
                // this process or claim an unvalidated schema version succeeded.
                self.state = State::StartFailed;
                let app = (self.factory)(&self.bootstrap)?;
                let actual = app.data_schema_version();
                self.application = Some(app);
                if actual != self.manifest.data_schema_version {
                    return Err("app_data_schema_mismatch".into());
                }
                self.state = State::Started;
                Ok(json!({"dataSchemaVersion":actual}))
            }
            "lifecycle/stop" => {
                if params != json!({}) || !self.manifest.lifecycle_allowed("stop") {
                    return Err("app_lifecycle_invalid".into());
                }
                if self.state != State::Started {
                    return Err("app_lifecycle_invalid".into());
                }
                let app = self.application.as_mut().ok_or("app_lifecycle_invalid")?;
                require_idle(&app.status()?)?;
                let result = app.stop()?;
                require_idle(&result)?;
                if result
                    .get("stopped")
                    .is_some_and(|v| v != &Value::Bool(true))
                {
                    return Err("app_stop_unconfirmed".into());
                }
                self.state = State::Stopped;
                Ok(result)
            }
            "tool/call" | "action/call" => {
                if self.state != State::Started {
                    return Err("app_not_started".into());
                }
                let call: Call =
                    serde_json::from_value(params).map_err(|_| "app_protocol_invalid")?;
                let action = method == "action/call";
                let specs = if action {
                    &self.manifest.actions
                } else {
                    &self.manifest.tools
                };
                let spec = specs
                    .iter()
                    .find(|s| s.name == call.name)
                    .ok_or("app_method_unknown")?;
                schema::bytes(&call.context, MAX_CONTEXT_BYTES)
                    .map_err(|_| "app_context_invalid")?;
                let context: NativeCallContext =
                    serde_json::from_value(call.context).map_err(|_| "app_context_invalid")?;
                context.validate(&spec.permission)?;
                if context
                    .permissions
                    .iter()
                    .any(|p| !self.manifest.permissions.iter().any(|item| &item.id == p))
                {
                    return Err("app_context_invalid".into());
                }
                schema::arguments(&spec.input_schema, &call.arguments, action)?;
                let surface = if !action {
                    CallSurface::Tool
                } else if spec.scope.as_deref() == Some("project") {
                    CallSurface::ProjectAction
                } else {
                    CallSurface::ApplicationAction
                };
                let result = self.application.as_mut().ok_or("app_not_started")?.call(
                    surface,
                    &call.name,
                    call.arguments,
                    &context,
                )?;
                if action {
                    return Ok(result);
                }
                let text = String::from_utf8(schema::bytes(&result, MAX_TOOL_FRAME_BYTES)?)
                    .map_err(|_| "app_result_invalid")?;
                let mut tool = json!({"content":[{"type":"text","text":text}],"isError":false});
                if result.is_object() {
                    tool["structuredContent"] = result;
                }
                Ok(tool)
            }
            _ => Err("app_method_unknown".into()),
        }
    }
    pub(crate) fn handle(&mut self, frame: &[u8]) -> (Vec<u8>, bool) {
        if frame.len() > MAX_INPUT_FRAME_BYTES {
            return reply(
                Value::Null,
                Err("app_message_too_large".into()),
                MAX_TOOL_FRAME_BYTES,
            );
        }
        let request = match serde_json::from_slice::<Request>(frame) {
            Ok(r) => r,
            Err(_) => {
                return reply(
                    Value::Null,
                    Err("app_request_invalid".into()),
                    MAX_TOOL_FRAME_BYTES,
                )
            }
        };
        if request.jsonrpc != "2.0" || !valid_id(&request.id) {
            return reply(
                Value::Null,
                Err("app_request_invalid".into()),
                MAX_TOOL_FRAME_BYTES,
            );
        }
        let limit = if request.method == "action/call" {
            MAX_ACTION_FRAME_BYTES
        } else {
            MAX_TOOL_FRAME_BYTES
        };
        let result = self.dispatch(&request.method, request.params);
        let (bytes, success) = reply(request.id, result, limit);
        (bytes, success && self.state == State::Stopped)
    }
    pub(crate) fn wait_idle(&mut self, mut pause: impl FnMut()) {
        // EOF, read failure and output failure do not cancel/retry accepted work.
        // Unknown status is deliberately not an idle receipt.
        if let Some(app) = self.application.as_mut() {
            loop {
                if app.status().is_ok_and(|v| require_idle(&v).is_ok()) {
                    break;
                }
                pause();
            }
        }
    }
}

fn require_idle(value: &Value) -> Result<(), String> {
    if value.get("busy") == Some(&Value::Bool(true)) {
        return Err("app_busy".into());
    }
    if !value.is_object()
        || value.get("busy") != Some(&Value::Bool(false))
        || [
            "uncertain",
            "outcomeUnknown",
            "ownerUnresolved",
            "quarantined",
        ]
        .iter()
        .any(|k| value.get(*k).is_some_and(|v| v != &Value::Bool(false)))
    {
        return Err("app_owner_unresolved".into());
    }
    Ok(())
}

pub(crate) enum Frame {
    Line(Vec<u8>),
    TooLarge,
    Partial,
    Eof,
}
pub(crate) fn read_frame(input: &mut impl BufRead) -> io::Result<Frame> {
    let mut data = Vec::new();
    let mut overflow = false;
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            return Ok(if overflow || !data.is_empty() {
                Frame::Partial
            } else {
                Frame::Eof
            });
        }
        let end = chunk.iter().position(|b| *b == b'\n');
        let count = end.unwrap_or(chunk.len());
        if !overflow {
            if count > MAX_INPUT_FRAME_BYTES.saturating_sub(data.len()) {
                data.clear();
                overflow = true;
            } else {
                data.extend_from_slice(&chunk[..count]);
            }
        }
        let consumed = count + usize::from(end.is_some());
        input.consume(consumed);
        if end.is_some() {
            if overflow {
                return Ok(Frame::TooLarge);
            }
            if data.last() == Some(&b'\r') {
                data.pop();
            }
            return Ok(Frame::Line(data));
        }
    }
}

pub(crate) fn run_loop(
    input: &mut impl BufRead,
    driver: &mut Driver,
    events: &EventSink,
) -> Result<(), String> {
    let outcome = loop {
        let frame = match read_frame(input) {
            Ok(frame) => frame,
            Err(_) => break Err("app_transport_input_failed".into()),
        };
        let (bytes, stop) = match frame {
            Frame::Eof => break Ok(()),
            Frame::Partial => {
                let (bytes, _) = reply(
                    Value::Null,
                    Err("app_partial_frame".into()),
                    MAX_TOOL_FRAME_BYTES,
                );
                let result = events.send(&bytes);
                break result.and(Err("app_partial_frame".into()));
            }
            Frame::TooLarge => reply(
                Value::Null,
                Err("app_message_too_large".into()),
                MAX_TOOL_FRAME_BYTES,
            ),
            Frame::Line(line) => driver.handle(&line),
        };
        if let Err(error) = events.send(&bytes) {
            break Err(error);
        }
        if stop {
            return Ok(());
        }
    };
    driver.wait_idle(|| std::thread::sleep(Duration::from_millis(250)));
    outcome
}

/// Launch exactly the declared package worker. No provider/model environment or
/// settings are read. The domain factory alone owns migration and execution.
pub fn run_stdio(factory: Factory) -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 5
        || args[0] != "--app-worker"
        || args[1] != "--data-dir"
        || args[3] != "--package-dir"
    {
        return Err("app_bootstrap_invalid".into());
    }
    let data = manifest::canonical_directory(Path::new(&args[2]))?;
    let package = manifest::canonical_directory(Path::new(&args[4]))?;
    if data == package || data.starts_with(&package) || package.starts_with(&data) {
        return Err("app_directory_invalid".into());
    }
    let (manifest, hash) = manifest::load(&package)?;
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| "app_entry_invalid")?;
    if executable != package.join(&manifest.entry) {
        return Err("app_entry_invalid".into());
    }
    let events = EventSink::writer(Box::new(io::stdout()));
    let boot = Bootstrap {
        data_dir: data,
        package_dir: package,
        app_id: manifest.id.clone(),
        data_schema_version: manifest.data_schema_version,
        events: events.clone(),
    };
    let mut driver = Driver::new(boot, manifest, hash, factory);
    run_loop(&mut io::stdin().lock(), &mut driver, &events)
}
