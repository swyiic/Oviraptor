# Assets app, v1 migration boundary

This is an independently packaged Node 24 app (`org.oviraptor.assets`). The host loads its manifest and starts its stdio worker; it does not import this domain's code or read its SQLite tables. Each call uses the host-bound project in protocol context, never a caller-supplied project argument. Declared permissions still require host grants.

Build a directory package from repository root:

```sh
node packages/app-sdk/bin/pack-app.mjs apps/assets /tmp/oviraptor-assets-package
```

The generated package contains an exact-hash `manifest.json` and vendored SDK runtime. Its source directory is an authoring input, not an installable package. The page supports the host's isolated iframe handshake; tools are invoked through the authorized host API, not through iframe messages.

## Implemented tools and data

The worker owns `inventory.sqlite3` in its host-assigned data directory. It creates no legacy desktop tables and performs no target network requests.

| Tool | Actual behavior |
| --- | --- |
| `inventory_import` | Atomically imports up to 128 explicit records. Replacement key is `(project, source, externalId)`. One whole observation replaces another; fields from older observations are not stitched into it. Operation IDs make retries idempotent and reject changed arguments. |
| `inventory_stats` | Counts this project's imported records exactly, with closed HTTP/probe buckets including unknown values. The unit is app inventory records, not provider discovery completeness, unique canonical endpoints, or current reachability. |
| `inventory_search` | Reads imported metadata with literal substring matching, exact matching count, ID order, and a cursor bound to project/filter/order. An adaptive serialized result budget keeps pagination within the SDK envelope. Separate pages are fresh reads rather than a durable snapshot. |
| `inventory_schema` / `inventory_query` / `project_overview` / `exposure_overview` | Agent-facing inventory schema, whitelist query, project overview, and exposure aggregates. |

`legacy_snapshot_stats` is **not** on the package manifest by default. The host re-registers it on capabilities **only** when a staged `legacy-input.sqlite3` exists **and** an `oviraptor-app-worker` (or test adapter) can serve it (`packages/agent-host/src/legacy-bridge.mjs`). Without that probe the tool stays absent, so agents cannot misread empty/unavailable as zero inventory. `createApplication(..., { legacyBridge })` remains the trusted in-process embedding seam (and publishes the tool on the returned manifest when injected).

`observedAt` is saved as supplied evidence; `receivedAt` is the time of this explicit import. No provider time is invented. Imports do not merge `www` with apex, merge protocols, confirm ownership, or grant scan authorization. This first app store deliberately uses explicit source IDs; the desktop's full canonical identity, discovery, ownership/exposure review, recycle, and filtering workflows have not been migrated into this app.

## Explicit legacy compatibility

`src-tauri/src/legacy_app_bridge.rs` and `src-tauri/src/bin/oviraptor-app-worker.rs` implement a separate, read-only compatibility adapter. Its CLI accepts `--app org.oviraptor.assets --data-dir ABSOLUTE_CANONICAL_DIRECTORY`; it opens only an explicitly staged regular `legacy-input.sqlite3` snapshot in that directory. It uses the existing Assets statistics/search owners without initializing or migrating the snapshot. Aggregated active/trash/project counts are permitted; individual saved-asset search is restricted to active records. It never reads the user's desktop database implicitly.

The installed Node worker has no child-process permission and cannot start that binary. `createApplication(manifest, {legacyBridge})` is an explicit trusted embedding seam, not a connected host feature. Host-managed staging + conditional tool exposure is wired; injecting legacyBridge into the Node stdio worker process remains deferred (no child-process grant). The original desktop remains the complete legacy application path.

## Verification

```sh
node --test apps/assets/test/*.test.mjs apps/scanning/test/*.test.mjs
```

Tests exercise actual app SQLite files, project isolation, replacement/idempotency, unknown buckets, more than 20 records, cursor rejection, directory-package hashes, and actual packaged workers under Node filesystem-only permissions. Rust owner reuse is tested separately by `legacy_app_bridge::tests::`; a Node package test does not prove a connected native bridge or complete domain migration.
