import { DatabaseSync } from 'node:sqlite';
import { existsSync } from 'node:fs';
import { assertAssetInventorySchema } from './asset-inventory-schema.mjs';

// Context only: no grants, approval mutation or raw certificate/page disclosure.
// Read in the same source transaction as inventory; never turn a stale mirror
// into authority to execute a network request.
function discoveryContext(desktop, projectId) {
  const has = table => Boolean(desktop.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?").get(table));
  let confirmedScope = { available: false, reason: 'scope_not_saved' };
  if (has('asset_discovery_scopes')) {
    const row = desktop.prepare('SELECT roots_json FROM asset_discovery_scopes WHERE project_id=?').get(projectId);
    if (row) {
      try {
        const values = JSON.parse(row.roots_json);
        if (!Array.isArray(values) || values.some(v => typeof v !== 'string' || !v || v.length > 253) || values.length > 500) throw Error('invalid_scope');
        const roots = []; let bytes = 0;
        for (const root of [...new Set(values)]) { bytes += Buffer.byteLength(root) + 3; if (bytes > 16_000) break; roots.push(root); }
        confirmedScope = { available: true, roots, totalRoots: new Set(values).size, truncated: roots.length !== new Set(values).size,
          source: 'desktop_user_confirmed_domain_scope', ownershipConfirmed: false, executionPermissionGranted: false };
      } catch { confirmedScope = { available: false, reason: 'saved_scope_invalid' }; }
    }
  }
  let seedReview = { available: false, reason: 'review_not_prepared' };
  if (has('asset_seed_reviews') && has('asset_seed_review_items')) {
    const row = desktop.prepare('SELECT id,organization,status,total,processed,requests,input_tokens,output_tokens,unknown_requests,updated_at FROM asset_seed_reviews WHERE project_id=? ORDER BY rowid DESC LIMIT 1').get(projectId);
    if (row) {
      const counts = {}, queueCounts = {}, manualCounts = {};
      const columns = desktop.prepare('PRAGMA table_info(asset_seed_review_items)').all().map(c => c.name);
      const complete = ['kind','evidence_json','decision'].every(c => columns.includes(c));
      const advice = complete ? "CASE WHEN advice='review' THEN 'evidence' WHEN advice IN ('retain','conflict') AND COALESCE(json_array_length(evidence_json,'$.subjectOrganizations'),0)=0 AND COALESCE(json_array_length(evidence_json,'$.icpOrganizations'),0)=0 AND COALESCE(json_array_length(evidence_json,'$.organizationHints'),0)=0 AND COALESCE(json_array_length(evidence_json,'$.correlatedOrganizations'),0)=0 AND COALESCE(json_array_length(evidence_json,'$.confirmedDomainMatches'),0)=0 THEN 'evidence' ELSE advice END": "CASE WHEN advice='review' THEN 'evidence' ELSE advice END";
      for (const value of desktop.prepare(`SELECT state,${advice} advice,${complete ? 'decision' : "'pending'"} decision,COUNT(*) n FROM asset_seed_review_items WHERE run_id=? GROUP BY state,2,3`).all(row.id)) {
        const key = value.state === 'done' ? value.advice : value.state;
        counts[key] = (counts[key] ?? 0) + value.n;
        const map = value.decision === 'pending' ? queueCounts : manualCounts;
        const queueKey = value.decision === 'pending' ? (['done','deferred'].includes(value.state) ? value.advice : value.state) : value.decision;
        map[queueKey] = (map[queueKey] ?? 0) + value.n;
      }
      seedReview = { available: true, id: row.id, targetOrganization: row.organization, savedStatus: row.status,
        totalAssets: row.total, processedAssets: row.processed, groups: Object.values(counts).reduce((a,b) => a+b,0), counts, queueCounts, manualCounts,
        modelRequests: row.requests, inputTokens: row.input_tokens, outputTokens: row.output_tokens, unknownRequests: row.unknown_requests,
        updatedAt: row.updated_at, storedStatusOnly: true, adviceIsNotScopeApproval: true, executionPermissionGranted: false };
    }
  }
  return { confirmedScope, seedReview, contextSnapshotAt: new Date().toISOString() };
}

function certificateCoverage(desktop, projectId) {
  if (!desktop.prepare('PRAGMA table_info(assets)').all().some(column => column.name === 'extra_json')) {
    return { available: false, reason: 'certificate_columns_missing' };
  }
  // Summarize in the source read transaction; never copy certificate bytes to the model mirror.
  const value = desktop.prepare(`
    WITH scoped AS (
      SELECT lower(COALESCE(a.link,'')) LIKE 'https://%' AS https,
        CASE WHEN json_valid(a.extra_json) THEN a.extra_json ELSE '{}' END AS e
      FROM assets a JOIN project_assets pa ON pa.asset_id=a.id
      WHERE pa.project_id=? AND COALESCE(pa.is_deleted,0)=0
    ), facts AS (
      SELECT https,
        (COALESCE(json_extract(e,'$."cert.domain"'),'')<>'' OR COALESCE(json_extract(e,'$."cert.subject.cn"'),'')<>''
          OR COALESCE(json_extract(e,'$."cert.sn"'),'')<>'') AS metadata,
        COALESCE(json_extract(e,'$.probe_cert_sha256'),json_extract(e,'$.cert_sha256'),'') AS fingerprint,
        (COALESCE(json_extract(e,'$.probe_cert_der_base64'),'')<>'' OR COALESCE(json_extract(e,'$.certificate_der_base64'),'')<>''
          OR COALESCE(json_extract(e,'$.certificate_pem'),'')<>'' OR COALESCE(json_extract(e,'$.ssl_certificate'),'') LIKE '%-----BEGIN CERTIFICATE-----%') AS raw
      FROM scoped
    ), classified AS (
      SELECT *, (length(fingerprint)=64 AND lower(fingerprint) NOT GLOB '*[^0-9a-f]*') AS has_fingerprint FROM facts
    )
    SELECT count(*) AS assets,
      COALESCE(sum(https),0) AS httpsAssets,
      COALESCE(sum(metadata),0) AS assetsWithCertificateMetadata,
      COALESCE(sum(has_fingerprint),0) AS assetsWithSavedFingerprint,
      COALESCE(sum(raw),0) AS assetsWithRawCertificateCandidate,
      COALESCE(sum(https AND NOT raw AND NOT has_fingerprint),0) AS httpsAssetsMissingSavedCertificate
    FROM classified
  `).get(projectId);
  return { available: true, ...value, unit: 'asset_records_not_unique_certificates',
    localExtractionRequestsNetwork: false,
    note: 'Metadata names/serials cannot produce a certificate fingerprint. Raw candidates are presence checks, not parsed certificate validation. Missing certificate bytes require an authorized TLS fetch; offline extraction does not fetch them. Fingerprint totals and coverage do not establish ownership or TLS trust.' };
}

/** Resolve an explicit namespace in the Assets-owned mirror. Numeric equality
 * between independent stores is never an identity relation. The desktop owner
 * still rechecks live membership before accepting the translated targets. */
export function resolveDesktopAssetIds({inventoryDbPath,hostProjectId,assetIds,assetIdNamespace}) {
  const fail=code=>{throw Object.assign(Error(code),{code,retryable:false});};
  if(!['inventory','desktop'].includes(assetIdNamespace)||!Number.isSafeInteger(hostProjectId)||hostProjectId<1||!Array.isArray(assetIds)||!assetIds.length||assetIds.length>128||assetIds.some(id=>!Number.isSafeInteger(id)||id<1)||new Set(assetIds).size!==assetIds.length)fail('asset_handoff_identity_invalid');
  if(!inventoryDbPath||!existsSync(inventoryDbPath))fail('asset_handoff_inventory_unavailable');
  const db=new DatabaseSync(inventoryDbPath,{readOnly:true});
  try{
    assertAssetInventorySchema(db);db.exec('BEGIN');
    const query=db.prepare(`SELECT id,external_id FROM inventory WHERE project_id=? AND source='desktop-assets' AND ${assetIdNamespace==='inventory'?'id':'external_id'}=?`);
    const bindings=assetIds.map(id=>{
      const row=query.get(hostProjectId,assetIdNamespace==='inventory'?id:String(id));
      if(!row||!/^[1-9][0-9]*$/.test(row.external_id)||String(Number(row.external_id))!==row.external_id||!Number.isSafeInteger(Number(row.external_id)))fail('asset_handoff_identity_unavailable');
      return {inputAssetId:id,inventoryRecordId:row.id,desktopAssetId:Number(row.external_id)};
    });
    db.exec('COMMIT');return {inputNamespace:assetIdNamespace,bindings,desktopAssetIds:bindings.map(row=>row.desktopAssetId),source:'assets_owned_desktop_mirror'};
  }finally{try{db.exec('ROLLBACK')}catch{}db.close();}
}

/** Assets-owned snapshot copy. The host calls this; it does not own the inventory SQL. */
export function syncDesktopAssetSnapshot({ desktopDbPath, inventoryDbPath, hostProjectId, desktopProjectId, inventoryConnection }) {
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
    inventory = inventoryConnection ?? new DatabaseSync(inventoryDbPath);
    inventory.exec('PRAGMA busy_timeout=2000');
    assertAssetInventorySchema(inventory);
    desktop.exec('BEGIN');
    const rows = desktop.prepare(`
      SELECT a.id, a.host, a.link, a.ip, a.port, a.protocol, a.domain, a.title, a.status_code, a.probe_outcome
      FROM assets a
      JOIN project_assets pa ON pa.asset_id = a.id
      WHERE pa.project_id = ? AND COALESCE(pa.is_deleted, 0) = 0
    `).all(desktopId);
    const hasSeeds = Boolean(desktop.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_discovery_seeds'").get());
    const coverage = certificateCoverage(desktop, desktopId);
    const discovery = hasSeeds ? {
      available: true, source: 'local_discovery_seed_index',
      total: desktop.prepare('SELECT COUNT(*) n FROM asset_discovery_seeds WHERE project_id=?').get(desktopId).n,
      bySource: desktop.prepare('SELECT source,COUNT(*) count,COUNT(DISTINCT host) distinctTargets FROM asset_discovery_seeds WHERE project_id=? GROUP BY source ORDER BY source').all(desktopId),
      certificateFingerprints: desktop.prepare("SELECT COUNT(DISTINCT host) n FROM asset_discovery_seeds WHERE project_id=? AND source='certificate_sha256'").get(desktopId).n,
      certificateNames: desktop.prepare("SELECT COUNT(DISTINCT host) n FROM asset_discovery_seeds WHERE project_id=? AND source IN ('certificate_domain','certificate_cn','certificate_transparency')").get(desktopId).n,
      certificateCoverage: coverage,
      note: 'Counts of saved local seeds only; zero does not establish absence of certificates or complete extraction.',
    } : { available: false, source: 'local_discovery_seed_index', reason: 'seed_index_missing', certificateCoverage: coverage };
    Object.assign(discovery, discoveryContext(desktop, desktopId));
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
    inventory.exec('CREATE TEMP TABLE IF NOT EXISTS desktop_mirror_seen(external_id TEXT PRIMARY KEY); DELETE FROM desktop_mirror_seen');
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
    if (!inventoryConnection) try { inventory?.close(); } catch { /* closed */ }
  }
}
