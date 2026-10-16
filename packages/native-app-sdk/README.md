# Native application SDK v2

This independent Rust crate implements the public native app transport. It has
only serde, serde_json and sha2 dependencies: no Tauri, Host, Agent, Assets,
Scanning, business database, provider configuration or authentication imports.
It does not grant authority to execute a scan or adopt existing work.

The Host launches the hash-listed installed entry directly:

```text
bin/application --app-worker --data-dir CANONICAL_DATA --package-dir CANONICAL_PACKAGE
```

Both directories must already exist and be canonical, separate directories.
The SDK reads `manifest.json` and only its declared package files. It validates
manifest schema 2, `hostApi: ^2.0.0`, native platform/architecture, portable file
paths, exact file inventory and SHA256, finite catalogues and schemas. Limits
are 256 KiB manifest, 512 files, 128 MiB/file and 256 MiB including the manifest.
The running executable must be the declared package entry. These checks are not
a publisher signature, hostile-code sandbox or a Windows/macOS packaging test.

`describe({hostApiVersion: "2.0.0"})` returns protocol 2, application/version,
the SHA256 of the **exact installed manifest bytes**, and ordered tool/action/page
names. It does not invoke the application factory or initialize its data.

Only a subsequent valid `lifecycle/start` with the exact launch directories and
`{hostApiVersion: "2.0.0", appId}` context can call the factory. The factory gets:

```rust,ignore
Bootstrap {
    data_dir, package_dir, app_id, data_schema_version, events
}
```

The domain must reject unsupported `data_schema_version` before migration.
`NativeApplication::data_schema_version()` must report the actual opened schema.
The SDK compares that fact to the manifest before acknowledging startup.
A failed startup is not replayed in the same process. Successful repeated start
acknowledges the already created application without invoking its factory again.

Implement `NativeApplication::{data_schema_version, call, status, stop}` and call
`native_app_sdk::run_stdio(factory)`. `call` receives `CallSurface::{Tool,
ProjectAction, ApplicationAction}` selected from the RPC method and declared
manifest scope. Arguments cannot choose the surface. Closed, bounded context
requires the active Host-bound project, matching project ID, grant/application
revisions, operation ID and the declared/granted permission. Domain adapters
must additionally enforce their own current project, custody, lease, budget,
approval and financial gates. Exclusive stdio is a trusted-parent channel, not
authentication for a public socket.

Tools return a raw JSON value, which the SDK wraps as MCP text content (and
object `structuredContent` when applicable). Actions return their raw JSON DTO,
including `null` or numbers. Tool JSON-RPC frames including the LF are at most
64 KiB; action frames are at most 4 MiB. Over-budget output is an explicit error,
never truncated rows. Only bounded `[A-Za-z0-9_]` error codes pass through; an
internal message/path/key becomes `app_operation_failed`. Invalid IDs are null,
and all errors use the same bounded encoder.

Arguments use the JavaScript SDK's finite closed schema subset: depth at most 8,
64 properties/object, scalar enums, required/type/numeric/text bounds, tool
arrays at most 128 and strings 16384 characters; action arrays at most 16384
and strings 1 Mi characters. Serialized tool arguments are at most 64 KiB,
actions 1 MiB and context 16 KiB. Input NDJSON frames have a fixed 1 MiB + 16 KiB
bound. Overflow is drained to the next newline with bounded storage. Invalid
UTF-8/JSON, unknown fields, unsupported schemas and partial EOF fail explicitly.

`EventSink` is cloneable, Send and Sync. `notify(name, &payload)` emits a standard
JSON-RPC `application/event` notification under the same stdout mutex as replies,
at most 64 KiB including LF. `silent()` retains validation without output. Events
are refresh hints; they never substitute for permission-checked results.

Stopping requires `status()` to report `{busy:false}` without unknown/uncertain
owner flags, followed by `stop()` independently confirming `{busy:false}` and,
if present, `stopped:true`. The actual response is written before normal exit.
Busy/unknown status refuses stop. EOF, partial input and input/output failures
retain the application and process while status remains busy or unknown; the
SDK waits for actual idle without calling cancel, stop, retry or a new factory.
The domain must make `status` an honest physical ownership/quiescence check.
If ownership can never be reconciled, the process intentionally remains alive.

The Host-only `lifecycle/status {}` control is available after successful start.
It calls the domain's actual `status()` and returns that observation in a complete
frame of at most 4 KiB. A missing/non-Boolean `busy`, invalid uncertainty flags,
or oversized data fails explicitly; the SDK never substitutes an idle status.
This control is not a model tool or a project action. The Host must expose only
fresh metadata such as busy/uncertain to its UI and keep raw domain status out of
model context, refresh notifications and general catalogue responses.

The tests use only temporary package/data fixtures, synthetic applications and
in-memory streams. They cover protocol/manifest identity, factory admission,
actual schema mismatch, permission/project isolation, tool/action routing,
finite schemas, exact wire budgets, atomic concurrent notifications, framing,
busy/uncertain stop, EOF and BrokenPipe retention. They are not real native
Assets/Scanning integration or a Grok/provider test.

Run through the repository's single Cargo coordinator:

```sh
cargo test --manifest-path packages/native-app-sdk/Cargo.toml --offline
```

The crate is standalone and is not automatically attached to the desktop Cargo
graph. Native application owners integrate it through an explicit path dependency.
