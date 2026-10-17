# Independent app upgrades and lifecycle

This document specifies the upgrade contract for the Node Agent host. A versioned manifest alone does not implement an upgrade: the initial host rejects an already installed app ID, uses a fixed `data/<id>` directory, and has no transactional migration or retained-version pointer. The behavior tests in `packages/agent-host/test/upgrades.test.mjs` exercise the contract below with synthetic local packages and real app worker processes. Test results must be reported separately from this design.

## Host and app ownership

The host owns immutable installed package bytes, the current package/data pointers, process supervision, project grants, and registry transactions. The app owns its data format and migration code. A migration runs through the candidate package's ordinary `lifecycle/start`, with only its installed package and a copied candidate data directory available through Node filesystem permission flags. The host never opens or rewrites app business databases. Node filesystem flags are not a network sandbox; arbitrary hostile packages and rollback of external side effects are outside this contract.

The public operation is `AppManager.upgrade(id, sourceDirectory, {expectedRevision})`. The source is a directory package, validated and copied into the content-addressed package store before any of its code executes. The app must already be disabled. The revision is mandatory and must match the current app row; the host must recheck the same identity and revision at commit. An absent or invalid revision is rejected before executing app code.

Upgrade requires the same app ID, a strictly newer semantic version, a supported manifest/host API, and a nondecreasing `dataSchemaVersion`. Same-version replacement, implicit downgrade, changing an app ID, or lowering the declared data schema are rejected. Rollback is a separate explicit operation and must not be disguised as an upgrade.

## Persistent data generations

`applications.data_path` is the internal current data directory and `applications.data_version` is an opaque host generation identifier. Neither is a project grant. The manifest's integer `dataSchemaVersion` describes app data format; it is not the generation identifier. Public app views may expose the generation and schema version, but must not disclose internal absolute paths through tools or pages.

Existing installs retain their original `data/<id>` directory. A missing legacy pointer may be initialized to that exact host-owned directory without moving or rewriting its contents. New upgrade generations live in a separate `data-versions/<id>/<generation>` namespace. A destination nested inside the old directory would make recursive copy unsafe and is forbidden. Restart and re-enable use the persisted pointer, never a newly reconstructed `data/<id>` path.

A retained release record must identify the previous app version, manifest/digest, package path, data generation/path, a replacement timestamp and the replacing version. Keeping bytes without a discoverable durable record is insufficient for reliable recovery. The host stores these references in `application_versions`. Old package/data generations remain available for an explicit future rollback operation; they are never granted to the new worker. This does not imply that an older app may read a newer schema. A rollback must restore the corresponding old data generation and clear grants again, with the same stop/exit requirements.

## Upgrade sequence

1. Reject enabled apps, stale revisions, another mutation, a pending worker start, or active accepted calls. Hold the same per-app mutation guard used by enable, grants, calls, and uninstall. The app remains disabled throughout preparation.
2. Validate and start the old installed worker if it is not already resident, using its current data pointer, so app-owned persistent busy state is consulted. Ask the old worker to stop. `{busy:true}` refuses upgrade without killing a running task. Wait for actual OS process exit. A killed-but-not-yet-exited worker is not quiescent. Remove only the stopped worker's cache entry so a later re-enable can create a fresh process. A failed worker may be replaced only after its actual exit is confirmed.
3. Copy and revalidate the candidate package. Never launch from the caller's source path. Copy the stopped app's entire regular-file data tree, including SQLite WAL files if still present, into a fresh host-owned candidate generation. Reject symlinks, nonregular files, ambiguous roots and path escapes. Copy at most 4096 filesystem entries, 64 MiB per file and 256 MiB in total; a limit failure leaves the active pointers unchanged. The host must not silently skip files.
4. Start a candidate worker with the candidate package and candidate data directory. `lifecycle/start` must complete with exactly the candidate manifest's `dataSchemaVersion`. No project grants or tool capability are issued. The app migrates only this copy. A timeout, malformed protocol, startup failure, or process crash aborts preparation.
5. Stop the migrated candidate and confirm actual process exit. A candidate reporting busy is not eligible to commit. Neither the old worker nor the migration worker may remain alive when the migrated directory becomes current.
6. In one host database transaction, conditionally replace the current package identity and data pointer, increment app revision, preserve `enabled=false`, record retained version metadata, and clear every project grant while advancing existing grant revisions. Commit only if the original row identity/revision still matches. Filesystem preparation precedes this transaction, so there is no window in which the registry points to a half-copied directory.
7. Return the disabled new version. A later explicit enable creates a fresh worker from its installed package and current data pointer. Enabling does not restore any old project permission; each project requires a fresh grant.

The filesystem cannot participate in the SQLite transaction. Atomicity therefore means that a transaction switches only between fully prepared immutable package/data references. On failure before commit, the original registry, retained-version records and grant rows are unchanged. Candidate writes cannot alter original app data. Normal old-owner startup may perform its own app migration or write lifecycle metadata, and shutdown may checkpoint its own SQLite files. Those legitimate owner effects are not rolled back. The host never runs candidate migration against the old files. The guarantee is candidate write isolation, not arbitrary byte-for-byte rollback of all old-owner lifecycle effects. A candidate whose stop is busy or unconfirmed stays in a separate supervised process map and blocks normal enable/uninstall until its exit can be confirmed. Failed candidate artifacts may be retained for diagnosis or removed only within the host-owned staging namespace; they are never installed implicitly.

## Uninstall and exact-package reinstall

Uninstall preserves the current generation, including data produced after upgrade. In the same transaction that removes the installed app row, `application_retained` records `app_id`, version, package digest, `data_path`, `data_version`, data schema version and retention time. A new install can restore that pointer only when app ID, version, digest and schema match exactly. Successful restoration consumes the retained marker and leaves the app disabled with no project grants.

An incompatible reinstall returns `host_app_retained_version_conflict` and leaves the retained pointer and all bytes intact. It must not fall back to the old `data/<id>` directory, start an older package on newer data, or silently create an empty inventory. Choosing another package or a historical generation requires an explicit future recovery operation. This is separate from normal upgrade and does not grant execution authority.

## Crash and recovery rules

Before commit, a host restart observes the old pointers and disabled state. Incomplete candidates are not resumed or automatically adopted. A durable preparation record should mark their owner and phase; an orphan candidate process can write only its candidate data, which must not become current. Recovery must not delete or adopt a generation while its writer may still be alive.

After commit, a restart observes the new package and matching data generation together, disabled with no prior grants. Re-enabling starts the new package on that generation. There is no automatic fallback to old code on new data and no automatic reactivation of prior authority. A failed normal startup after commit leaves the new version disabled/failed for an explicit operator decision; it must not silently roll back and restore grants.

An interrupted upgrade can leave additional disk usage. Retained versions, candidate generations and failed-upgrade diagnostics need an explicit bounded retention policy before large production workloads. This first contract does not claim automatic garbage collection, full power-loss durability across filesystems, a hostile-code sandbox, or a complete rollback UI.

## Domain migration limits

A generic data copy does not transfer execution authority. The old native scanning engine's execution birth receipts are bound to the current executable's SHA and canonical database path. Changing either by copying app data or upgrading code cannot legitimately resume, re-sign, or adopt those tasks. Paid, pending, paused, or unfinished legacy work must continue through the original desktop/engine and original database path. The current independent Scanning app stores drafts and journals only; it does not provide a live execution migration.

An app owner must reject a migration it cannot prove safe. Both the old worker's stop hook and the candidate's migration/start or stop hook may refuse, including when a paused task has no currently active tool call. The current `busy` veto is a necessary refusal mechanism, not a general proof that every domain has no retained execution obligations. A future protocol needs an explicit owner-issued `canUpgrade`/migration receipt for live execution domains; this implementation must not invent it or weaken existing engine receipts. No automatic resume, scan, target request, model call, or reauthorization is part of upgrade.

## Behavior verification

The upgrade suite constructs local v1/v2 packages, writes only synthetic app-owned JSON, and launches actual Node workers through `AppProcess`. It checks successful copy migration and restart; old artifact/data retention; migration exceptions after candidate writes; actual candidate process crashes; stale identity/schema/version rejection; busy and unconfirmed exit refusal; disabled state and grant revocation; rejection of symlinked app data; and exact-package reinstall of the latest upgraded generation without restoring permissions, with incompatible reinstall refusal. No provider, model, scan target, legacy desktop database, or user data is involved.

```sh
node --test --test-reporter=tap --test-concurrency=1 packages/agent-host/test/upgrades.test.mjs
```

The source package and data generation paths are observed inside the synthetic app solely to prove which copy ran; real APIs should not return these internal paths to users or runtimes.
