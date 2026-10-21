# Native Scanning package, SDK 2

This package uses the existing Rust Web investigation engine in its own application process and private compatibility database. The separate [Node v1 package](README.md) remains draft-only and unchanged. Installing a native package does not grant targets, credentials, tools, budget, lease or replay authority.

Author an installable package using a real newly built `oviraptor-scan-service` executable:

```sh
node apps/scanning/build-native-page.mjs
node apps/scanning/package-native.mjs --binary /absolute/oviraptor-scan-service --output /absolute/new-package
```

The first command is an author-time esbuild operation using repository dependencies. It produces `native-bundle.html`, a deterministic inline IIFE and CSS document suitable for an opaque `sandbox="allow-scripts"` iframe. Its embedded input manifest comes from esbuild's actual transitive browser import closure, plus the author template, stylesheet and build script. The packager only hashes this closed input set and copies the current document to package `native.html`; a changed SDK dependency requires a fresh author build. Runtime needs no external script/module/CSS import, fetch, token, modal permission or same-origin exception. The installer never executes author build hooks.

The package hashes the actual executable and the same five worker/browser-location resources checked by the engine's release-pinned resource verifier. The existing Node package cannot spawn it. A generic SDK-2 Host launches the verified immutable native entry directly, with trusted private data/package roots and a sanitized environment. Native service file custody is currently Unix-specific; real executable/build and protocol acceptance must be recorded for each shipped platform. Linux artifact measurements from the bootstrap phase do not establish another platform's support.

The page receives only the public SDK page context. It finds this service's saved project tasks, creates explicit URL drafts, reads current status, and offers separate start/pause/resume/retry/draft-delete actions. Start/resume/retry use an explicit page-local confirmation bound to the selected task. Cancelling a confirmation sends no action; closing the page only detaches its observer. Unknown mutation outcomes retain their operation ID and block blind retries. An old accepted receipt is historical, not proof of completion.

An Assets handoff uses `scan.targets.v1` in the generic project's `draft-only` handoff envelope. Scanning independently validates the exact project, HTTP(S) targets, mode and bounded asset-selection metadata. The page displays the complete selection ID, digest, source instance and count as untrusted provenance; they grant no ownership or execution authority. Receiving the handoff performs no mutation. Explicit review fills the existing draft form, a separate click creates the draft, and execution still needs its own confirmation and the original engine gates. A changed trusted page context invalidates the old handoff and task confirmation and requires reopening the page.

Model endpoint/deployment/key setup is an operator-only application action. It uses the original profile persistence and the single existing resolver, performs no provider request, and never places credentials in tool schemas or results. New task startup still passes the original authorization, model, resource, tool approval, lease, budget and financial gates. Old executable/path births are not adopted or re-signed. Paid or indeterminate continuation remains refused by the original owner.

Current native entry covers explicit-URL Web tasks. Source/Greybox/CI, full workbench options, auth sessions, skill/tool configuration, detailed results/artifacts/logs and operator financial/directive recovery still need adapters and actual acceptance. See [the complete capability ledger](../../docs/AGENT_FIRST_SCANNING_CAPABILITY_PARITY_2026-10-16.md). A thin URL page or a busy-label fixture does not establish full Scanning migration.

Unit and author-package verification:

```sh
node --test apps/scanning/test/*.test.mjs
```

For an actual newly built SDK-2 worker, a separate explicit smoke packages and launches that exact binary, exercises real protocol/SQLite/draft isolation, refuses startup before model setup, verifies zero attempts/runs/dispatches, configures a synthetic local model without calling it, and reopens the same store:

```sh
node apps/scanning/test/native-service.smoke.mjs --binary /absolute/new-binary --evidence-dir /absolute/new-evidence-directory
```

The smoke writes a binary/manifest-bound result receipt. It deliberately does **not** demonstrate a successful long scan, paid provider operation, external target, complete migration, or safe replay after a crash. Original native tests and actual fixture-backed long-owner/page-detach tests remain separate acceptance layers.
