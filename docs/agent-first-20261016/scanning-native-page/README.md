# Scanning native page and package checkpoint

This checkpoint commits the native Scanning author package/page and capability ledger, with no Rust source. [The evidence index](evidence-index.json) binds all 13 source files and byte-identical copied receipts/logs/screenshots. All eight retained Node-v1 source files remain byte-identical to the preceding commit.

- Node 24: **16 passed, 0 failed, 0 skipped**. [Actual TAP](node/raw-run-04.log). This includes persisted v1 drafts, real ELF package hashing and UI controller dispatch; controller fixtures do not prove the scan engine executed.
- Real Chromium + production Host/SDK 2 + built Rust service: **7 passed, 0 failed**, outer exit 0. [Result](browser-current/result.json), [process receipt](browser-current/raw-process.process.json), [narrow screenshot](browser-current/scanning-narrow.png), [wide screenshot](browser-current/scanning-draft-wide.png). The unchanged opaque sandbox supports explicit inline-DOM confirmation and button actions. Counted tables verify no scan attempts, roots or dispatches; no external/provider/account requests occurred.
- Actual packaged Rust SDK-2 subprocess: **6 chains passed**. [Result](native-sdk2-smoke/result.json). It covers describe/start, original atomic draft/operation replay, tools and task isolation, missing-model original admission refusal, operator configuration without a provider call, and reopening the same private store. It does not test a successful long-running scan.

The tested development executable is SHA256 `7c2f81a000582ae314318824bea1e575f50b26204a2193c6e7b8149b255ada56`. It predates the subsequent commit-hook error propagation and exact-child selector fixes. Those Rust changes require their own rebuild and validation; this commit does not borrow these earlier successes as final native acceptance.

Original failures are retained: [opaque-module preflight](browser-first-cors-failure/result.json), [blocked form submission](browser-form-failure/result.json) and its [screenshot](browser-form-failure/07-scanning-real-native-url-draft-list-and-no-execution-failed.png). Earlier Node raw logs are preserved unchanged. The original native focused result also remains [276 passed / 3 failed](native-known-red/raw-core.log); the child selector's zero-case failure and two Ownership fixtures were not hidden or marked ignored.

The native page now runs a deterministic author-built inline IIFE/CSS artifact: no runtime ES-module/CSS request, fetch, native modal or form submission. Package installation executes no author build hooks. Runtime sandbox, CORS and token policy are unchanged.

This is the explicit-URL slice. [The 113-command capability ledger](../../AGENT_FIRST_SCANNING_CAPABILITY_PARITY_2026-10-16.md) lists remaining Source/Greybox/CI, identities, skills/tools, detailed results/logs/artifacts, advanced policies and operator financial/directive recovery. It is not a claim of full Scanning parity or safe adoption/re-signing of old executable/database births.
