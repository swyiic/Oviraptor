import { mkdir, readFile, writeFile, copyFile, rename, rm, realpath, lstat } from 'node:fs/promises';
import { join, dirname, isAbsolute } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { validatePackageDirectory, validateToolArguments, qualifiedToolName } from '../../app-sdk/src/index.mjs';
import { AppProcess } from './app-process.mjs';
import { hostError, positiveId } from './store.mjs';
import { upgradeApplication } from './upgrades.mjs';

export class AppManager {
  constructor(store, root, { processFactory = (app, data) => new AppProcess(app, data) } = {}) {
    this.store = store; this.root = root; this.processFactory = processFactory; this.workers = new Map(); this.startingWorkers = new Map(); this.changing = new Set();
  }
  async install(source) {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    if (typeof source !== 'string') throw hostError('host_package_path_invalid');
    const checked = await validatePackageDirectory(source);
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    const manifest = checked.manifest;
    if (this.store.applications().some(app => app.id === manifest.id)) throw hostError('host_app_already_installed', 409);
    if (this.changing.has(manifest.id)) throw hostError('host_app_changing', 409);
    this.changing.add(manifest.id);
    const digest = createHash('sha256').update(JSON.stringify(manifest)).digest('hex');
    const retained = this.store.db.prepare('SELECT * FROM application_retained WHERE app_id=?').get(manifest.id);
    if (retained && (retained.version !== manifest.version || retained.digest !== digest || retained.data_schema_version !== manifest.dataSchemaVersion)) {
      this.changing.delete(manifest.id);
      throw hostError('host_app_retained_version_conflict', 409);
    }
    const packageRoot = join(this.root, 'packages');
    const staging = join(packageRoot, `.staging-${randomUUID()}`);
    const target = join(packageRoot, `${manifest.id}-${manifest.version}-${digest}`);
    try {
      await mkdir(staging, { recursive: true, mode: 0o700 });
      // Copy only validated entries, then validate the copied bytes again.
      for (const file of manifest.files) {
        const destination = join(staging, file.path);
        await mkdir(dirname(destination), { recursive: true, mode: 0o700 });
        await copyFile(join(source, file.path), destination);
      }
      await writeFile(join(staging, 'manifest.json'), JSON.stringify(manifest), { mode: 0o600 });
      await validatePackageDirectory(staging);
      try { await rename(staging, target); }
      catch (error) {
        if (!['ENOTEMPTY', 'EEXIST'].includes(error.code)) throw error;
        const retained = await validatePackageDirectory(target);
        if (JSON.stringify(retained.manifest) !== JSON.stringify(manifest)) throw hostError('host_package_identity_conflict');
      }
      this.store.db.exec('BEGIN IMMEDIATE');
      try {
        this.store.db.prepare('INSERT INTO applications(id,version,digest,path,manifest,data_path,data_version) VALUES(?,?,?,?,?,?,?)').run(manifest.id, manifest.version, digest, target, JSON.stringify(manifest), retained?.data_path ?? null, retained?.data_version ?? 'legacy');
        this.store.db.prepare('DELETE FROM application_retained WHERE app_id=?').run(manifest.id);
        this.store.db.exec('COMMIT');
      } catch (error) { this.store.db.exec('ROLLBACK'); throw error; }
      return this.view(this.store.application(manifest.id));
    } finally { this.changing.delete(manifest.id); await rm(staging, { recursive: true, force: true }); }
  }
  view(app) {
    const worker = this.workers.get(app.id);
    return { id: app.id, name: app.id, version: app.version, enabled: app.enabled, revision: app.revision, dataVersion: app.data_version,
      status: worker?.stopped ? 'failed' : app.status, busy: Boolean(worker?.active), manifest: app.manifest,
      permissions: app.manifest.permissions, grants: this.store.grants(app.id),
      pages: app.manifest.pages.map(page => ({ ...page, url: `/apps/${app.id}/${app.version}/pages/${page.id}` })) };
  }
  list() { return this.store.applications().map(app => this.view(app)); }
  async worker(app) {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    if (this.upgradeWorkers?.has(app.id)) throw hostError('host_app_busy', 409);
    if (this.startingWorkers.has(app.id)) return this.startingWorkers.get(app.id);
    const existing = this.workers.get(app.id);
    if (existing?.stopped) throw hostError('host_app_failed');
    if (existing) { await existing.start(); return existing; }
    const pending = (async () => {
      const checked = await validatePackageDirectory(app.path);
      if (createHash('sha256').update(JSON.stringify(checked.manifest)).digest('hex') !== app.digest) throw hostError('host_app_integrity_changed');
      const data = app.data_path ?? join(this.root, 'data', app.id);
      const learningLegacy = app.data_path === null || app.data_path === undefined;
      if (typeof data !== 'string' || !isAbsolute(data) || learningLegacy && app.data_version !== 'legacy') throw hostError('host_app_data_invalid');
      // Only a never-started install proves that no prior data must exist.
      if (learningLegacy && !app.enabled && app.revision === 1) await mkdir(data, { recursive: true, mode: 0o700 });
      let directory;
      try { directory = await lstat(data); }
      catch (error) {
        if (['ENOENT', 'ENOTDIR'].includes(error.code)) throw hostError('host_app_data_missing');
        throw error;
      }
      if (!directory.isDirectory() || directory.isSymbolicLink()) throw hostError('host_app_data_invalid');
      if (learningLegacy) {
        const learned = this.store.db.prepare("UPDATE applications SET data_path=? WHERE id=? AND digest=? AND version=? AND revision=? AND data_path IS NULL AND data_version='legacy'")
          .run(data, app.id, app.digest, app.version, app.revision);
        if (learned.changes !== 1) throw hostError('host_app_revision_conflict', 409);
      }
      const worker = this.processFactory(app, data); this.workers.set(app.id, worker);
      await worker.start(); return worker;
    })();
    this.startingWorkers.set(app.id, pending);
    try { return await pending; }
    finally { if (this.startingWorkers.get(app.id) === pending) this.startingWorkers.delete(app.id); }
  }
  async setEnabled(id, enabled) {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    if (typeof enabled !== 'boolean') throw hostError('host_app_state_invalid');
    const app = this.store.application(id);
    if (this.changing.has(id)) throw hostError('host_app_changing', 409);
    this.changing.add(id);
    try {
      if (enabled) await this.worker(app);
      this.store.db.exec('BEGIN IMMEDIATE');
      try {
        this.store.db.prepare('UPDATE applications SET enabled=?,revision=revision+1 WHERE id=?').run(Number(enabled), id);
        if (!enabled) this.store.db.prepare("UPDATE application_grants SET permissions='[]',revision=revision+1 WHERE app_id=?").run(id);
        this.store.db.exec('COMMIT');
      } catch (error) { this.store.db.exec('ROLLBACK'); throw error; }
      // Disable changes admission, never force-kills an accepted operation.
      return this.view(this.store.application(id));
    } finally { this.changing.delete(id); }
  }
  setGrants(id, projectId, permissions, expectedRevision, disclosureRuntime = '') {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    const app = this.store.application(id); this.store.project(projectId);
    if (!app.enabled || this.changing.has(id)) throw hostError('host_app_disabled');
    if (!Array.isArray(permissions) || permissions.length > 128 || permissions.some(p => typeof p !== 'string' || !app.manifest.permissions.some(item => item.id === p))) throw hostError('host_app_permission_invalid');
    const current = this.store.grant(id, projectId);
    if (expectedRevision !== undefined && current.revision !== expectedRevision) throw hostError('host_grant_revision_conflict', 409);
    this.store.db.prepare(`INSERT INTO application_grants(app_id,project_id,permissions,revision,disclosure_runtime) VALUES(?,?,?,1,?)
      ON CONFLICT(app_id,project_id) DO UPDATE SET permissions=excluded.permissions,disclosure_runtime=excluded.disclosure_runtime,revision=application_grants.revision+1`).run(id, Number(projectId), JSON.stringify([...new Set(permissions)].sort()), disclosureRuntime);
    return this.store.grant(id, projectId);
  }
  capabilities(projectId, { runtimeIdentity } = {}) {
    this.store.project(projectId);
    return this.store.applications().filter(app => app.enabled && !this.changing.has(app.id) && !this.workers.get(app.id)?.stopped).flatMap(app => {
      const grant = this.store.grant(app.id, projectId);
      if (runtimeIdentity !== undefined && grant.disclosureRuntime !== runtimeIdentity) return [];
      return app.manifest.tools.filter(tool => grant.permissions.includes(tool.permission)).map(tool => ({ ...tool, appId: app.id, name: qualifiedToolName(app.id, tool.name), grantRevision: grant.revision }));
    });
  }
  async call(id, name, projectId, args, expectedGrantRevision, operationId = randomUUID()) {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    const scope = positiveId(projectId); this.store.project(scope);
    const app = this.store.application(id); const grant = this.store.grant(id, scope);
    const tool = app.manifest.tools.find(tool => tool.name === name);
    if (!app.enabled || this.changing.has(id) || !tool || !grant.permissions.includes(tool.permission)) throw hostError('host_tool_permission_denied', 403);
    if (expectedGrantRevision !== undefined && expectedGrantRevision !== grant.revision) throw hostError('host_grant_revision_conflict', 409);
    validateToolArguments(tool.inputSchema, args);
    const worker = await this.worker(app);
    // Enabling/starting a worker can yield: recheck before it sees arguments.
    this.#current(id, scope, app.revision, grant.revision);
    const result = await worker.call(name, args, { hostBoundProjectId: scope, grantRevision: grant.revision, operationId });
    this.#current(id, scope, app.revision, grant.revision);
    return result;
  }
  #current(id, projectId, appRevision, grantRevision) {
    const app = this.store.application(id); const grant = this.store.grant(id, projectId);
    if (this.closing || !app.enabled || app.revision !== appRevision || grant.revision !== grantRevision || this.changing.has(id)) throw hostError('host_tool_authority_changed', 403);
  }
  async uninstall(id) {
    const app = this.store.application(id);
    if (this.closing || this.changing.has(id)) throw hostError('host_app_changing', 409);
    if (this.startingWorkers.has(id)) throw hostError('host_app_busy', 409);
    if (this.upgradeWorkers?.has(id)) throw hostError('host_app_busy', 409);
    this.changing.add(id);
    try {
      const worker = this.workers.get(id);
      if (worker?.active) throw hostError('host_app_busy', 409);
      await worker?.stop();
      this.store.db.exec('BEGIN IMMEDIATE');
      try {
        const retainedData = app.data_path ?? (app.data_version === 'legacy' && app.revision > 1 ? join(this.root, 'data', id) : null);
        this.store.db.prepare('INSERT OR REPLACE INTO application_retained(app_id,version,digest,data_path,data_version,data_schema_version,retained_at) VALUES(?,?,?,?,?,?,?)').run(id, app.version, app.digest, retainedData, app.data_version, app.manifest.dataSchemaVersion, new Date().toISOString());
        this.store.db.prepare('DELETE FROM applications WHERE id=?').run(id);
        this.store.db.exec('COMMIT');
      } catch (error) { this.store.db.exec('ROLLBACK'); throw error; }
      this.workers.delete(id);
      // Retain both application data and immutable package bytes for recovery.
      return { id, uninstalled: true, dataRetained: true };
    } finally { this.changing.delete(id); }
  }
  async pageFile(id, version, kind, key) {
    const app = this.store.application(id);
    if (!app.enabled || app.version !== version) throw hostError('host_app_page_unavailable', 404);
    const path = kind === 'pages' ? app.manifest.pages.find(page => page.id === key)?.path : key;
    if (!path || !app.manifest.files.some(file => file.path === path)) throw hostError('host_app_page_unavailable', 404);
    const location = join(app.path, path);
    const canonical = await realpath(location); const packagePath = await realpath(app.path);
    if (!canonical.startsWith(packagePath + '/')) throw hostError('host_package_path_invalid');
    const bytes = await readFile(canonical);
    const expected = app.manifest.files.find(file => file.path === path).sha256;
    if (createHash('sha256').update(bytes).digest('hex') !== expected) throw hostError('host_app_integrity_changed');
    return { bytes, path };
  }
  async close() {
    const busy = this.upgradeWorkers?.size > 0 || this.startingWorkers.size > 0 || this.changing.size > 0 || [...this.workers.values()].some(worker => worker.active);
    if (busy) throw hostError('host_app_busy', 409);
    this.closing = true;
    try { for (const worker of this.workers.values()) await worker.stop(); }
    catch (error) { this.closing = false; throw error; }
  }
  upgrade(id, source, options) { return upgradeApplication(this, id, source, options); }
}
