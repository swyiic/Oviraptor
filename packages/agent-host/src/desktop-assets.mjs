import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { syncDesktopAssetSnapshot } from '../../app-sdk/src/desktop-asset-sync.mjs';
export { DesktopAssetSynchronizer } from './desktop-asset-synchronizer.mjs';

const READ_TOOLS = new Set(['inventory_stats', 'inventory_search', 'inventory_query', 'project_overview']);

export function desktopAssetDbPath(hostDataDir) {
  if (!hostDataDir) return null;
  const candidate = join(dirname(hostDataDir), 'oviraptor.sqlite3');
  return existsSync(candidate) ? candidate : null;
}

export function isDesktopAssetRead(toolName) {
  const short = String(toolName ?? '').split('::').pop()?.split('__').pop();
  return READ_TOOLS.has(short);
}

/** Host adapter. Inventory SQL lives in the Assets SDK. */
export function syncDesktopAssets(input) {
  return syncDesktopAssetSnapshot(input);
}
