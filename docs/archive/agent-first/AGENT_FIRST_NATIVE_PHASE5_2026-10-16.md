# Native applications: phase 5 checkpoint

This is a development checkpoint, not complete application acceptance. Work remains on
the independent `feature/agent-first-app-platform-20261016` branch. No main merge or
user Mac modification is part of this change.

## Changes in this batch

Assets retains its original Exposure import owner and limits: 20 files, 2 MiB per
file and 8 MiB per batch. Five operator-only upload actions transfer verified private
chunks, then admit one original import. The durable commit claim is never retried
automatically after an unknown result. The four existing layout preferences have an
instance-bound CAS store. An immutable project-bound selection receipt carries at most
200 canonical URLs to Scanning; it does not create a scan or grant execution.

Scanning imports a bounded immutable private source reference and calls the existing
Source/Greybox/CI workbench owner. Its service membership is published in the original
task transaction. Source reference import is operator-only; workbench model arguments
contain a reference rather than an arbitrary path or Git history. Original Scope,
startup, financial, lease and retry decisions remain with the original engine.

The Scanning manifest now declares `ui.handoff`. Actual installed-browser run 06
correctly refused the undeclared target before this fix. The declaration has no model
tool or business action and grants no draft or execution authority. The SDK reads the
new manifest at process start; the existing phase-5 binaries can therefore be packaged
again. Fresh installation and project grants still require a new browser run.

## Actual validation

The [sealed native evidence](agent-first-20261016/native-phase5/summary.json) records:

| Execution | Actual result |
| --- | --- |
| First all-targets check | exit 101; `usize: ToSql` at one new Source SQL binding |
| Checked integer-conversion fix and second all-targets check | exit 0, 76.290 seconds |
| Standalone Rust SDK | 32 passed; all original 31 names retained |
| Two dev service binaries | build exit 0, 83.798 seconds; clean system-library link checks passed |
| Library no-run, opt level 1 | exit 0, 431.264 seconds |
| Actual library case list | 4006 unique cases; previous 3942 and original 7 ignored names retained |
| Selected core cases | 347 passed, 20.38 seconds |
| Original paid-history neighbors | 15 passed, 5.75 seconds |
| Full library suite and release | **not executed for this source** |

All 64 new Rust cases ran in the selected 362, with no failures or ignored cases.
The 1820 native input fingerprints and 21 protected artifacts were unchanged across
the recorded build and test commands. Original failure evidence is retained alongside
the successful retry. Binary and source archive bodies are excluded from Git; their
cloud paths and hashes are recorded.

The separate [Scanning Node receipt](agent-first-20261016/native-source-handoff-node/result-full.json)
records all 23 selected owner cases passing after the handoff declaration, with
unchanged before/after inputs and a fresh author build. Its packaging fixture is not
native execution. Browser run 06 was 12/13 before the declaration fix; it is not
reclassified as a pass. The next complete 18-case installed-browser run is pending.

## Ownership and remaining boundaries

| Owner | Owns | Dependency boundary |
| --- | --- | --- |
| Assets | Inventory, ownership, Exposure, original jobs, private configuration/layout and selection provenance | Produces a typed handoff; never reads the Scanning database |
| Scanning | Drafts, workbench execution, findings, attempts and original engine authority | Receives reviewed URLs/source references; never adopts foreign asset integer IDs |
| Chat and project workspace | Current project selection, conversation access and chat lifecycle | Queries permitted app capabilities through the Host |
| Skills | Import, package provenance, integrity and revocation | Scanning produces candidates; Skills owns review/application |
| Host and SDK | Package/lifecycle custody, project grants, generic page transport, navigation and artifacts | No asset/scan SQL, business defaults or model execution rules |
| Shared | Reused UI primitives and finite transport types/utilities | No concrete business implementation or general Agent rewrite |

Known Source follow-ups remain explicit: this first import is limited to 128 files,
128 KiB per file and 512 KiB total; it does not match the original source-tree scale.
Fullscale staged upload must verify bytes outside a long SQLite write transaction.
A plain source directory below a Git ancestor must use an explicit no-repository
capture seam so it cannot inherit the application's unrelated Git HEAD. These are
required fixes, not accepted behavior.

Full mature Scanning readers/operator workflows, isolated interactive authentication,
Assets scan-result backlinks and old Node application data migration are still being
implemented. First native custody admission is not domain-data migration. Final
full-library, release, installation, browser and whole legacy regression must run on
one coherent final source. Official Grok authenticated model/latency acceptance still
needs an official fresh-profile login; no account caches or API-key fallback are used.

The new Dashboard, AI refinement and incremental discovery remain deferred by the
user. They are not part of this implementation checkpoint.
