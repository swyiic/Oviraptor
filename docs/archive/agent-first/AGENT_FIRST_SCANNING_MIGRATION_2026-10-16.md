# Independent Scanning service: concrete migration plan

Status: design for the next implementation batch; no execution migration is accepted by this document. Written 2026-10-16 against `4d9fd1218746cca36af2ef9e5c835b7d63985c5e` in `feature/agent-first-app-platform-20261016`, following the native bridge batch `f1971fd`. Existing desktop behavior remains available. No main merge, external target, paid model request, or production mock AppHandle is authorized.

Read with `AGENT_FIRST_ARCHITECTURE_2026-10-16.md` and `AGENT_FIRST_ACCEPTANCE_2026-10-16.md`. The current Scanning app's own SQLite drafts and journal are real; they do not execute the existing scanner. The read-only bridge passed five native tests, alongside eleven existing Assets statistics tests; that is not acceptance of a running service.

## Decision

Reuse the existing Rust Scanning owner and Native runtime inside a real application-owned service process. Extract the few actual desktop dependencies into a trusted bootstrap environment and an event sink. Keep Tauri IPC as a desktop adapter to the same internal methods. Do not construct a test/mock AppHandle in production and do not write another general Agent or another budget coordinator.

The service owns admitted tasks and the runtime threads for their lifetime. A chat request or page action submits a durable operation and then reads a task reference. Closing Chat, an iframe, or an event subscription does not drop that owner. A successful tool response is not itself an execution receipt.

The current Node worker cannot launch this process: the host grants it filesystem permissions only. Native executable hosting requires an explicit, generic host/SDK transport change, not an undocumented `allow-child-process` exception for Scanning.

## Actual call graph

The relevant production functions and their current files are:

```text
Desktop asset selection
  scan_lifecycle.rs::create_sentinel_scan
  -> create_sentinel_asset_scan_in
     -> one transaction: active project + selected project_assets/assets
        + fuse filtering + exact target set + new Web mode birth

Desktop explicit URL draft
  scan_lifecycle.rs::create_sentinel_url_scan
  -> create_sentinel_url_scan_with_mode_in
  -> create_sentinel_url_scan_scheduling_in
     -> create_sentinel_url_draft_rows + register_new + commit

Desktop confirm / resume / retry
  scan_control.rs::confirm_sentinel_scan / resume_sentinel_scan
  scan_lifecycle.rs::rescan_sentinel_scan
  -> scan_control.rs::start_web_scan
     -> claim_scan_control
     -> IMMEDIATE transaction + claim_scan_quiescence_in
     -> scan_startup.rs::prepare_web_startup_in
     -> agent_backend.rs::agent_web_pipeline_runtime
     -> write immutable attempt artifacts
     -> persist_web_startup_in
     -> register_web_dispatch_binding_in + register_private_web_mode_on
     -> commit, preserving files on ambiguous commit
     -> scan_execution.rs::launch_sentinel_url_pipeline_with_tool_configuration
        -> real native-web-pipeline Rust thread
        -> NativeBranchGuard::claim_with_preflight
        -> launch_frontend_recon_producer
        -> run_agent_target_with_tool_configuration
           -> target NativeInvocationOwner + current attempt check
           -> run_owned_agent_target -> existing Native executor
        -> finish_native_branch / existing terminal publication

Desktop code / greybox / CI/CD workbench
  scan_control.rs::start_workbench_scan / rescan_workbench_scan
  -> code_analysis.rs::start_workbench_scan_impl
     -> validate source/project/auth/model/skills, original retry basis
     -> workbench_startup.rs::publish_workbench_start transaction
     -> native_source_scan.rs::launch_native_source_pipeline
        and/or the same Web pipeline
     -> record_workbench_spawn_result, without fabricating execution

Pause
  scan_control.rs::pause_sentinel_scan
  -> scan_quiescence.rs::request_sentinel_pause
  -> actual owners observe pausing and exit
  -> finish_sentinel_pause / ScanWorkerOwner::drop
     -> preserved financial/evidence hashes + original attempt checks
```

The Web and Source launch functions already take owned paths and values, and spawn `std::thread` workers without AppHandle. The Native run/branch/target guards and database remain the execution authority. There is no reason to wrap the whole runtime in a fake Tauri application.

## The desktop dependencies to remove first

| Current dependency | Actual use | Smallest shared mechanism; domain policy stays in Scanning |
| --- | --- | --- |
| `worker_resources.rs::resolve_frontend_recon_worker(&AppHandle)` | Reads Tauri resource root, then checks release-pinned worker bytes and browser-location resources. | Add a root-based resolver taking trusted bootstrap resource locations. Desktop wrapper supplies the same candidates in the same order. Preserve `verify_bundled_worker` exactly; no tool argument chooses an executable or bypasses its digest. |
| `tool_supply/approval.rs::ToolAdministratorConfiguration::from_app` | Resolves the application's configuration directory; later readers enforce private metadata, administrator/publisher anchors and original approvals. | Add an internal constructor from a trusted bootstrap directory descriptor. Keep desktop `from_app` and acceptance-root precedence. No Agent/tool field may supply configuration paths, anchors, approval flags, or a replacement cache. |
| `agent_backend.rs::agent_web_pipeline_runtime` | AppHandle is used only to obtain the worker, then a real connection resolves policy and persists the context. | A worker-parameter internal method shares the original policy resolver and original context write. Desktop wrapper still resolves exactly as before. |
| `scan_control.rs::start_web_scan` | AppHandle supplies the two dependencies above. | An internal start method accepts the typed environment; the IPC wrapper builds it. Preserve transaction, synchronous FULL, files, original owner checks and commit-before-spawn order. |
| `code_analysis.rs::start_workbench_scan_impl` | AppHandle supplies a Web worker when URLs exist. | Same typed environment, with lazy worker resolution: a source-only task must not gain a browser requirement. |
| `scan_execution_runtime.rs::NEST_LOG_APP`, `native_sdk_log.rs`, native process-log notifier | Send bounded/redacted display hints to Tauri after the original evidence/log path. | A narrow `ScanEventSink` carries only existing notification DTOs. Desktop sink emits the exact existing event names and payloads. Service sink writes its own bounded observation journal; observer loss confers no authority. |

The service environment is constructed once by the trusted process bootstrap and is not deserialized from a model tool call. It contains canonical application data root, administrator configuration root and verified resources. Credentials are not event fields and are not copied into the host's app catalogue or environment. Paths remain private implementation inputs, not page/tool arguments.

An event trait is useful here because three event kinds currently reach the same global AppHandle. It is not a generic task executor. Existing file/DB evidence stays authoritative; replacing the display sink must not change whether a model/tool invocation is admitted. A sink failure is explicit and cannot be interpreted as a successful state transition.

## Execution and persistence boundaries that must remain unchanged

1. `scan_startup.rs::prepare_web_startup_in` requires an existing transaction and validates active project, deletion/closure/recovery state, the exact old attempt, targets and fuse state. The new transport must call it rather than duplicate its SQL.
2. `native_web_resume_finance.rs::require_web_start_financial_history_in` refuses paid-history continuation even after inspecting original owners and determinate balances. A service restart or new operation ID must not create another budget. This migration does not implement the future remaining-budget continuation contract.
3. `web_dispatch_binding.rs::web_dispatch_runtime_binding` binds the running executable SHA-256, settings/model, environment, toolchain and resources. Its private HMAC and directory checks must keep working. A native service binary is a different build from the desktop binary.
4. `native_source_runtime.rs::source_runtime_material` binds original source scope, model, policy, private key and physical work directory; it grants zero target requests/host actions by itself. Capped Source model cost remains unavailable where the existing gate refuses it.
5. `agent_runtime/execution_owner.rs` hashes invocation identity beside the canonical DB path and holds an OS file lock. Do not move an active DB or lock inode. Process death releases a lock; it does not prove a prior request had no effect.
6. `NativeBranchGuard`, `ScanWorkerOwner`, current attempt/epoch/fence checks, original root/capability leases, per-request budget reservations, provider/HTTP receipts and terminal publications continue to run inside the existing executor. The host grant is an additional outer admission, not a substitute.
7. `tool_supply/sandbox/container_parser.rs` reexecutes `current_exe()` for its fixed private guardian. A new service main must dispatch `guardian_entry()` before parsing its normal protocol, exactly as the desktop entry does. Otherwise its cleanup transport breaks even though the library compiles.
8. Existing workbench spawn failure reconciliation and ambiguous commit handling remain in use. Lost transport replies are reconciled by durable operation identity and original records, never by retrying a start blindly.

Old desktop attempts stay with their original DB/work directories and executable contract. A new service can read an explicitly exported retained projection, but it cannot re-sign or rewrite an old attempt to fit itself. Initial execution support is for genuinely new births in service-owned data. Historical pending/paid tasks remain readable through the legacy route; their unsupported continuation remains an explicit refusal.

## Application-owned database and host context

The host owns projects/grants, not `sentinel_*`, `agent_*`, billing or tool-supply tables. The service owns its engine database, work roots and task operation journal. The new Node `investigations.sqlite3` draft file is not silently promoted into the engine database and its `engine:not_migrated` rows are not execution births.

`db::initialize` currently creates a broad desktop schema and runs Assets, Skills and project migrations (`db.rs:369` onward). Reusing it in an app-private directory can be an explicit transitional compatibility schema, but must not be called a domain-only initializer. Before independent execution acceptance, extract/declare the real schema closure and test it with Assets absent. No new host code should import `db.rs` or inspect that schema.

The service needs an explicit, versioned input for the host project's identity/status and operator-controlled scanning configuration. It must not query the new host SQLite file or invent an active project when a call first arrives. Archived/revoked projects require fresh outer admission and deny new work. Existing accepted work follows the specified drain/cancel policy rather than silently reacquiring authorization after a page closes.

The standalone URL path can reuse existing URL draft creation without an Assets database. The existing `create_sentinel_asset_scan_in` intentionally queries real membership/assets in one snapshot and must stay the desktop selection adapter. Future cross-app submission carries an immutable Assets selection/evidence reference, exact selected targets and host-bound project; weak ownership hints are not execution approval. Do not send asset IDs to a service that has never owned those IDs.

Model endpoint/key, browser sessions, tool administrator trust and skill content require separate explicit operator provisioning or a narrow authenticated local broker. No implicit desktop DB copying, ACP token extraction, arbitrary inherited environment, or model-supplied paths is part of the service bootstrap. The old resolver remains unique; transport normalization must not introduce a second set of defaults.

## Minimal service protocol

Use bounded JSON-RPC framing for control and typed result envelopes. The native process remains alive while it owns accepted work; request EOF is a disconnected controller, not permission to cancel or exit a busy service. A detached/retained controller attachment needs an authenticated local transport before host-restart survival is claimed. First acceptance can prove Chat/page close survival while the host itself stays alive.

| Method | Contract |
| --- | --- |
| `service/describe` | Protocol/build/schema/feature versions; no execution permission implied. |
| `service/status` | Service health and original live owners/unresolved dispatches; `busy` is not derived solely from a friendly task status. |
| `task/create` | Strict new-draft input, host-bound project and durable operation ID. Validate complete input before publication. Returns a service task reference; no execution starts. |
| `task/start` | Exact service-owned draft reference and expected revision; explicit execution grant/cost policy. Original startup/admission then commit-before-spawn. Returns accepted/uncertain/rejected plus original attempt identity. |
| `task/read` / `task/events` | Project-bound safe projection with bounded cursor/sequence. Event gaps are explicit. Model/provider material and paths are redacted before truncation. |
| `task/pause` | Calls the original pause request and observes actual quiescence; never immediately labels live work paused. |
| `task/resume` / `task/retry` | Calls original original-history gates and preserves refusal codes; no service-specific financial reset. |
| `service/stop` | Refuses busy/unresolved owners by default. Explicit drain/cancel policies require separate tested methods; no `SIGKILL` masquerading as successful uninstall. |

Task context includes the host-bound project, grant revision and operation ID, plus a service-side authenticated channel/session. A plain JSON claim of `grantRevision` is not proof of permission. Revocation prevents new admissions/results as specified; accepted tasks keep their explicit task authority until pause/cancel/drain policy is applied. Events are observations, never reusable grants or invoices.

Durable operation records bind `(project, operationId, method, canonical arguments)` to the original created task/attempt and disposition. A repeated operation returns that reference. Changed arguments conflict. A commit failure or lost reply is `uncertain` until the original records establish the outcome; operation retry must not allocate fresh attempt directories, grants or budgets.

## Packaging and hosting choice

The recommended final transport is a generic native-service worker kind in a separately versioned SDK/host contract. The host starts a verified executable from its immutable installed package and communicates using the same bounded protocol family. The host contains no Scanning SQL or function imports. The package grants neither model credentials nor administrator tool approval by declaring a service.

Current v1 is specifically a Node entry, runs without child-process permission, has no service field, and limits each package file to 8 MiB/whole package to 32 MiB. Do not smuggle a native executable or an unbounded file exception into it. Measure a real stripped release service artifact and its bundled resources before setting a finite new package budget; a test binary is not release size evidence. Keep v1 packages compatible while the new descriptor/installer checks are introduced and tested. A direct CLI/service smoke may precede host integration, but is reported as such.

A production headless Tauri AppHandle was rejected: it preserves a GUI lifetime/resource dependency, needs a UI runtime and hides the actual service/bootstrap boundary. A test MockRuntime is also rejected for production. A separate implementation of scanning/model/billing was rejected because it would duplicate the mature executor and its original authority chain.

## Ordered implementation batches and ownership

Each batch requires a design commit first, a source freeze, actual compile/behavior checks, and a reviewable implementation commit. Existing desktop handlers remain registered and retain their signatures.

1. **Desktop dependencies only.** Own `worker_resources.rs`, `tool_supply/approval.rs`, `commands/{agent_backend,scan_control,code_analysis,scan_execution_runtime,native_sdk_log}.rs`, the actual native process-log notifier, and a small new bootstrap/event module. `lib.rs` gets only the desktop environment/sink installation wiring if required. Preserve original resource candidate order, acceptance-root behavior, event names, payloads, errors, startup tuple and SQL. Root coordinates shared files and native freezes. No service start is advertised by this batch.
2. **Real service executable and private schema.** Add one Rust binary and a narrow `scanning::service` facade. Resolve the dependency closure, operator provisioning and process guardian path. Implement actual lifecycle/read/new draft/operation journal against the existing owner. Test no GUI/no Assets install/no target transport. Do not pretend current Node drafts are engine tasks.
3. **New-task execution and durable ownership.** Adapt existing Web startup first, followed by source/greybox/CI. Keep one original engine and every gate above. Preserve process lifetime through controller disconnect, and expose actual pause/quiescence, busy, uncertainty and retained projections. Test fixture transports only; no real provider or external targets.
4. **Generic host-native worker contract.** SDK/installer/process manager support explicit native service descriptors, finite artifact limits and authenticated current-project/grant context. Reuse the independent app permission/page boundary. No child-process grant to arbitrary Node app code. Test install/enable/disable/busy-uninstall and both app combinations with actual service processes.
5. **Legacy parity and migration.** Map retained evidence safely, replace placeholder app tools/pages with actual service APIs and validate selected-assets handoff. History does not gain execution by import. Complete regression evidence before calling Scanning fully migrated.

## Required tests and acceptance

The first extraction batch needs real tests for explicit roots resolving identical pinned resources, modified/symlink resource refusal, administrator root provenance/metadata rejection, desktop event identity preservation and sink failure behavior. Existing web startup, workbench/source admission, environment binding, pause/quiescence, role/capability lease, budget/HTTP/financial receipt and paid-history refusal tests must still execute unchanged.

Later service acceptance must use the actual built binary, nonzero actual test selection and real SQLite/work directories. Tests must cover duplicate concurrent start/changed operation, cross-project task references, archived project, revoked outer grant, original lease expiry/fence changes, byte-identical budget/finance records on refusal, startup commit ambiguity and crash before/after dispatch. Killing a process must never be asserted equivalent to a settled invoice or safe retry. Source guardian reentry needs its own actual-binary check.

An actual fixture-backed long task must remain alive and keep the same run/attempt/lease/budget identity after Chat and page consumers disconnect. Disable must remove new admissions and preserve the explicit drain policy; uninstall must return busy while any original invocation, active task, uncertain dispatch or cleanup obligation remains. Reattach reads the durable journal with an explicit cursor; it does not rerun the model or target. A busy-state SQL fixture alone is insufficient for this acceptance.

Expected engineering time, not a completion promise: dependency extraction and its native verification about 1–2 hours; actual service schema/bootstrap/draft/operation facade 2–4 hours; original execution/lifetime/recovery integration 4–8 hours; host-native packaging/authenticated transport 2–4 hours; full mode parity/migration and regression 1–2 additional working days depending on schema/resource/receipt issues. Native focused build cycles currently take roughly 7–8 minutes and the existing full regression adds about 25–30 minutes per frozen version. No full-migration claim is made while these batches remain incomplete.
