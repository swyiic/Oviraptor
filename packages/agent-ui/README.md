# Standalone Agent UI

This is a new, dependency-free browser ESM client for the standalone Node Agent host. It does not import the existing Vue/Tauri renderer or any Assets, Scanning, HackerOne or application package. Its CSS and DOM renderer are original to this package; no business renderer was copied.

Build and test from the repository root, without installing the legacy frontend dependencies:

```sh
node packages/agent-ui/scripts/build.mjs
node --test packages/agent-ui/test/*.test.mjs
node packages/agent-host/src/cli.mjs --data-dir /tmp/oviraptor-empty-host --runtime none
```

Open the loopback URL printed by the host. An empty application installation supports projects, conversations and local messages. The default runtime is unconfigured and makes no model request. The Settings panel selects None or Grok ACP explicitly. Saving a runtime preference does not log in, copy an API key, make a model request or resend a message. Existing model provider profiles remain in the legacy desktop application; runtime selection does not promise the same model, endpoint, cost or speed. `dist/` is generated and contains only native ESM, CSS, HTML and a SHA-256 build manifest.

The host injects a session token into the root page. All API requests, including the fetch-based SSE connection, use `X-Oviraptor-Session`; the token never enters a URL. Reconnecting SSE does not resend messages, install requests, grants or permission choices. HTTP errors keep the user's draft. Project and conversation read epochs prevent late responses from replacing another workspace.

Messages are rendered with DOM text nodes, without interpreting HTML. Runtime permission options require explicit clicks and match the active conversation. Cancellation requests wait for the runtime's terminal receipt; they do not claim immediate completion. Reconnecting consumes the authoritative `host.snapshot` active-run and approval state.

Applications can be installed from a local directory, enabled, disabled, granted specific project permissions, and uninstalled with application data retained. Installation grants nothing. A separate disclosure checkbox confirms that the current project's Agent may use the selected permissions and send tool data to the current runtime. Grants include the observed revision. Disabling stops admission and revokes grants while accepted work continues; busy applications cannot be uninstalled. Closing an application page only removes its iframe.

Application pages use `sandbox="allow-scripts"` with no same-origin permission. Only registered local `/apps/:id/:version/pages/:pageId` URLs are accepted. The parent sends a random per-frame nonce and project context, never its session token. The `oviraptor.app-ui.v1` bridge checks the exact frame, opaque origin, application ID, nonce and lifecycle-only message allowlist: `ready`, `resize`, `notice`, `close`. It has no generic API, tool, permission or navigation proxy. The host must independently enforce session/Origin checks and page CSP.

Tests cover the API/SSE transport, stale scope responses, duplicate sends, explicit permission choices, cancellation receipts and the frame boundary. The independent-build test copies this package into a temporary directory without the repository or node_modules. The initial Chromium integration evidence is outside the repository under `/workspace/validation/agent-first-20261016/ui/`; it uses the real Node Host, SQLite and a packaged local Echo worker. Runtime streaming/approval/cancellation uses an explicitly injected synthetic runtime. Its busy UI case controls a worker-active counter. These do not constitute live Grok, paid provider, desktop or real scanning execution acceptance.

Skills have a separate library and conversation-selection panel. A local directory is previewed before an import sends its SHA-256 fingerprint; editing the path invalidates the preview. Bundled import uses only the host distribution. Importing, enabling, previewing resource text or selecting reference guidance grants no application tool permissions. Each conversation selection carries its observed revision and explicit disclosure. Disabled, missing or stale selections require review; the UI never silently replaces them. Resource reads request at most 8192 bytes per page, render as text, and never execute resource scripts. Package uninstall retains package data.

Runtime cancellation is a request, not a promise of cancellation. Terminal receipts clear the waiting state; an unknown outcome is visibly marked as unknown with no automatic retry.
