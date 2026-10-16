# Controlled navigation prepared implementation

This directory contains a prepared-only addition of `packages/agent-host/src/navigation.mjs` and its focused unit tests. Neither file was applied to the repository by this task. `source-ready-01.json` pins both files, the patch, the actual 46-case unit log and its limitations. `prepared.patch` passed `git apply --check --whitespace=error-all` without mutation.

The module consumes the existing Host `AppManager` metadata interface. It never calls a worker, fetches a URL, spawns a process, queries a domain database, or treats payload fields as authority. A current native-v2 page requires its declared and granted `ui.external.open` or `ui.handoff` permission. The target requires its own current `ui.handoff` declaration/grant in the same Host project. Both sides bind page, version, app revision, grant revision and digest, and are rechecked at confirmation and delivery. Disabled, changing, upgrading, stopped and unresolved owners fail closed.

## Locked API

`new PageNavigation(apps, {now=Date.now, ttlMs=30000, maxPending=64})`

Every scope is exactly `{projectId,pageId,version,expectedAppRevision,expectedGrantRevision}`.

- `prepareExternal(appId,scope,{url})` returns `{token,kind:'external',projectId,url,expiresAt}`.
- `prepareHandoff(appId,scope,{targetAppId,targetPageId,payload})` returns `{token,kind:'handoff',targetAppId,targetPageId,projectId,preview,expiresAt}`. `preview` is the complete cloned payload.
- `confirm(token,scope)` consumes preparation and returns `{kind:'external',url}` or `{kind:'handoff',targetAppId,targetPageId,projectId,deliveryToken}`.
- `consumeHandoff(deliveryToken,targetAppId,targetScope)` consumes delivery and returns exactly `{protocol:'oviraptor.handoff.v1',purpose:'draft-only',sourceAppId,projectId,payload}`.
- `cancel(token,scope)` cleans up a matching pending confirmation and returns `{cancelled:true}`. It can still clean up after grant revocation.
- `close()` clears tickets and disables admission.

Both token types have one use, share the original expiry and count toward the maximum of 64. TTL is at most 60 seconds. Restart invalidates all tickets. Failed confirmation/delivery consumes its ticket so an old dialog cannot regain authority after settings change back.

## Parent and transport obligations

Routes remain protected by the existing Host session/Origin boundary and accept closed envelopes. The navigation JSON body requires 512KiB plus at most 4KiB envelope allowance. Tokens must remain in the parent window, never app frames or model results. The trusted parent checks current page, project, nonce, epoch and revisions before and after every await, shows the entire payload for review, and explicitly confirms or cancels. The Host cannot independently prove a physical user gesture.

External confirmation only returns a canonical HTTP(S) URL. The parent offers a visible `noopener noreferrer` link for a separate manual click and reports `offered:true`, not that navigation occurred. The Host performs no target request. Sandboxes and CSP stay unchanged.

Handoff opens only a registered target page, consumes using its current scope and supplies the SDK a token-free `init.handoff`. The SDK validates/deep-freezes the envelope. Target business code must validate its finite draft contract and require explicit review before any draft/execution operation. An embedded payload `projectId` or similar field is untrusted data and cannot change the Host-bound envelope project. This module creates no business draft and performs no scan.

Payloads are complete plain JSON objects: actual encoded UTF8 JSON at most 512KiB, scalar strings4096bytes, array1024items, object64keys, depth8, total8192nodes. No truncation; oversize returns `host_navigation_payload_too_large`. Accessors, cycles, exotic/nonfinite values and prototype-sensitive keys are rejected. Tests preserve a full 100-item selection containing about2048bytes per URL. A domain may impose a separate explicit target limit.

## Actual validation

`node --test --test-concurrency=1 staged/packages/agent-host/test/navigation.test.mjs` executed 46 distinct cases: 46 pass, no failures/skips/cancellations,111.03473ms. This was standalone unit execution using mutable Host metadata fixtures. It did not exercise real HTTP routes, browsers, native workers, business persistence or external navigation. Root's integrated Host/UI suites remain required after application.
