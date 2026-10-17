# Agent-first acceptance ledger

Baseline `19756cecd5d457c3128af3e7806c5c2990443d69`. Checked items below describe the implemented, synthetic-cloud scope on clean code `93e75a7`; they do not accept complete business parity or an authenticated official runtime. Exact results and original failures: [implementation report](AGENT_FIRST_IMPLEMENTATION_2026-10-16.md), [evidence index](agent-first-20261016/evidence-index.json). The original-path full native rerun with 4 threads completed with 3834 pass / zero fail / original 7 ignored and actual exit 0. The earlier 8-thread full run had 3833 pass / 1 fail / 7 ignored, actual exit 101, and remains unresolved; the 4-thread result does not establish a fix or cause.

## Build and application combinations

- [x] Independent host/UI build with no Asset/Scanning source imports or domain tables.
- [x] Host startup, project, saved chat and unavailable-runtime behavior with zero applications installed.
- [x] Asset only; Scanning only; both installed. Each combination actually executed for current inventory/draft apps.
- [x] Future Echo package built and installed without host source modifications.
- [x] Invalid identity/version/hash/path/symlink/extra entry rejected without inventory/data mutation.
- [x] Worker crash, timeout, disabled/incompatible app and stale page do not break Chat.

## Lifecycle, authority and persistence

- [x] Install grants no permissions; UI close leaves app state unchanged.
- [x] Backend disable removes discovery and rejects new dispatch.
- [x] Cross-project IDs, unexpected arguments and stale grant revisions rejected.
- [x] Revocation during a read withholds result; re-enable/reinstall does not restore consent.
- [x] Data/migration belongs to app; isolated candidate migration failure preserves prior active pointers. Normal old-owner lifecycle writes are not rolled back.
- [x] Uninstall retains data and revokes grants; busy tasks reject uninstall explicitly.
- [ ] Scan work continues independently of Chat/page; cancel/drain/restart state is truthful.
- [ ] Legacy database import is explicit, non-destructive and owner-validated; old desktop retained.
- [x] Host API rejects absent token, foreign/null origin and iframe-origin requests.

## Runtime and Skills

- [ ] Official protocol sources/version recorded; CLI binary version detection verified if available.
- [x] Real child-process fixture verifies incremental updates before prompt completion and persistent session reuse.
- [x] Cancellation, denied/accepted permission, capability-negotiated load, process crash, malformed/oversized data and uncertain completion exercised.
- [x] Host never reads/copies OAuth tokens, performs real login or consumes paid requests in tests.
- [x] Cold start/first text/first tool/total/outcome metrics recorded with fixture label, bounded export and evidence-separated success rates.
- [x] Skills actual UI invoke uses a backend-accepted page budget; UTF8 pagination and failures verified.
- [x] Skill import/selection does not install or authorize executable tools.

## Regression and release limits

- [x] Existing frontend build and relevant Chat/Skills/application regressions rerun on changed source: 1654 pass / 12 macOS skips / zero failures; new independent suite 320/320.
- [x] Changed Rust passed all-target check, focused tests and the original-path full3841 rerun at 4 threads: 3834 pass / original 7 ignored, actual exit 0. Frozen native inputs, binary identity, exact case set and ignored set verified. Old PR7 totals are not reused.
- [ ] The earlier full3841 at 8 threads has one unresolved failure; no diagnosis or production fix is accepted from passing reruns alone.
- [x] Independent review of core imports, data ownership, lifecycle and protocol boundaries.
- [x] Commands, source identity, raw logs, exit status, counts and known failures indexed, including native full02 exit101 and original panic, copy-path repeated diagnostics, interrupted full03 exit143 and original-path full04 exit0.
- [x] Branch push and remote SHA verified; main unchanged at the recorded push. Recheck at final publication.

Pending user/environment acceptance: authenticated official Grok subscription, live latency comparison, actual Mac/Windows GUI/build, real user database migration and complete legacy business parity. These must remain explicit until genuinely tested.
