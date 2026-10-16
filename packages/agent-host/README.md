# Independent Agent host

This is an opt-in migration host in the same repository. The legacy desktop stays available while complete Assets/Scanning parity is unfinished. Core storage does not read or initialize application domain tables. It uses Node 24's built-in SQLite and starts with zero applications and no connected runtime.

From repository root:

```sh
npm run build:agent-first
npm run build:app-packages
node .agent-build/agent-first/packages/agent-host/src/cli.mjs --data-dir /absolute/private/host-data
```

Open the loopback URL printed by the host. Install a generated package directory through Applications. Installation starts disabled and grants nothing. Enable it and explicitly choose project permissions/data disclosure. Closing a page is distinct from disabling the app. Disable removes new capability admission and revokes grants without force-cancelling accepted work. Busy uninstall is rejected; successful uninstall keeps application data and immutable package bytes. Reinstall requires new grants.

To select the official CLI runtime explicitly, add `--runtime grok-acp`. This starts no model request until a message is sent. The CLI must already be installed and authenticated by the user using its official flow. The host never reads/copies OAuth tokens. Cloud tests use synthetic ACP processes only; do not run authenticated account tests without the user's participation. No `--always-approve` or security-setting changes are used.

Applications run separate Node workers and own data/migrations. The current trusted-local-package installer verifies every file and applies Node filesystem permissions; it is not a secure sandbox for arbitrary third-party native code or networking. Package identity/hash integrity is not publisher signature verification. OS-level distribution/signing and upgrade rollback are not yet accepted.

Grok receives a conversation-bound stdio MCP server. Tool discovery is the current intersection of installed/enabled/healthy app, declared schema and explicit project permission. The host binds project and grant revision, validates before dispatch and withholds results after revocation. Application pages have no host API token and cannot turn page messages into arbitrary API calls.

The new ACP host does not yet replace every legacy Chat/Skills/model-profile feature. Legacy provider configuration remains in the existing desktop and is never silently converted into Grok subscription credentials. Full configuration/Skills migration and complete scanning execution must pass their own acceptance before switching the default application.

Tests and limits: see `docs/AGENT_FIRST_ACCEPTANCE_2026-10-16.md` and `packages/runtime-acp/README.md`. Do not interpret a mock-protocol pass or an app-owned draft as a real paid model request or executed scan.
