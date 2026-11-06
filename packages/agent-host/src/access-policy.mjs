import { hostError, positiveId } from './store.mjs';
import { createHash } from 'node:crypto';

const DISCOVERY_EXECUTION = 'inventory.discovery.run';
const ASSETS_APP = 'org.oviraptor.assets';
const BUILTIN_PROVENANCE = 'builtin_assets_discovery_access_v1';
const digest = manifest => createHash('sha256').update(JSON.stringify(manifest)).digest('hex');

/** Operator-owned policy, shared by Configuration and all chats in one project.
 * Full access is a snapshot of enabled permission categories, never a new target
 * scope, an application enable switch, or a bypass of effect/budget validation. */
export class ProjectAccessPolicy {
  constructor(store, identity) {
    this.store = store; this.identity = identity;
    this.builtinDiscoveryProofs = new Map();
    store.db.exec(`CREATE TABLE IF NOT EXISTS project_access_policies(
      project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
      mode TEXT NOT NULL CHECK(mode IN ('ask','auto','full')), runtime_identity TEXT NOT NULL,
      revision INTEGER NOT NULL, apps_json TEXT NOT NULL)`);
    // Migrate an older table whose CHECK predates the 'auto' tier (same column
    // set, so a straight copy is exact). Without this, saving 'auto' on an
    // existing install would violate the old constraint.
    if (!this.store.db.prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='project_access_policies'").get().sql.includes("'auto'")) {
      this.store.db.exec(`BEGIN IMMEDIATE;
        ALTER TABLE project_access_policies RENAME TO project_access_policies_before_auto;
        CREATE TABLE project_access_policies(
          project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
          mode TEXT NOT NULL CHECK(mode IN ('ask','auto','full')), runtime_identity TEXT NOT NULL,
          revision INTEGER NOT NULL, apps_json TEXT NOT NULL);
        INSERT INTO project_access_policies SELECT * FROM project_access_policies_before_auto;
        DROP TABLE project_access_policies_before_auto; COMMIT;`);
    }
  }
  view(projectId) {
    const id = positiveId(projectId); this.store.project(id);
    const row = this.store.db.prepare('SELECT * FROM project_access_policies WHERE project_id=?').get(id);
    const current = this.identity();
    // Both elevated tiers require the saved runtime identity to still match; on a
    // mismatch they degrade to 'ask' and need re-confirmation. 'auto' grants only
    // the non-destructive workspace permissions (read + isolated run) and never
    // auto-grants any app; 'full' grants read/write/run plus the stored app set.
    const saved = row?.mode ?? 'ask';
    const elevated = saved === 'full' || saved === 'auto';
    const active = elevated && row.runtime_identity === current;
    const mode = active ? saved : 'ask';
    const workspacePermissions = !active ? []
      : mode === 'full' ? ['workspace.read','workspace.write','workspace.run']
      : ['workspace.read','workspace.run'];
    return { projectId:id, mode, savedMode:saved, revision:row?.revision??0,
      runtimeIdentity:current, needsReview:elevated&&!active,
      apps:row?JSON.parse(row.apps_json):{}, workspacePermissions };
  }
  set(input = {}) {
    if (!['ask','auto','full'].includes(input.mode)) throw hostError('host_access_policy_invalid');
    const before = this.view(input.projectId);
    if (input.expectedRuntimeIdentity !== before.runtimeIdentity) throw hostError('host_runtime_identity_changed',409);
    if (input.expectedRevision !== before.revision) throw hostError('host_access_policy_conflict',409);
    // Either elevation is a deliberate widening of default-deny and needs the
    // operator's explicit confirmation, even though 'auto' only opens
    // non-destructive workspace ops.
    if ((input.mode === 'full' || input.mode === 'auto') && input.operatorConfirmed !== true) throw hostError('host_access_policy_confirmation_required',403);
    // Only 'full' auto-grants enabled apps' permissions; 'auto' and 'ask' grant
    // no apps, so an app action under 'auto' still prompts.
    const apps = input.mode==='full'?Object.fromEntries(this.store.applications().filter(app=>app.enabled).map(app=>[
      app.id,[...new Set(app.manifest.permissions.map(p=>p.id))].sort(),
    ])):{};
    this.store.db.prepare(`INSERT INTO project_access_policies VALUES(?,?,?,?,?)
      ON CONFLICT(project_id) DO UPDATE SET mode=excluded.mode,runtime_identity=excluded.runtime_identity,revision=excluded.revision,apps_json=excluded.apps_json`)
      .run(before.projectId,input.mode,before.runtimeIdentity,before.revision+1,JSON.stringify(apps));
    return this.view(before.projectId);
  }
  // Only the trusted bundled-package loader calls this after validating its
  // source files. An app ID or an editable builtin flag is insufficient proof.
  recordBundledApplication(app, validatedManifest) {
    if (app.id !== ASSETS_APP || validatedManifest?.id !== ASSETS_APP || !app.enabled
      || !app.builtin || app.uninstallable !== false || app.version !== validatedManifest.version
      || app.digest !== digest(validatedManifest) || app.digest !== digest(app.manifest)) return;
    const value = { schemaVersion: 1, source: 'validated-bundled-package', appId: app.id,
      version: app.version, digest: app.digest, path: app.path };
    this.store.db.prepare('INSERT OR REPLACE INTO host_settings(key,value) VALUES(?,?)').run(BUILTIN_PROVENANCE, JSON.stringify(value));
    this.builtinDiscoveryProofs.clear();
  }
  compatibleBuiltinDiscovery(app, allowed) {
    if (app.id !== ASSETS_APP || !app.enabled || !app.builtin || app.uninstallable !== false
      || !Array.isArray(allowed) || !allowed.includes('inventory.write') || !allowed.includes('certificate.collect')
      || allowed.includes(DISCOVERY_EXECUTION) || !app.manifest.permissions.some(p => p.id === DISCOVERY_EXECUTION)) return false;
    const key = JSON.stringify([app.id, app.version, app.digest, app.path, app.revision]);
    if (this.builtinDiscoveryProofs.has(key)) return this.builtinDiscoveryProofs.get(key);
    const row = this.store.db.prepare('SELECT value FROM host_settings WHERE key=?').get(BUILTIN_PROVENANCE);
    let valid = false;
    try {
      const proof = JSON.parse(row?.value ?? 'null');
      valid = proof?.schemaVersion === 1 && proof.source === 'validated-bundled-package' && proof.appId === app.id
        && proof.version === app.version && proof.path === app.path && proof.digest === app.digest
        && app.digest === digest(app.manifest);
    } catch { /* Missing or malformed provenance never extends access. */ }
    this.builtinDiscoveryProofs.clear();
    this.builtinDiscoveryProofs.set(key, valid);
    return valid;
  }
  grant(app, projectId, base, runtimeIdentity) {
    if (runtimeIdentity === undefined) return base;
    const policy = this.view(projectId);
    const allowed = policy.mode==='full' && runtimeIdentity===policy.runtimeIdentity ? policy.apps[app.id] : null;
    const compatible = this.compatibleBuiltinDiscovery(app, allowed);
    return { ...base, permissions:allowed?[...new Set([...(base.disclosureRuntime===runtimeIdentity?base.permissions:[]),...allowed,...(compatible?[DISCOVERY_EXECUTION]:[])])]:base.permissions,
      disclosureRuntime:allowed?runtimeIdentity:base.disclosureRuntime,
      ...(compatible ? { permissionCompatibility: { source: 'compatibility', rule: BUILTIN_PROVENANCE, permissions: [DISCOVERY_EXECUTION] } } : {}),
      accessPolicyRevision:policy.revision, accessRuntimeIdentity:runtimeIdentity };
  }
}
