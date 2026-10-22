// SQLite snapshot work belongs off the Host event loop. A persistent read
// connection makes data_version meaningful across calls (reopening it does not).
import { parentPort } from 'node:worker_threads';
import { DatabaseSync } from 'node:sqlite';
import { statSync } from 'node:fs';
import { syncDesktopAssetSnapshot } from '../../app-sdk/src/desktop-asset-sync.mjs';

const watches = new Map(), snapshots = new Map();
function version(path, readOnly = true) {
  const stat = statSync(path, { bigint: true }), identity = `${stat.dev}:${stat.ino}`;
  let watch = watches.get(path);
  if (watch?.identity !== identity) {
    watch?.db.close();
    watch = { identity, db: new DatabaseSync(path, { readOnly }) };
    watch.db.exec('PRAGMA busy_timeout=2000');
    watches.set(path, watch);
    // Replacement invalidates cached versions even if SQLite reused the value.
    for (const [key, value] of snapshots) if (value.paths.includes(path)) snapshots.delete(key);
  }
  return `${identity}:${watch.db.prepare('PRAGMA data_version').get().data_version}`;
}
parentPort.on('message', ({ id, input }) => {
  try {
    const paths = [input.desktopDbPath, input.inventoryDbPath];
    const key = JSON.stringify([paths, input.hostProjectId, input.desktopProjectId]);
    // One namespace can have only one current desktop mapping. Our own writes
    // intentionally do not advance the persistent writer's data_version.
    const namespace = JSON.stringify([input.inventoryDbPath, input.hostProjectId]);
    for (const [other, value] of snapshots) if (other !== key && value.namespace === namespace) snapshots.delete(other);
    const versions = () => [version(paths[0]), version(paths[1], false)].join('|');
    const before = versions(), cached = snapshots.get(key);
    if (cached?.version === before) {
      parentPort.postMessage({ id, result: { ...cached.result, updated: 0, removed: 0, unchanged: true, cached: true } });
      return;
    }
    const result = syncDesktopAssetSnapshot({ ...input, inventoryConnection: watches.get(input.inventoryDbPath).db });
    const after = versions();
    // The SAME connection performs the mirror write. Its data_version changes
    // only for other writers, so a concurrent external commit cannot be mistaken
    // for our own completed copy and cached as if we had incorporated it.
    if (result.complete && before === after) {
      snapshots.set(key, { paths, namespace, version: after, result });
    } else snapshots.delete(key);
    parentPort.postMessage({ id, result: { ...result, cached: false } });
  } catch {
    parentPort.postMessage({ id, result: { synced: 0, reason: 'desktop_sync_failed' } });
  }
});
