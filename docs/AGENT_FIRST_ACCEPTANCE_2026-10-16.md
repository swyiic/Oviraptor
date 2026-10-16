# Agent-first acceptance ledger

Baseline `19756cecd5d457c3128af3e7806c5c2990443d69`. All boxes start unaccepted. Record exact commands/results after execution; design and mock fixtures are not end-user runtime acceptance.

## Build and application combinations

- [ ] Independent host/UI build with no Asset/Scanning source imports or domain tables.
- [ ] Host startup, project, saved chat and unavailable-runtime behavior with zero applications installed.
- [ ] Asset only; Scanning only; both installed. Each combination actually executed.
- [ ] Future Echo package built and installed without host source modifications.
- [ ] Invalid identity/version/hash/path/symlink/extra entry rejected without inventory/data mutation.
- [ ] Worker crash, timeout, disabled/incompatible app and stale page do not break Chat.

## Lifecycle, authority and persistence

- [ ] Install grants no permissions; UI close leaves app state unchanged.
- [ ] Backend disable removes discovery and rejects new dispatch.
- [ ] Cross-project IDs, unexpected arguments and stale grant revisions rejected.
- [ ] Revocation during a read withholds result; re-enable/reinstall does not restore consent.
- [ ] Data/migration belongs to app; migration failure preserves prior active version.
- [ ] Uninstall retains data and revokes grants; busy tasks reject uninstall explicitly.
- [ ] Scan work continues independently of Chat/page; cancel/drain/restart state is truthful.
- [ ] Legacy database import is explicit, non-destructive and owner-validated; old desktop retained.
- [ ] Host API rejects absent token, foreign/null origin and iframe-origin requests.

## Runtime and Skills

- [ ] Official protocol sources/version recorded; CLI binary version detection verified if available.
- [ ] Real child-process fixture verifies incremental updates before prompt completion and persistent session reuse.
- [ ] Cancellation, denied/accepted permission, capability-negotiated load, process crash, malformed/oversized data and uncertain completion exercised.
- [ ] Host never reads/copies OAuth tokens, performs real login or consumes paid requests in tests.
- [ ] Cold start/first text/first tool/total/outcome metrics recorded with fixture label.
- [ ] Skills actual UI invoke uses a backend-accepted page budget; UTF8 pagination and failures verified.
- [ ] Skill import/selection does not install or authorize executable tools.

## Regression and release limits

- [ ] Existing frontend build and relevant Chat/Skills/application regressions rerun on changed source.
- [ ] Appropriate native checks rerun if Rust changes; old PR7 totals not reused.
- [ ] Independent review of core imports, data ownership, lifecycle and protocol boundaries.
- [ ] Commands, source identity, raw logs, exit status, counts and known failures indexed.
- [ ] Branch push and remote SHA verified; main unchanged.

Pending user/environment acceptance: authenticated official Grok subscription, live latency comparison, actual Mac/Windows GUI/build, real user database migration and complete legacy business parity. These must remain explicit until genuinely tested.
