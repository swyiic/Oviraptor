# Scanning app, v1 migration boundary

This independent Node 24 package (`org.oviraptor.scanning`) currently owns investigation drafts and their durable journal. It does not run a new scanner, recreate the existing NativeAgent, or claim that storing a draft starts a scan. Full desktop scanning continues through the existing desktop engine and its existing financial, lease, authorization, cancellation, and replay gates.

Build an installable directory package from repository root:

```sh
node packages/app-sdk/bin/pack-app.mjs apps/scanning /tmp/oviraptor-scanning-package
```

The package includes a hashed manifest and vendored SDK. The isolated page implements only the host's lifecycle handshake; it receives no session token and forwards no tool calls.

## Implemented tools and data

The worker owns `investigations.sqlite3` under its host-assigned data directory. It persists tasks, journal events, and idempotent operations. Every read/write is bound to the current tool call's host project; one worker can serve multiple projects.

| Tool | Actual behavior |
| --- | --- |
| `draft_create` | Atomically stores a draft and its first journal event for 1–32 explicit HTTP(S) target URLs. Rejects credentials, fragments, duplicate targets, and conflicting operation retries. Returns `executionStarted:false`, `executionAvailable:false`, and `authorization:not_granted`. |
| `draft_cancel` | Cancels drafts. It rejects live engine states rather than forging engine cancellation. |
| `task_status` | Returns only id/title/status/timestamps for a project-owned task. No journal payload and no execution claim. |
| `journal_read` | Reads the project's persisted task and its last 50 journal events. Long target lists are an explicitly marked complete-value prefix (`targetsTotal`, `targetsReturned`, `targetsTruncated`); the original targets stay unchanged in SQLite. The actual serialized tool response stays bounded. |

`legacy_saved_scans` is **not** on the public manifest/tool surface until a host-managed `legacyBridge` is injected. Without that bridge the handler is not registered, so agents cannot call it and misread empty/unavailable as zero inventory. The underlying `legacy.mjs` path remains for future trusted embedding.

Stop checks actual persisted queued/running/pausing/cancel-requested states and returns `busy:true` until the owner makes them terminal. The current app has no tool that starts those states. The busy test deliberately inserts an engine-state fixture to test lifecycle refusal; it does not demonstrate a migrated background scanner.

## Remaining execution migration

The Rust compatibility adapter has a real read-only `saved_scans_list` projection backed by the existing Scanning owner, with project binding and no private working paths. It reads only an explicitly staged `legacy-input.sqlite3` snapshot. Original scan SQL and row mapping are shared through a connection-only extraction; the desktop's opener and execution paths remain intact.

The installed Node process cannot spawn the Rust binary because the host grants filesystem permissions only. `createApplication(manifest, {legacyBridge})` permits an explicit trusted embedding adapter, but host-managed transport is not connected in this version. Snapshot export/staging and the actual engine service protocol, task admission/start/resume/cancel, fees/lease/replay integration, and complete scanning UI migration remain follow-up work. None of those capabilities is represented by a timer or a fabricated completed task.

## Verification

```sh
node --test apps/assets/test/*.test.mjs apps/scanning/test/*.test.mjs
```

Tests use real persisted SQLite drafts, restart/reopen, cross-project rejection, operation conflicts, explicit unavailable legacy behavior, active-state stop refusal, and a maximum-size journal response. Actual packaged stdio workers run under Node filesystem-only permissions. Native bridge tests remain a separate verification layer.

## Phase-1 thin service entry

`thin-service.mjs` validates the non-sensitive Phase-1 tool surface (`draft_create` / `draft_cancel` / `task_status` / `journal_read`) and constructs the same application worker. It does not rewrite Nest/multi_agent kernels; native execution stays behind `oviraptor-scan-service` and the separate native package.
