# Oviraptor App SDK v1

A dependency-free ESM SDK for Node 24 or newer. It defines a versioned public app boundary. It contains no Assets, Scanning, Chat, or host database code.

```js
import {
  API_VERSION, PROTOCOL_VERSION, AppSdkError, MANIFEST_LIMITS,
  validateManifest, validateToolArguments, qualifiedToolName,
  validatePackageDirectory, createDirectoryPackage,
  createWorker, startStdioWorker,
} from './packages/app-sdk/src/index.mjs';
```

`validateManifest(value)` returns a validated clone or throws `AppSdkError`. `validatePackageDirectory(directory)` additionally checks the actual directory and returns `{manifest,directory,files,totalBytes,manifestSha256}`. `files` contains `{path,sha256,size}`. `manifestSha256` hashes the exact UTF-8 `manifest.json` bytes, not a normalized rendering. The host must safely copy and revalidate its installed copy; checking a mutable source directory does not freeze it.

## Directory package

An installable directory contains `manifest.json`, `worker.mjs`, all declared pages, and every runtime dependency or asset in `files`. Files must be regular files with exact lowercase SHA-256 hashes. Symlinks and unlisted files are rejected. Paths use portable relative names; absolute paths, `..`, `.`, backslashes, Windows reserved names, trailing dots, case-insensitive file collisions, and file/directory collisions are rejected. The manifest does not hash itself.

```json
{
  "schemaVersion": 1,
  "id": "org.oviraptor.example",
  "version": "1.0.0",
  "hostApi": "^1.0.0",
  "entry": "worker.mjs",
  "dataSchemaVersion": 1,
  "pages": [{"id":"home","title":"Example","path":"page.html"}],
  "tools": [{
    "name":"echo",
    "description":"Echo local text",
    "inputSchema":{"type":"object","properties":{"text":{"type":"string","maxLength":2048}},"required":["text"],"additionalProperties":false},
    "permission":"echo.invoke"
  }],
  "permissions": [{"id":"echo.invoke","description":"Invoke the local echo tool"}],
  "lifecycle": {"start":true,"stop":true},
  "files": [{"path":"worker.mjs","sha256":"<64 lowercase hex>"},{"path":"page.html","sha256":"<64 lowercase hex>"}]
}
```

`lifecycle` is optional and only accepts `start`/`stop` booleans; it declares hook availability, not authorization. Other manifest fields shown are required, with no unknown fields. App IDs use lowercase DNS labels. Versions use semantic version syntax. Host API compatibility is currently exactly `^1.0.0`. Tool names are local lowercase identifiers, exposed by the host as `appId::toolName`. Each tool references one declared permission. Permission grants and project authorization belong to the host; declaring a permission never grants it.

The finite input schema subset supports `type`, `description`, scalar `enum`, closed objects (`properties`, optional `required`, `additionalProperties:false`), bounded arrays (`items`, `maxItems` up to 128, optional `minItems`), strings (`minLength`/`maxLength` up to 16384), numbers/integers (`minimum`/`maximum`), booleans, and null. Objects have at most 64 properties and nesting is at most 8. References, unions, regexes, open objects, and unknown keywords are rejected. Tool inputs must be objects. Tools and pages are also checked against the 64 KiB describe result budget.

Budgets: manifest 128 KiB; 256 files; 8 MiB per file; 32 MiB whole package; 64 tools; 16 pages; 64 permissions. See `MANIFEST_LIMITS` for the enforced values.

## Pack a source directory

The authoring source has `app.package.json`:

```json
{"manifest":{"...":"all manifest fields except files"},"files":["worker.mjs","application.mjs","page.html"],"vendorSdk":true}
```

```sh
node packages/app-sdk/bin/pack-app.mjs examples/echo-app /tmp/oviraptor-echo-package
```

Or call `createDirectoryPackage({sourceDirectory,outputDirectory,manifest,files,vendorSdk:true})`. It reads only explicitly listed source files, adds the six SDK runtime modules to `sdk/`, generates hashes, validates the result, and returns the same result shape as `validatePackageDirectory`. Source and output must be separate directories. Existing output is never overwritten. The output name is exclusively reserved and the manifest is published last; installation atomicity belongs to the host installer. With `vendorSdk:true`, source files must not use the reserved `sdk/` prefix.

Worker code imports `./sdk/worker.mjs`, never a path outside its installed package. `worker.mjs`, `protocol.mjs`, `schema.mjs`, `manifest.mjs`, `sqlite-data.mjs`, and `legacy-bridge-stub.mjs` are vendored; the filesystem packager is not included in the runtime.

## Worker protocol

JSON-RPC 2.0 requests and responses are UTF-8 newline-delimited JSON on stdin/stdout. A request must have `jsonrpc`, a string or safe-integer `id`, `method`, and `params`, with no extra fields. Notifications and batches are unsupported. Every line ends in LF; CRLF is accepted. Input lines are capped at 128 KiB and all method results at 64 KiB. There are at most 32 concurrent requests and 16 active tools. An in-flight ID cannot be reused. stdout is exclusively protocol traffic. Transport diagnostics emit fixed codes to stderr; arbitrary handler exceptions become a generic `app_internal_error`, without raw exception messages or stack traces.

| Method | Params | Result |
| --- | --- | --- |
| `describe` | `{hostApiVersion:"1.0.0"}` | `{protocolVersion:1,appId,version,dataSchemaVersion,tools,pages}` |
| `lifecycle/start` | `{dataDir,context:{hostApiVersion:"1.0.0",appId}}` | `{started:true,dataSchemaVersion}` |
| `tool/call` | `{name,arguments,context:{hostBoundProjectId,grantRevision,operationId}}` | `{content:[{type:"text",text}],structuredContent?,isError?}` |
| `lifecycle/stop` | `{}` | `{busy:boolean}` |

`hostBoundProjectId` is a positive safe integer, `grantRevision` a nonnegative safe integer, and `operationId` a nonempty string up to 128 characters. A process may serve different projects; each tool must use its current host-supplied context to scope app-owned data. Start has no project binding. The host verifies grants before dispatch and again before publishing a result. The SDK validates context shape, not its authenticity. An app must not trust a project ID supplied inside tool arguments as a replacement for host context.

```js
const controller = startStdioWorker({
  manifest, // read the installed manifest.json
  tools: {
    echo: async (args, context) => ({content:[{type:'text',text:args.text}]}),
  },
  onStart: async ({dataDir, context}) => {
    // Migrate this app's data here, before accepting tools.
    return {dataSchemaVersion:manifest.dataSchemaVersion};
  },
  onStop: async () => ({busy:false}),
});
await controller.closed;
```

`onStart` can return `undefined` or the exact declared schema version. Failed startup rejects subsequent tools. `onStop` may return `undefined` or `{busy:boolean}`. Stop during an active tool returns `{busy:true}` immediately without calling `onStop`; an app with background tasks must also veto stop while they run. A busy stop keeps the app running. A successful stop rejects future tools but does not kill the process; the host owns termination. `controller.close()` detaches input; `closed` resolves after input ends and in-flight requests finish. `createWorker(definition)` provides `handle(request)`, `state`, and `activeCalls` for embedding and tests.

Unknown methods return JSON-RPC `-32601`; parse errors use `-32700`; invalid arguments generally use `-32602`; handler failures use `-32603`. Stable details are in `error.data.code`, e.g. `app_not_running`, `app_arguments_invalid`, `app_request_duplicate`, `app_result_invalid`. Neither retry nor exactly-once semantics are implied. Apps own durable operation IDs, migrations, and reconciliation of side effects.

## Trust boundary

App code is trusted executable code. Node filesystem permission flags can constrain package/data access but are not a network sandbox or a complete isolation boundary for hostile code. This SDK does not install a network policy, OS sandbox, or arbitrary untrusted plugin execution. An app can lie about its own results or busy state. The host must enforce installation provenance, grant revisions, project bindings, safe page serving, and disable/uninstall policy. App data lives outside the immutable package and is preserved by the host across disable/uninstall according to host policy.

## Tests

```sh
node --test --test-concurrency=1 packages/app-sdk/test/*.test.mjs examples/echo-app/test/*.test.mjs
```

Tests use temporary local directories and actual child processes. They do not contact providers or remote targets.
