# Local Echo app

This example is a real directory package and stdio worker, with no network calls or host database access. It demonstrates app-owned JSON persistence, migrations, project-scoped records, and durable operation IDs. Its static HTML page has no host bridge or scripts.

Build from the repository root:

```sh
node packages/app-sdk/bin/pack-app.mjs examples/echo-app /tmp/oviraptor-echo-package
```

Install that output directory through the host's app manager. The authoring directory itself is not an installable package. The packager includes every runtime file plus the SDK under `sdk/`, so the installed worker needs no repository paths or dependencies. Node 24 or newer is required.

The host supplies an independent app data directory at `lifecycle/start`. `echo-state.json` schema version 2 stores up to 128 operations (at most 512 KiB). Version 1 data `{schemaVersion:1,echoes:[{projectId,operationId,text}]}` is migrated by the app before startup succeeds. Unknown versions, corrupt data, or overflow are refused without replacing the original state. App writes use a private temporary file and rename. This simple demo relies on the host's single worker per app; it does not provide multi-process data locking or crash recovery beyond atomic file replacement.

`echo({text})` records text for `context.hostBoundProjectId`. Reusing that project's operation ID and exact text returns the prior record without writing; different text under the same ID is refused. Records are never silently evicted: a full store refuses new operations but can replay existing ones. `history({})` returns up to the latest 16 records for that project, shortened further as necessary to fit the 64 KiB UTF-8 result budget, with explicit `total` and `truncated`. No credentials or sensitive text should be entered into this demonstration: supplied echo text is deliberately persisted locally.

Stop is vetoed while an echo write is pending. The host preserves this data when disabling or uninstalling the package. The page is informational; tools are invoked through host-authorized calls, not HTML actions. The SDK's Node filesystem permissions are not a network sandbox for untrusted apps.

Run local tests from the repository root:

```sh
node --test --test-concurrency=1 examples/echo-app/test/*.test.mjs
```
