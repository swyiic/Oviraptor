import { DatabaseSync } from 'node:sqlite';
import { existsSync } from 'node:fs';
import { assertAssetInventorySchema } from './asset-inventory-schema.mjs';

/** Assets-owned snapshot copy. The host calls this; it does not own the inventory SQL. */
export function syncDesktopAssetSnapshot({ desktopDbPath, inventoryDbPath, hostProjectId, desktopProjectId }) {
  if (!desktopDbPath || !inventoryDbPath || !existsSync(desktopDbPath)) return { synced: 0, reason: 'desktop_db_missing' };
  const desktopId = Number(desktopProjectId);
  const hostId = Number(hostProjectId);
  if (!Number.isSafeInteger(desktopId) || desktopId <= 0 || !Number.isSafeInteger(hostId) || hostId <= 0) {
    return { synced: 0, reason: 'project_unmapped' };
  }
  let desktop;
  let inventory;
  try {
    desktop = new DatabaseSync(desktopDbPath, { readOnly: true });
    if (!existsSync(inventoryDbPath)) return { synced: 0, reason: 'desktop_inventory_upgrade_required' };
    inventory = new DatabaseSync(inventoryDbPath);
    assertAssetInventorySchema(inventory);
    desktop.exec('BEGIN');
    const rows = desktop.prepare(`
      SELECT a.id, a.host, a.link, a.ip, a.port, a.protocol, a.domain, a.title, a.status_code, a.probe_outcome
      FROM assets a
      JOIN project_assets pa ON pa.asset_id = a.id
      WHERE pa.project_id = ? AND COALESCE(pa.is_deleted, 0) = 0
    `).all(desktopId);
    const hasSeeds = Boolean(desktop.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_discovery_seeds'").get());
    const discovery = hasSeeds ? {
      available: true, source: 'local_discovery_seed_index',
      total: desktop.prepare('SELECT COUNT(*) n FROM asset_discovery_seeds WHERE project_id=?').get(desktopId).n,
      bySource: desktop.prepare('SELECT source,COUNT(*) count,COUNT(DISTINCT host) distinctTargets FROM asset_discovery_seeds WHERE project_id=? GROUP BY source ORDER BY source').all(desktopId),
      certificateFingerprints: desktop.prepare("SELECT COUNT(DISTINCT host) n FROM asset_discovery_seeds WHERE project_id=? AND source='certificate_sha256'").get(desktopId).n,
      certificateNames: desktop.prepare("SELECT COUNT(DISTINCT host) n FROM asset_discovery_seeds WHERE project_id=? AND source IN ('certificate_domain','certificate_cn','certificate_transparency')").get(desktopId).n,
      note: 'Counts of saved local seeds only; zero does not establish absence of certificates or complete extraction.',
    } : { available: false, source: 'local_discovery_seed_index', reason: 'seed_index_missing' };
    desktop.exec('COMMIT');
    const write = inventory.prepare(`
      INSERT INTO inventory(project_id, source, external_id, observation_json, received_at)
      VALUES(?, 'desktop-assets', ?, ?, ?)
      ON CONFLICT(project_id, source, external_id) DO UPDATE SET
        observation_json = excluded.observation_json,
        received_at = excluded.received_at
      WHERE json_remove(inventory.observation_json, '$.observedAt')
        IS NOT json_remove(excluded.observation_json, '$.observedAt')
    `);
    const now = new Date().toISOString();
    inventory.exec('BEGIN');
    inventory.exec('CREATE TABLE IF NOT EXISTS inventory_discovery_summary(project_id INTEGER PRIMARY KEY,summary_json TEXT NOT NULL)');
    inventory.prepare('INSERT INTO inventory_discovery_summary VALUES(?,?) ON CONFLICT(project_id) DO UPDATE SET summary_json=excluded.summary_json').run(hostId, JSON.stringify(discovery));
    inventory.exec('CREATE TEMP TABLE desktop_mirror_seen(external_id TEXT PRIMARY KEY)');
    const seen = inventory.prepare('INSERT OR IGNORE INTO desktop_mirror_seen VALUES(?)');
    let updated = 0;
    for (const row of rows) {
      const observation = {
        externalId: String(row.id), host: row.host || '', link: row.link || '', ip: row.ip || '',
        port: row.port == null ? '' : String(row.port), protocol: row.protocol || '', domain: row.domain || '',
        title: row.title || '', statusCode: row.status_code == null ? '' : String(row.status_code),
        probeOutcome: row.probe_outcome || '', observedAt: now,
      };
      seen.run(String(row.id));
      updated += Number(write.run(hostId, String(row.id), JSON.stringify(observation), now).changes);
    }
    const removed = Number(inventory.prepare("DELETE FROM inventory WHERE project_id=? AND source='desktop-assets' AND external_id NOT IN (SELECT external_id FROM desktop_mirror_seen)").run(hostId).changes);
    inventory.exec('COMMIT');
    return {
      schemaVersion: 1, source: 'desktop-assets', owner: 'assets',
      sourceProjectBinding: { hostProjectId: hostId, desktopProjectId: desktopId },
      sourceSnapshotRevision: rows.map(row => String(row.id)).sort().join(','),
      complete: true, synced: rows.length, updated, removed, unchanged: updated === 0 && removed === 0,
    };
  } catch (error) {
    try { inventory?.exec('ROLLBACK'); } catch { /* not open */ }
    if (/^asset_inventory_(schema|identity)_/u.test(error?.code ?? '')) return { synced: 0, reason: 'desktop_inventory_upgrade_required' };
    return { synced: 0, reason: 'desktop_sync_failed', detail: String(error?.message || error).slice(0, 180) };
  } finally {
    try { desktop?.close(); } catch { /* closed */ }
    try { inventory?.close(); } catch { /* closed */ }
  }
}
