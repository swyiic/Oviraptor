import { hostError, positiveId } from './store.mjs';

/** Operator-owned policy, shared by Configuration and all chats in one project.
 * Full access is a snapshot of enabled permission categories, never a new target
 * scope, an application enable switch, or a bypass of effect/budget validation. */
export class ProjectAccessPolicy {
  constructor(store, identity) {
    this.store = store; this.identity = identity;
    store.db.exec(`CREATE TABLE IF NOT EXISTS project_access_policies(
      project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
      mode TEXT NOT NULL CHECK(mode IN ('ask','full')), runtime_identity TEXT NOT NULL,
      revision INTEGER NOT NULL, apps_json TEXT NOT NULL)`);
  }
  view(projectId) {
    const id = positiveId(projectId); this.store.project(id);
    const row = this.store.db.prepare('SELECT * FROM project_access_policies WHERE project_id=?').get(id);
    const current = this.identity(), active = row?.mode === 'full' && row.runtime_identity === current;
    return { projectId:id, mode:active?'full':'ask', savedMode:row?.mode??'ask', revision:row?.revision??0,
      runtimeIdentity:current, needsReview:row?.mode==='full'&&!active,
      apps:row?JSON.parse(row.apps_json):{}, workspacePermissions:active?['workspace.read','workspace.write']:[] };
  }
  set(input = {}) {
    if (!['ask','full'].includes(input.mode)) throw hostError('host_access_policy_invalid');
    const before = this.view(input.projectId);
    if (input.expectedRuntimeIdentity !== before.runtimeIdentity) throw hostError('host_runtime_identity_changed',409);
    if (input.expectedRevision !== before.revision) throw hostError('host_access_policy_conflict',409);
    if (input.mode === 'full' && input.operatorConfirmed !== true) throw hostError('host_access_policy_confirmation_required',403);
    const apps = input.mode==='full'?Object.fromEntries(this.store.applications().filter(app=>app.enabled).map(app=>[
      app.id,[...new Set(app.manifest.permissions.map(p=>p.id))].sort(),
    ])):{};
    this.store.db.prepare(`INSERT INTO project_access_policies VALUES(?,?,?,?,?)
      ON CONFLICT(project_id) DO UPDATE SET mode=excluded.mode,runtime_identity=excluded.runtime_identity,revision=excluded.revision,apps_json=excluded.apps_json`)
      .run(before.projectId,input.mode,before.runtimeIdentity,before.revision+1,JSON.stringify(apps));
    return this.view(before.projectId);
  }
  grant(app, projectId, base, runtimeIdentity) {
    if (runtimeIdentity === undefined) return base;
    const policy = this.view(projectId);
    const allowed = policy.mode==='full' && runtimeIdentity===policy.runtimeIdentity ? policy.apps[app.id] : null;
    return { ...base, permissions:allowed?[...new Set([...(base.disclosureRuntime===runtimeIdentity?base.permissions:[]),...allowed])]:base.permissions,
      disclosureRuntime:allowed?runtimeIdentity:base.disclosureRuntime,
      accessPolicyRevision:policy.revision, accessRuntimeIdentity:runtimeIdentity };
  }
}
