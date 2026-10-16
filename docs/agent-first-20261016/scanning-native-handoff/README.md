# Scanning handoff and closed browser build evidence

The actual Node suite ran 20 cases: 20 passed, zero failed/skipped, outer exit 0. Both SDK input hashes still match that run. The artifact digest now covers the actual esbuild import closure, including `handoff.mjs`; packaging remains static verification.

The receiving page treats the bounded asset-selection envelope as untrusted data. Reception and review send no mutation, separate draft creation excludes provenance, and execution remains separate. The new tests also refuse mismatched projects, extra/invalid selection fields, invalid target sets and changed page context.

Runs 01 and 02 retain their original failures. Run 02 demonstrates that a changed SDK dependency is refused until author rebuilding. No browser or native runtime result is claimed for this new handoff batch; the preceding page/native evidence remains in `../scanning-native-page/` with its exact binary and scope.
