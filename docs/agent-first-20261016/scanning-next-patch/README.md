# Prepared next step: real native Scanning bootstrap

Prepared outside the frozen repository during the `3d2eca1` native full run. This patch is **not applied, type-checked, linked, or behavior-tested**. Parsing checks below are not native acceptance. It admits no execution, creates no model/HTTP fee, and has no start/resume/retry handler.

## Concrete implementation

`prepared.patch` adds a real `oviraptor-scan-service` binary and private Scanning bootstrap module, eight Rust behavior tests and an actual-binary Node smoke. It changes only three existing Rust files: one module declaration, one public entry facade, and one original-draft publication seam. No Cargo dependency or lock change is proposed.

The public entry first delegates the existing private container guardian and preserves its exit code. Normal bootstrap accepts three explicit trusted CLI roots (`--data-dir`, `--configuration-dir`, `--resource-dir`) and constructs the already accepted `ScanServiceEnvironment`. No path comes from model/page arguments. The generic native SDK runner is not implemented by this patch.

The bootstrap exclusively creates `engine-bootstrap-v1/` inside app-owned data. Only a newly created empty directory reaches `db::initialize`. That is explicitly a **private legacy compatibility schema**, not a domain-only Scanning migration. Existing directories require the original store marker and matching canonical root, DB device/inode and private store owner. A copied desktop DB, a missing/partial marker, a replaced DB inode or a moved store is refused; reopening never runs the broad initializer or repairs historical rows. Initial migration failure is visible and leaves evidence for explicit recovery, not automatic overwrite. The current prototype supports Unix file identities; Windows is explicitly unavailable pending an equivalent custody check.

One actual `NativeInvocationOwner` controls this private service store. This is a process-controller lock, **not** a task lease, capability or financial receipt. It prevents two processes maintaining conflicting in-memory host scopes. It is never unlinked or treated as proof of safe retry. Existing per-attempt/branch/target ownership is untouched.

`host/project` is an exclusive-parent control, never a tool. It provisions active/archived project context and a revision with a closed draft.read/draft.write permission set. A process restart forgets all grants and requires fresh host provisioning. Same-revision changes/stale revisions refuse. Host project names are not unique, whereas the legacy engine requires unique names: the local mirror uses a deterministic `[host:ID] name` to avoid cross-project collision. No host DB, user desktop DB or credential store is opened by this module.

`engine_draft_create` calls the original URL draft publisher with a receipt callback. The callback executes **inside the original draft/mode-birth transaction**. Its operation receipt, exact generated scan ID and arguments hash commit together. An ignored/failed receipt insertion rolls back the original target, scan, mode birth and associated writes. Existing desktop callers use a no-op callback and retain their complete original transaction body. This avoids a parallel draft creator, an orphan between two commits, and an operation retry that silently creates another task.

`engine_draft_read` exposes only new task references joined to this service instance's operation ownership. A foreign scan row is not adopted. Creation retries return the historical creation receipt (`liveState:false`), not an assertion about current execution. Reads are separately authorized. All permissions remain draft-only. Unknown execution methods fail closed and do not reset an old budget or manufacture an original receipt.

`service/status` reports a conservative bootstrap-only busy check. Any execution records, non-draft state or unknown physical runtime state prevents a clean stop. The EOF path does not claim successful stop while such state remains. This is **not** the final mature-runtime busy/cleanup implementation; no execution can be admitted by this binary. There is no fake long task or simulated completed scan.

## Prepared tests

The eight `scanning::service_bootstrap::tests::` tests exercise actual compatibility-schema SQLite, real original mode registration and the publication transaction: original new draft with zero execution/fees; ignored callback rollback across all real tables; operation idempotency/conflict; duplicate display names, project boundaries and revision/permission refusal; foreign/incomplete DB zero-adoption; reopen without inherited grants and foreign-row rejection; execution/argument rejection with full DB snapshot unchanged; and a real process-controller file lock with release semantics.

The separate `tools/test_scan_service_bootstrap.mjs` smoke requires an actual executable and refuses to skip if it is absent. It starts the native process without DISPLAY or a desktop AppHandle, provisions a synthetic host project, creates/retries/reads an original draft, proves execution unavailable, exits, reopens and reauthorizes the same persisted reference. No target or model transport is called. Its cleanup terminates failed synthetic processes; it is not a cancellation implementation for admitted engine work.

After the ongoing frozen full run finishes and root applies/reviews the patch, required verification is:

1. `cargo check --locked --offline --all-targets` using the agreed private native environment and one coordinator.
2. One `cargo test --lib --no-run`, actual nonzero list validation, then all eight new exact cases plus original draft/mode/admission and environment boundary tests. No old test is replaced or ignored.
3. Build the **actual** `oviraptor-scan-service` executable and run the smoke with `OVIRAPTOR_SCAN_SERVICE_TEST_BINARY` pointing at the frozen artifact. A successful library build alone does not verify this process. Linking an auto-bin can need normal-library artifacts in addition to the existing test library; inspect disk before building the three configured library crate types. No new dependencies are needed.
4. Freeze source/artifact hashes and run the appropriate full regression before acceptance.

## Public native SDK contract remains separate

Current SDK v1 is a Node worker, package file limit 8 MiB/total 32 MiB, with no child-process grant and no native descriptor. None of this changes in this patch. The bootstrap wire identifies itself as `scanning-bootstrap-v1` and reports `nativeSdkIntegrationAvailable:false`.

Recommended next public contract: a separately versioned manifest with explicit `worker.kind=native`, immutable platform/architecture-specific entry and file hashes, finite measured package limits, generic host-owned process launch and authenticated control context. The host must not import this Rust domain, query its tables, or expose `host/project` as a model/page tool. No per-app child-process escape is added to a Node worker. Measure a stripped release artifact before changing package budgets; a large test artifact is not that measurement.

The public host control session also needs an install/data-generation binding and explicit revision semantics for project status/name as well as grants. Current host grant rows can be recreated on reinstall, and project labels need not be unique. A plain persisted monotonically increasing grant integer alone cannot authenticate a new installation. This prototype keeps scope grants ephemeral under one exclusive controller and expects a current trusted parent; it must not be placed on a public socket unchanged.

## Execution still blocked on concrete work

- Full native hosting/installer/SDK v2 and authenticated controls are not connected.
- The broad compatibility initializer must become a declared, tested schema closure or remain explicitly transitional. Existing stores have no automatic upgrade/adoption in this patch.
- Operator model endpoint/key provisioning, browser sessions, administrator trust and Skills snapshots require explicit safe bridges. No OAuth/key extraction or inherited credential environment is proposed.
- Web execution can next call the existing `start_web_scan_with_environment`, using this store's actual engine root, only after new-task authority and cost controls are ready. Source/greybox/CI follow through the existing workbench inner method; do not skip their source path/model/cost gates.
- Original `current_exe` build/environment/directory HMACs, canonical DB invocation locks, root budgets/fees, fencing and paid-history refusal remain byte-identical. Old desktop pending/paid attempts stay with the original binary/DB/path. They cannot be copied, re-signed or resumed by this service.
- Genuine long-running service ownership, reconnect transport and crash/uncertain-dispatch reconciliation remain to implement and test with actual fixture-backed work. This patch does not claim survival of a real scan after a host crash. A page/Chat disconnect must eventually be merely observer loss.
- Tool-selection draft IPC and recovery IPC still have desktop AppHandle wrappers; their internal configuration/worker inputs can now use the accepted environment. Environment installation and administrator UI remain desktop concerns. The service binary reuses the current library and still links its Tauri/system dependency graph; it does not start a GUI, and it is not yet a separately minimal Rust crate.

Estimated next effort after review: applying/typing/testing this bootstrap about 1–2 hours including binary linking. Public native host/SDK transport and task authority/config provisioning remain several additional hours before connecting original execution; full mode parity remains a separate regression stage. These are estimates, not completion claims.
