# Native applications: implementation batch after ad726b0

This continues the complete migration on `feature/agent-first-app-platform-20261016`. The first-stage inventory/draft applications are not substitutes for the desktop's full business functions. `ad726b0` and its original failures remain reviewable. Main and the user's Mac stay untouched.

## Ownership and batches

1. Apply and actually compile/test the reviewed native bootstrap proposal. It is a new process and private compatibility database, not permission to adopt desktop attempts. Continue into original Web/source execution, status, pause/resume/retry and durable operation reconciliation.
2. Extract Assets' existing Rust command bodies behind explicit trusted paths and typed project/application dispatch. Preserve desktop wrappers, SQL, DTOs, errors, ownership checks and task authority. Continue through collection/import jobs, ownership and exposure, rather than stopping at inventory queries.
3. Add a public native application contract to the SDK and generic Host process manager. A separately installed Assets or Scanning process owns its data, migrations, configuration, tasks and domain code. The Host imports neither domain and opens neither business database.
4. Connect the actual application pages to bounded, permission-checked public actions. Preserve the old desktop throughout. Validate all four installation combinations, actual subprocesses, scoped reads/writes, active work, upgrades, retained data and failures.
5. Diagnose the earlier eight-thread Mapper cancellation in an independent diagnostic worktree. Passing reruns establish no cause. Deterministic observation must preserve the original fees, deadlines, leases and assertions before any production fix is accepted.

Assets and Scanning each own a private compatibility database. Reusing existing initialization is explicitly a transitional broad schema, not a domain-only initializer. Equal host project numbers do not imply equal asset IDs. Asset selection transfers require an immutable, project-bound provenance/evidence contract; the scanner must not open Assets' database or accept unknown asset IDs as authority.

## Public native descriptor and transport

Version 1 Node packages and their 8 MiB/file, 32 MiB/package budgets remain compatible. A separately versioned manifest/Host API 2 descriptor declares `runtime: { kind: "native", platform, arch }`, a portable hash-listed executable entry and supported lifecycle/actions. It does not declare arbitrary executable arguments, inherited credentials or permission for Node packages to spawn children.

The Host validates and copies an immutable directory package, checks the declared platform, sets the declared executable's managed mode and starts that exact entry. Native package budgets remain finite; the actual linked/stripped service artifact is measured before packaging acceptance. A test executable is not release-size evidence. No publisher signature or hostile-code OS sandbox is implied.

Protocol 2 retains bounded JSON-RPC `describe`, `lifecycle/start`, `lifecycle/stop` and `tool/call`. Native startup receives Host-managed package/data directories and a versioned Host context. The exclusive child stdio channel carries the trusted context; model arguments cannot select a database, configuration root, executable or model credentials. Every business request uses the current Host-bound project, operation ID, granted permission and application/grant revisions. Native adapters enforce their own owner/lease/financial gates in addition to Host admission.

Project-facing tools stay in the model catalogue. Application administration, provider configuration and global rules require a separate declared `action/call` operator surface; they are never exposed by adding an arbitrary `invoke(command,args)` tool. Secret values are write-only/redacted in projections and are not copied into the Host catalogue or model context. Old global operations must not be represented as project-scoped tools.

Tool results keep their existing 64 KiB semantic budget. Operator action responses get a separate finite frame budget; oversized data must fail explicitly or use declared bounded pagination/resource reads, preserving source data. Native descriptions bind the installed manifest identity instead of growing an unbounded tool/action response. Input and output framing, invalid IDs and all error paths have the same bounds.

## Pages and lifecycle

Opaque-origin application iframes retain no Host token or same-origin privilege. A versioned page bridge accepts only the current iframe source, nonce, application/page identity and declared action. The Host UI binds its current project and checks fresh application/grant state before dispatch and before returning data. No arbitrary HTTP proxy, navigation or command facility is added. Project changes and closed/replaced pages invalidate in-flight replies.

Closing a page or Chat is not a service stop. Disable removes new admissions without silently killing accepted work. Stop/uninstall/upgrade must obtain an actual idle owner response and process exit. A native transport timeout or controller loss is an uncertain/quarantined owner, not permission to kill, replay or transfer paid work. Restart must report unresolved ownership honestly until an authenticated reconciliation path exists.

Service-managed database paths have explicit custody across every opening, including background/direct readers and writers. They use no-create/no-follow, original file identity and SQLite moved-file checks. Desktop paths that were not registered retain their original behavior. A service process registers one store for its lifetime; dropping a controller does not remove the guard beneath running workers. Replacement or copying cannot renew old execution authority.

Native upgrades must preserve original executable/path-bound receipts. A generic candidate data copy cannot re-sign or adopt old attempts. Unsupported migrations must reject before pointer commit; retained history and new births require explicit, independently tested semantics.

## Evidence gates

Each batch records its exact source, actual nonzero test selection, command, exit code and original failures. Native work uses one coordinated Cargo job. Protected historical executables and logs remain immutable; only identified generated caches may be removed for build space. Required behavior includes actual installed-process tool/action calls, full original Assets DTOs/operations, fixture-backed original scan execution, project and permission revocation, durable same-identity work after page/Chat close, uncertain results, busy uninstall and safe data retention.

Official Grok may be checked only through supported official repositories/releases/package sources. A blocked installer does not authorize a mirror, authentication bypass, another client's identity or the user's Mac. Missing authenticated Grok/real latency remains explicit and does not stop unrelated business migration.
