# Native phase2–4 audit delivery

This is a lightweight, read-only evidence copy for a later Git documentation commit.
It contains no executable binary, SQLite database, or source tar body. Absolute paths in the
original receipts identify the original cloud environment; copied evidence paths are relative.

## Recorded result scope

- Phase2: first all-targets compile really exited101 (E0308). The later compile succeeded.
  Actual3938 library names were listed. Core279 executed:276passed,3failed. Paid15 was planned
  but did not execute after the core failure. The3panic bodies remain in the complete raw log.
  The GTK-linked Assets dev binary was a real runtime limitation, not installed acceptance.
- Phase3: all-targets and opt1 library no-run succeeded. Actual3942 names preserved3938 and
  added4. Core283 and original paid15 all passed, every selected case exactly once.
  This is298 focused cases, not the full3942 suite. It applies to the exact1810 recorded
  source inputs before the later2-file artifact wire change.
- Phase4: nextOffset is an integer end offset even on EOF. Only those2 source paths changed.
  New all-targets and actual Assets dev link succeeded. Clean dependency inspection found
  only4standard Linux libraries. No new library no-run/list/test/full/release was executed.
  Browser acceptance is separate. The old phase3 passed library binary remains preserved.

## Reading and verification

Start with `summary.json` and `delivery-index.json`. `originals/` contains byte-exact copies
of all regular small files referenced by the3sealed original indices, plus the3indices.
Original source tar bodies are omitted with their original SHA/size/path recorded.
The original indices are unchanged historical objects; their absolute references may name
excluded large originals. `delivery-index.json` maps copied bytes to those originals.
`derived/` contains plain case-name lists, exact recorded source fingerprint maps, and a
fresh read-only hash observation of19protected artifacts plus the2latest Assets paths.

Run `python3 verify-delivery.py` after copying this directory. It only hashes files and
checks the delivery index. It does not invoke Cargo, native binaries, or project tests.
No helper stored in `originals/` should be run as an acceptance command: they are historical,
may have obsolete source policies, and do not authorize a new run.

No new passing test count is created by this packaging step. Existing red logs, actual tool
exits, failed partial archive attempt, scope restrictions, source-HEAD changes during JS/docs
work, and excluded-runtime limitations remain visible. No source, handoff, prior evidence,
or protected binary was modified. Full/default library, release, and final whole-tree scope
must be established separately under the parent's next explicit source freeze.
