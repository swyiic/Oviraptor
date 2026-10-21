# Independent Agent host

This is an opt-in migration host in the same repository. The legacy desktop stays available while complete Assets/Scanning parity is unfinished. Core storage does not read or initialize application domain tables. It uses Node 24's built-in SQLite and starts with zero applications and no connected runtime.

From repository root:

```sh
npm run build:agent-first
npm run build:app-packages
node .agent-build/agent-first/packages/agent-host/src/cli.mjs --data-dir /absolute/private/host-data
```

Open the loopback URL printed by the host. Install a generated package directory through Applications. Installation starts disabled and grants nothing. Enable it and explicitly choose project permissions/data disclosure. Closing a page is distinct from disabling the app. Disable removes new capability admission and revokes grants without force-cancelling accepted work. Busy uninstall is rejected; successful uninstall keeps application data and immutable package bytes. Reinstall requires new grants.

To select the official CLI runtime explicitly, add `--runtime grok-acp`. Omitting the option restores the saved runtime preference; `--runtime none` explicitly starts in save-only mode without changing that preference. The Settings panel can change the persisted preference. Selection starts no process, authentication or model request. The CLI must already be installed and authenticated by the user using its official flow. The host never reads/copies OAuth tokens. Cloud tests use synthetic ACP processes only; do not run authenticated account tests without the user's participation. No `--always-approve` or security-setting changes are used.

Applications run separate Node workers and own data/migrations. The current trusted-local-package installer verifies every file and applies Node filesystem permissions; it is not a secure sandbox for arbitrary third-party native code or networking. Package identity/hash integrity is not publisher signature verification. OS-level distribution/signing and upgrade rollback are not yet accepted.

Grok receives a conversation-bound stdio MCP server. Tool discovery is the current intersection of installed/enabled/healthy app, declared schema and explicit project permission. The host binds project and grant revision, validates before dispatch and withholds results after revocation. Application pages have no host API token and cannot turn page messages into arbitrary API calls.

The Skills panel previews/imports local packages, installs the bundled 36-entry/443-file catalog, manages availability and resources, and selects reference guidance per conversation. Consent is tied to the observed runtime, library revision and selected content/provenance fingerprints. Revocation during a turn withholds subsequent resource bytes and final text. Scripts stay inert; Skills do not grant app permissions. Existing desktop Skills data is not silently adopted.

The new ACP host does not yet replace every legacy Chat/model-profile feature. Legacy provider configuration remains in the existing desktop and is never silently converted into Grok subscription credentials. Full configuration migration and complete scanning execution must pass their own acceptance before switching the default application.

`GET /api/runtime/metrics?limit=100` exports a bounded recent window (maximum 1000 turns) using the ordinary host session header. It reports cold start, first text, first tool, runtime turn and host elapsed times, with absent observations remaining null. Success rate is known completed turns divided by every terminal attempt in the returned evidence group; cancellations, refusals and unknown outcomes remain in the denominator. Synthetic/custom and unverified official-command observations are separated. These measurements do not establish authenticated Grok performance.

Upgrades require an explicitly disabled app, its current revision and a separately reviewed newer directory package (`POST /api/apps/:id/upgrade`). The old and candidate workers must actually exit; the candidate migrates only a bounded copy of the app-owned data. Current package/data pointers, retained history and grant revocation commit together. Exact-package reinstall restores retained current data without restoring grants. Automatic rollback, garbage collection and adoption of legacy execution receipts are unavailable. See `docs/AGENT_FIRST_APP_UPGRADES_2026-10-16.md`.

Tests and limits: see `docs/AGENT_FIRST_ACCEPTANCE_2026-10-16.md` and `packages/runtime-acp/README.md`. Do not interpret a mock-protocol pass or an app-owned draft as a real paid model request or executed scan.
