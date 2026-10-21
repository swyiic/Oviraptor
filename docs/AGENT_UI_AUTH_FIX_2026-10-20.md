# Chat UI, authorization and project reads — 2026-10-20

## Scope and observed failures

The running `/Applications/Oviraptor.app` showed nested user bubbles, large whitespace and `host_provider_request_failed`. A private browser fixture reproduced a missing Teleport target followed by Vue `__vnode` errors. Read-only Host metadata showed no saved application grants. Existing desktop deliveries retained an older Host because the new Host imported an Assets SDK file absent from the staged package.

## Changes

- Defer chat history Teleports until their targets mount. Use the shared message component as the single bubble renderer, remove inherited template whitespace and duplicate headings, collapse completed turn detail, and wrap compact controls at narrow widths.
- Load saved messages independently of task metadata. Handle background failures without unhandled promise rejections. Ignore stale model-settings responses and unregister the settings listener on unmount.
- Keep an explicitly accepted file permission for this conversation and runtime identity when the user selects Remember. Support clearing it. Preserve separate read/write permissions, project isolation, request/run binding, runtime invalidation and checks before file effects.
- For a desktop project data question, request the actual Assets `inventory.read` grant before the model request. Reuse the saved project grant. File access does not stand in for inventory access. A changed runtime disclosure remains an explicit renewal.
- Synchronize project-scoped counts from the local discovery index: SHA-256 fingerprints, certificate names and other seed sources. Missing/unextracted indexes are not reported as proof of absent certificates. The tool receives aggregate counts, not arbitrary extra JSON or credentials.
- Package the desktop snapshot adapter inside the SDK so all Host ESM imports resolve in a relocated distribution. Full desktop build now stages the current Host.
- Make the explicit Update bundled components action upgrade an older package through the existing stopped-worker/data-generation upgrade flow. Startup does not upgrade the old database. Retain the prior package/data generation and report failures; component upgrade requires renewed application disclosure once.
- Distinguish provider request-format, quota, rate limit, authentication, missing endpoint/model and server errors. Do not reveal raw upstream response bodies or replay tools automatically.

## Verification boundaries

All write-path verification uses temporary stores and synthetic fixtures. The existing user database was read only for diagnostic metadata. No VPN settings changed. The installed App was observed but not replaced or relaunched. Browser layout evidence is synthetic, not a live model or Hunter acceptance test.

Private validation logs are under `/private/tmp/ovi-chat-fix12-*`; these are not repository artifacts. `TMPDIR=/private/tmp` is used for tests because macOS `/var` aliases are intentionally rejected by canonical-path authority checks. Exact counts and the pushed revision are reported after the final checks finish.

## Completed focused checks

- `npm run build:desktop`: type checking, production UI build and current 99-file Host distribution staging passed.
- Host suite: 515 passed, 17 skipped, 0 failed (`TMPDIR=/private/tmp`).
- UI/SDK/Assets suite: 265 passed, 1 skipped, 0 failed.
- Asset UI regression: 127 passed; shell/configuration/conversation regression: 140 passed; final distribution/chat/permission checks: 20 passed.
- Local discovery/pagination Rust suite: 48 passed. A broader Rust run found two old full-JSON-equality assertions in profile selection tests; category defaults intentionally include builtin/provenance fields. Their updated assertions retain fallback identity checks for all eight categories; both targeted regressions passed on the current source. The broad run was not completed and is not claimed as a full Rust-suite pass.
- Browser fixture: 1280px viewport, no horizontal overflow, one user bubble, both messages readable, no Vue warnings/errors after a fresh load. Authorize/continue closes the synthetic request panel. Screenshots are private local evidence.

The existing `/Applications` App is not updated by a Git push. Live configured-provider failures and Hunter connectivity remain separate real-environment acceptance boundaries.
