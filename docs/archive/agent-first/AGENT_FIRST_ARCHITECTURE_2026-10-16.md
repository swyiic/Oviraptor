# Agent-first host, applications and Grok ACP

Status: approved implementation starts 2026-10-16; acceptance is tracked separately. Baseline: `19756cecd5d457c3128af3e7806c5c2990443d69`, branch `feature/agent-first-app-platform-20261016`. One repository; no merge to main is authorized. Historical WIP, local-source integration and PR7 remain intact.

## Actual starting point

The current desktop has business owners, compile-time bundled manifests and lazy UI imports. It does not have installable application packages. UI preferences do not revoke conversation grants. Chat tools directly import Assets and read sentinel tables; project queries, database startup and diagnostics also retain business dependencies. This design changes those boundaries rather than relabelling the existing registry.

Read together with `PROJECT_AWARE_AGENT_CHAT_2026-10-14.md`, `BUSINESS_MODULE_BOUNDARIES_2026-10-15.md` and the PR7 implementation report. Old test totals do not accept new source.

## Deliverable layout and migration

* `packages/app-sdk`: versioned manifest, JSON-RPC contracts, validation and package build helpers. No asset/scan queries, models or execution policies.
* `packages/agent-host`: independently built local host. Owns projects, conversations, runtime sessions, application inventory and explicit grants in its own database. It must not import Assets/Scanning or query their tables.
* `packages/agent-ui`: host-only UI. Chat first; project/history/settings/application management. Application pages load through a sandboxed, isolated page boundary.
* `packages/runtime-acp`: an adapter to the official existing Grok runtime, not a new model executor or provider impersonation.
* `examples/echo-app`: first real separately packaged worker/page. Installation must work without edits to host source or a static application registry.
* `apps/assets`, `apps/scanning`: independently built application packages, each owning data, migrations, tool/page implementation and tasks. Explicit migration bridges preserve the existing desktop until equivalent capabilities pass acceptance. A limited read bridge is not declared full migration.

The existing Tauri application and its model configuration/chat/Skills remain available during migration. Domain-neutral rendering, message semantics and configuration contracts are reused where applicable. There is no destructive automatic database conversion or fallback to guessed configuration defaults. The new ACP path lets the existing official agent perform model intelligence directly, with no extra model intermediary. The legacy desktop remains the compatibility route while its complete business functions move behind application adapters.

## Public application contract v1

A directory package is the first supported installation format. It contains `manifest.json`, an executable worker entry, pages/resources, and a hash inventory. Required identity is schema version, DNS-style app ID, semantic app version, compatible host API range, data schema version, tool/page declarations, permissions, lifecycle and SHA256 file list. Unknown/unsafe paths, symlinks, missing/extra files, conflicting identities, invalid schema and incompatible versions fail before activation. Package bytes are copied to host-managed immutable version directories; execution never uses the mutable import directory.

Hash validation detects content changes; it does not authenticate a publisher. The first installer accepts explicitly selected trusted local application code. Child processes provide crash/lifecycle separation, not an advertised OS sandbox. Filesystem restrictions are applied when supported; arbitrary untrusted executable packages are outside this first release. No credentials are copied into application environments.

Workers speak bounded newline-delimited JSON-RPC over stdio: `describe`, `lifecycle/start`, `lifecycle/stop`, `tool/call`. The host verifies that runtime declarations match the installed manifest. Public tool names are app-ID qualified. The host binds project ID, grant revision and operation ID; model/page arguments cannot replace them. All results remain untrusted data and are size/schema checked.

Applications own app-specific data directories and transactional migrations. Installation and data migration failure cannot replace the active version. Uninstall removes availability/package registration and revokes grants; data is retained by default. Reinstall does not resurrect grants. An incompatible schema or failed worker is visible without breaking core Chat.

## State, permission and task ownership

UI close only hides a page. Disable stops new admissions and removes tools; it does not silently cancel accepted durable work. Install does not grant permissions. Effective capability is the intersection of installed, enabled, compatible, healthy, explicit current-project grant and current runtime disclosure. Dispatch repeats checks, and read-after-revocation bytes must be withheld.

Long scan jobs belong to the scanning application service and its durable task records, not a chat turn, browser tab or SSE connection. Uninstall while jobs are running is rejected by default with an explicit busy result. Cancel/drain/detach must be a separately visible supported policy; forcibly killing a busy worker is not a successful uninstall. Historical data and references remain readable only through authorized retained-data/migration paths.

The host process binds loopback. APIs require a per-process random session token, reject cross-origin and opaque `Origin:null` requests and do not enable CORS. App frames omit same-origin privilege and receive no host token. Page messages are allowlisted against frame identity and a navigation nonce, never forwarded as arbitrary host API calls.

## Agent discovery, Skills and analysis

The host exposes a cheap installed/enabled/permitted capability summary and only the selected permitted schemas. Returning a tool name as text does not register it: the runtime adapter must connect the actual callable protocol. Domain services retain argument validation, scope, costs and execution authority. The SDK does not absorb domain implementations.

Skills remain untrusted guidance and reference resources. Installation/selection cannot create tool authority. Preserve source/license/content fingerprints, per-conversation consent and revocation behavior. Fix the current resource UI default of 65536 versus backend maximum 8192 and verify the actual invoke path. Package uninstall/upgrade must not silently reactivate previous grants.

Existing exact asset statistics and scoped reads remain reusable through the Assets owner. A sample or saved scan heading is not a complete analysis. Structured evidence snapshots/results, versioned references and approval/execution bridges are separate deliverables; they are not implied by installing an app or streaming a response.

## Grok ACP runtime

Official sources checked 2026-10-16:

* https://docs.x.ai/build/cli/headless-scripting — `grok agent stdio`, ACP JSON-RPC, protocolVersion 1 in the official example; assistant output arrives in `session/update`, while `session/prompt` returns completion metadata. Automated runs should use `--no-auto-update`.
* https://x.ai/news/grok-opencode — official subscription integration for OpenCode. This establishes that integration's support, not transferable OAuth credentials or equal performance in another host.
* ACP protocol reference/schema must be checked for session loading, cancellation and permission response details; record exact source/version with adapter fixtures.

One persistent child handles initialize, official cached authentication, sessions and streaming. Never read/copy OAuth tokens, borrow another client's identity, auto-login or enable always-approve. Normal local authentication remains in the official CLI. No real account, paid model request, external target or security-setting change is part of cloud acceptance.

Adapter state covers unconfigured/starting/ready/failed, negotiated protocol/capabilities, request IDs, bounded framing, one in-flight prompt per session, session update attribution, explicit permission decisions, cancellation acknowledgements, process exit, deadlines and restart. Resume is attempted only when advertised, never automatic replay of an uncertain prompt. Unknown filesystem/terminal methods are rejected unless a separately implemented scoped service is granted.

Measure process cold start, first assistant chunk, first tool event, total turn time and success/failure/cancel outcome. Fixture latency is functional evidence only. Matching a model name does not establish terminal-equivalent runtime behavior or speed; authenticated same-condition comparison is pending user participation.

## Ordered implementation and evidence

1. Commit this design and acceptance checklist before implementation commits.
2. Correct Skills resource contract; build protocol/SDK and install/enable/disable/uninstall the real Echo package with isolated data.
3. Build/start the independent host and connect real ACP process transport using synthetic process fixtures; expose streaming, cancellation, permission and session state in UI.
4. Migrate Assets and Scanning behind independently installable packages with explicit legacy compatibility and durable-task ownership. Preserve full old desktop while parity is incomplete.
5. Run absence/combinations, malformed/crashed/disabled packages, permission/project boundaries, migrations and old-feature regressions. Record exact source, commands, exits, counts and logs, then push the independent branch. No untested completion claim or main merge.

The first progress checkpoint is 2026-10-16 03:30 UTC / 11:30 Beijing. It reports actual commits, completed checks, failures and estimates, not a promised completion deadline.
