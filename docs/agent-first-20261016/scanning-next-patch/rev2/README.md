# Prepared bootstrap revision 2 — review fixes, still unapplied

This directory is a separate revision. The original `../prepared.patch` remains byte-identical at SHA-256 `cad7f7b6699330526290d3fbab4545dfb8cc80ffad475ed149e9bfa9cfb0a4a0`; its staged sources and receipts have not been overwritten. Revision 1 has three review blockers and must not be applied in preference to this revision.

**Status: not applied, not compiled, no behavior tests or actual binary smoke executed.** Rustfmt parsing, Node syntax parsing, and `git apply --check` are the only permitted checks during the current frozen native run. This revision is a future implementation candidate, not part of the accepted `3d2eca1` source/binary or its 3841-case run.

All scope limits in the original README continue to apply: new private service drafts only, Unix custody prototype, broad private compatibility schema, exclusive trusted-parent control, no public SDK v2 integration, no scanner execution or old-task adoption, and no financial/lease/birth-signature changes.

## 1. Retained database custody

The service now holds one original database `File` and one SQLite `Connection` for its entire controller lifetime. All subsequent connections use `READ_WRITE | FULL_MUTEX | NOFOLLOW`, with **no CREATE and no URI interpretation**. Production methods do not reopen a database pathname. The only permitted creation is the original initializer inside a newly and exclusively created private first-boot directory.

Every control/read/status/create entry verifies the original file's private metadata and dev/inode, the current regular no-symlink/nlink-one pathname, the connection's main path, SQLite's `SQLITE_FCNTL_HAS_MOVED == 0` with `SQLITE_OK`, and the unique `main.scan_service_store` instance/schema. Reads and cached operation receipts use the retained connection and recheck before returning bytes.

Write methods check custody before beginning, immediately inside their IMMEDIATE transaction before any business mutation, and again before commit. The original draft publisher gains a second narrow callback for this preflight; existing desktop callers supply two no-ops, retaining their original body and SQL. The service's post-publication callback also rechecks before writing its receipt. A missing or replaced DB is an explicit failure; it is never recreated, initialized or silently adopted. SQLite and its original file drop before the controller lock is released.

New regression specifications cover a live foreign replacement, a live missing/renamed database with real `HAS_MOVED` inspection, original/foreign main/WAL/SHM byte preservation on refused entries and after explicit service/SQLite drop, retained-connection `total_changes`, and the custody preflight running inside the original transaction before any draft mutation. These are specifications pending native execution.

## 2. Closed RPC IDs and complete wire budgets

IDs are only nonempty strings up to 128 UTF-8 bytes without control characters, or exact safe signed integers. A malformed, oversized, Boolean, fractional or non-scalar ID becomes `null` in the protocol error; the input is never echoed into an error response.

Every success and error passes through one envelope encoder. Its exact serialized JSON **plus the NDJSON LF** must fit 64 KiB. Oversize success results become a fixed bounded error, error codes have a closed bounded spelling, and the final write encoder enforces the same limit again. The budget includes the ID, JSON escaping, protocol fields and errors, rather than measuring only a successful result value.

New regression specifications include a 120 KiB invalid Unicode ID, object/array IDs, controls and unsafe numbers; all yield a small null-ID error without database writes. Separate tests measure actual encoded success/error frames, escaped bounded IDs, an under-64-KiB result whose full envelope exceeds the limit, and rejection of an unbounded value passed directly to the final writer.

## 3. Literal targets cannot expand through desktop tokenization

The bootstrap accepts a list of literal URLs, not the desktop paste-list syntax. It rejects every whitespace, comma/full-width comma, semicolon/full-width semicolon, and edge quote/angle/punctuation wrapper that the original owner would split or trim. This is a deliberately narrower new bootstrap input; the original desktop tokenizer is unchanged.

Each resulting trailing-slash-normalized literal is checked for HTTP(S), host, no userinfo and no fragment; the deduplicated set is capped at 32. Before its operation receipt is published, the service compares the original owner's **actual** `sentinel_targets` set to that validated set inside the same original transaction. A changed set rolls the entire publication back. Encoded delimiters remain literal encoded URL content.

New regression specifications cover an apparently valid first URL containing separators followed by a credential URL, one raw element expanding to 33 targets, whitespace/full-width separators and trim wrappers, zero logical/physical publication on rejection, and a positive 32-target case that checks the actual owner output, case, `www`, encoded delimiters and absence of engine runs.

## Pending verification

The original eight Rust cases remain; eight review regressions bring the proposed selector `scanning::service_bootstrap::tests::` to 16 cases. The actual-binary smoke remains required and has not run. No tests are ignored or replaced to claim acceptance.

After the current native failure is resolved and root releases the source window: review this complete patch, type-check all targets, perform one coordinated native test build, verify all 16 names exist, run the exact new/old owner cases, link the actual binary and execute its synthetic stdio smoke. Until then there is no proof that this revision compiles or its assertions pass. The generic native SDK transport, full engine admission, provisioning, durable long-task ownership and reconnect/crash reconciliation remain separate work.
