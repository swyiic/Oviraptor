import { mkdir, readFile, writeFile, copyFile, rename, rm, realpath, lstat, chmod } from 'node:fs/promises';
import { join, dirname, isAbsolute } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { validatePackageDirectory, validateToolArguments, validateActionArguments, qualifiedToolName } from '../../app-sdk/src/index.mjs';
import { AppProcess } from './app-process.mjs';
import { hostError, positiveId } from './store.mjs';
import { upgradeApplication } from './upgrades.mjs';
import { CertificateTasks, CERTIFICATE_TOOLS } from './certificate-tasks.mjs';
import { DiscoveryEnrichment, ENRICHMENT_TOOLS } from './discovery-enrichment.mjs';
import { DiscoveryAssist, DISCOVERY_ASSIST_TOOLS } from './discovery-assist.mjs';
import { ScanningTasks, SCANNING_TOOLS, validateScanningResultsArguments } from './scanning-tasks.mjs';
import { resolveDesktopAssetIds } from '../../app-sdk/src/desktop-asset-sync.mjs';
import { NativeOwners } from './native-owners.mjs';
import { shellEnrichmentForApp } from './shell-apps.mjs';
import {
  ASSETS_APP_ID, SCANNING_APP_ID, LEGACY_TOOL_BY_APP, LEGACY_BRIDGE_TRANSPORT_BLOCKER,
  probeLegacyBridge, stageLegacyInputSnapshot, createHostLegacyBridge, legacyToolResult,
  appDataDirectory, resolveLegacyAppWorkerPath,
} from './legacy-bridge.mjs';

export class AppManager {
  constructor(store, root, { processFactory, legacyAppWorkerPath, legacyBridgeAdapters } = {}) {
    this.store = store; this.root = root; this.processFactory = processFactory ?? ((app, data) => new AppProcess(app, data, { onActivity: event => this.onActivity?.(event) })); this.workers = new Map(); this.startingWorkers = new Map(); this.changing = new Set();
    this.nativeOwners = new NativeOwners(store.db);
    this.certificateTasks = new CertificateTasks();
    this.discoveryEnrichment = new DiscoveryEnrichment();
    this.discoveryAssist = new DiscoveryAssist();
    this.scanningTasks = new ScanningTasks();
    // Host-managed legacy bridge: optional worker binary + per-app test adapters.
    this.legacyAppWorkerPath = legacyAppWorkerPath ?? resolveLegacyAppWorkerPath() ?? undefined;
    this.legacyBridgeAdapters = legacyBridgeAdapters instanceof Map ? legacyBridgeAdapters : new Map(Object.entries(legacyBridgeAdapters ?? {}));
  }
  async install(source) {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    if (typeof source !== 'string') throw hostError('host_package_path_invalid');
    const checked = await validatePackageDirectory(source);
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    const manifest = checked.manifest;
    this.checkPlatform(manifest);
    this.nativeOwners.assertSettled(manifest.id);
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
        if (manifest.schemaVersion === 2 && file.path === manifest.entry) await chmod(destination, 0o700);
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
        this.nativeOwners.assertSettled(manifest.id);
        this.store.db.prepare('INSERT INTO applications(id,version,digest,path,manifest,data_path,data_version) VALUES(?,?,?,?,?,?,?)').run(manifest.id, manifest.version, digest, target, JSON.stringify(manifest), retained?.data_path ?? null, retained?.data_version ?? 'legacy');
        this.store.db.prepare('DELETE FROM application_retained WHERE app_id=?').run(manifest.id);
        this.store.db.exec('COMMIT');
      } catch (error) { this.store.db.exec('ROLLBACK'); throw error; }
      return this.view(this.store.application(manifest.id));
    } finally { this.changing.delete(manifest.id); await rm(staging, { recursive: true, force: true }); }
  }
  view(app) {
    const worker = this.workers.get(app.id);
    const ownerUnresolved = this.ownerUnresolved(app.id, worker);
    const shell = shellEnrichmentForApp(app.id);
    const pairedSkills = Array.isArray(app.manifest?.paired_skills)
      ? app.manifest.paired_skills.map(item => ({ name: item.name, source: item.source, required: Boolean(item.required) }))
      : [];
    return { id: app.id, name: app.id, version: app.version, enabled: app.enabled, revision: app.revision, dataVersion: app.data_version,
      builtin: Boolean(app.builtin), uninstallable: app.uninstallable !== false,
      status: ownerUnresolved ? 'owner_unresolved' : worker?.stopped ? 'failed' : app.status, busy: Boolean(ownerUnresolved || worker?.active || worker?.native && worker.nativeBusy !== false), manifest: app.manifest,
      permissions: app.manifest.permissions, grants: this.store.grants(app.id), pairedSkills,
      pages: app.manifest.pages.map(page => ({ ...page, url: `/apps/${app.id}/${app.version}/pages/${page.id}` })),
      ...(shell ?? {}) };
  }
  checkPlatform(manifest) {
    if (manifest.schemaVersion === 2 && (manifest.runtime.platform !== process.platform || manifest.runtime.arch !== process.arch)) throw hostError('host_app_platform_incompatible');
  }
  list() { return this.store.applications().map(app => this.view(app)); }
  ownerUnresolved(id, worker = this.workers.get(id)) {
    const owner = this.nativeOwners.get(id);
    if (!owner) {
      const app = this.store.applications().find(item => item.id === id);
      return Boolean(app?.manifest.schemaVersion === 2 && (app.enabled || app.revision !== 1 || app.data_path != null));
    }
    return Boolean(owner && owner.state !== 'stopped' && (owner.state === 'unresolved' || worker?.nativeOwnerTicket?.nonce !== owner.generation_nonce));
  }
  createWorker(app, data, nodeUpgradeProof) {
    const ticket = app.manifest.schemaVersion === 2 ? this.nativeOwners.reserve(app, data, nodeUpgradeProof) : null;
    try {
      const worker = this.processFactory(app, data);
      if (ticket) {
        worker.nativeOwnerTicket = ticket;
        worker.onOwnerState = event => this.nativeOwners.transition(ticket, event);
      }
      return worker;
    } catch (error) {
      if (ticket) { try { this.nativeOwners.transition(ticket, { state: 'unresolved' }); } catch { /* The already durable starting row remains fail-closed. */ } }
      throw error;
    }
  }
  async startWorker(worker, app) {
    try {
      if (this.ownerUnresolved(app.id, worker)) throw hostError('host_app_owner_unresolved', 409);
      await worker.start();
      if (worker.nativeOwnerTicket) this.nativeOwners.transition(worker.nativeOwnerTicket, { state: 'running', pid: worker.child?.pid });
      return worker;
    } catch (error) {
      if (worker.nativeOwnerTicket) { try { this.nativeOwners.transition(worker.nativeOwnerTicket, { state: 'unresolved', pid: worker.child?.pid }); } catch { /* Preserve the original failure and prior durable owner. */ } }
      throw error;
    }
  }
  async worker(app) {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    if (this.ownerUnresolved(app.id)) throw hostError('host_app_owner_unresolved', 409);
    if (this.upgradeWorkers?.has(app.id)) throw hostError('host_app_busy', 409);
    if (this.startingWorkers.has(app.id)) return this.startingWorkers.get(app.id);
    const existing = this.workers.get(app.id);
    if (existing?.stopped) throw hostError('host_app_failed');
    if (existing) { await this.startWorker(existing, app); return existing; }
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
      const worker = this.createWorker(app, data); this.workers.set(app.id, worker);
      await this.startWorker(worker, app); return worker;
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
    const next = [...new Set(permissions)].sort();
    const narrowed = current.permissions.some(permission => !next.includes(permission));
    this.store.db.prepare(`INSERT INTO application_grants(app_id,project_id,permissions,revision,disclosure_runtime) VALUES(?,?,?,1,?)
      ON CONFLICT(app_id,project_id) DO UPDATE SET permissions=excluded.permissions,disclosure_runtime=excluded.disclosure_runtime,revision=application_grants.revision+1`).run(id, Number(projectId), JSON.stringify(next), disclosureRuntime);
    if (narrowed) this.store.purgeProjectRecords(projectId);
    return this.store.grant(id, projectId);
  }
  /** Data directory used for an installed app (learned path or root/data/<id>). */
  dataDirectoryFor(app) {
    return appDataDirectory(this.root, app);
  }

  /** Probe whether host can expose legacy read tools for this app right now. */
  legacyBridgeProbe(app) {
    const dataDir = this.dataDirectoryFor(app);
    if (!dataDir) return { available: false, code: 'host_app_data_missing' };
    return probeLegacyBridge({
      appId: app.id,
      dataDir,
      workerPath: this.legacyAppWorkerPath,
      adapter: this.legacyBridgeAdapters.get(app.id),
    });
  }

  /**
   * Stage a desktop snapshot into the app data dir as legacy-input.sqlite3.
   * Does not enable tools unless an app-worker / adapter can also serve it.
   */
  async stageLegacySnapshot(appId, sourcePath) {
    const app = this.store.application(appId);
    if (![ASSETS_APP_ID, SCANNING_APP_ID].includes(app.id)) throw hostError('host_legacy_app_unsupported');
    const dataDir = this.dataDirectoryFor(app);
    if (!dataDir) throw hostError('host_app_data_missing');
    return stageLegacyInputSnapshot(dataDir, sourcePath);
  }

  legacyToolSpec(app) {
    const probe = this.legacyBridgeProbe(app);
    if (!probe.available) return null;
    return probe.tool ?? LEGACY_TOOL_BY_APP[app.id] ?? null;
  }

  effectiveGrant(app, projectId, identity) {
    const base=this.store.grant(app.id,projectId);
    return this.agentGrant?this.agentGrant(app,projectId,base,identity):base;
  }
  capabilities(projectId, { runtimeIdentity } = {}) {
    this.store.project(projectId);
    return this.store.applications().filter(app => app.enabled && !this.changing.has(app.id) && !this.workers.get(app.id)?.stopped && !this.ownerUnresolved(app.id)).flatMap(app => {
      const grant = this.effectiveGrant(app, projectId, runtimeIdentity);
      if (runtimeIdentity !== undefined && grant.disclosureRuntime !== runtimeIdentity) return [];
      const tools = [...app.manifest.tools];
      const legacy = this.legacyToolSpec(app);
      if (legacy && !tools.some(tool => tool.name === legacy.name)) tools.push(legacy);
      return tools.filter(tool => grant.permissions.includes(tool.permission) && (!DISCOVERY_ASSIST_TOOLS.has(tool.name) || app.id===ASSETS_APP_ID && this.discoveryAssist.available) && (!ENRICHMENT_TOOLS.has(tool.name) || app.id===ASSETS_APP_ID && this.discoveryEnrichment.available) && (!CERTIFICATE_TOOLS.has(tool.name) || app.id === ASSETS_APP_ID && this.certificateTasks.available) && (app.manifest.schemaVersion===2 || !SCANNING_TOOLS.has(tool.name) || app.id===SCANNING_APP_ID && this.scanningTasks.available)).map(tool => ({ ...tool, appId: app.id, name: qualifiedToolName(app.id, tool.name), grantRevision: grant.revision, ...(grant.accessPolicyRevision!==undefined?{accessPolicyRevision:grant.accessPolicyRevision}:{}) }));
    });
  }
  async call(id, name, projectId, args, expectedGrantRevision, operationId = randomUUID(), pageContext, agentRuntimeIdentity) {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    const scope = positiveId(projectId); this.store.project(scope);
    const app = this.store.application(id); const grant = this.effectiveGrant(app, scope, agentRuntimeIdentity);
    if (pageContext && (app.revision !== pageContext.expectedAppRevision || app.version !== pageContext.version || !app.manifest.pages.some(page => page.id === pageContext.pageId))) throw hostError('host_app_page_unavailable', 409);
    const legacySpec = this.legacyToolSpec(app);
    const tool = app.manifest.tools.find(tool => tool.name === name)
      ?? (legacySpec && legacySpec.name === name ? legacySpec : undefined);
    if (!app.enabled || this.changing.has(id) || !tool || !grant.permissions.includes(tool.permission)) throw hostError('host_tool_permission_denied', 403);
    if (expectedGrantRevision !== undefined && expectedGrantRevision !== grant.revision) throw hostError('host_grant_revision_conflict', 409);
    validateToolArguments(tool.inputSchema, args);
    // Host-routed legacy reads: do not send to the Node worker (no child-process grant there).
    if (legacySpec && name === legacySpec.name) {
      this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
      const dataDir = this.dataDirectoryFor(app);
      const bridge = createHostLegacyBridge({
        appId: app.id,
        dataDir,
        workerPath: this.legacyAppWorkerPath,
        adapter: this.legacyBridgeAdapters.get(app.id),
      });
      const context = this.#callContext(app, grant, scope, operationId, tool.permission);
      const structured = await bridge({ appId: app.id, name: legacySpec.rustName, arguments: args, context });
      this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
      return legacyToolResult(structured);
    }
    if (app.id === ASSETS_APP_ID && DISCOVERY_ASSIST_TOOLS.has(name)) {
      this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
      const result = await this.discoveryAssist.call(this.store, scope, name, args, operationId);
      this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
      return result;
    }
    if (app.id === ASSETS_APP_ID && ENRICHMENT_TOOLS.has(name)) {
      this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
      const result=await this.discoveryEnrichment.call(this.store,scope,name,args,operationId);
      this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
      return result;
    }
    if (app.id === ASSETS_APP_ID && CERTIFICATE_TOOLS.has(name)) {
      this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
      let nativeArgs=args,binding;
      if(name==='certificate_task_start'){
        const {assetIdNamespace,...rest}=args;nativeArgs=rest;
        if(args.assetIds!==undefined){
          try{binding=resolveDesktopAssetIds({inventoryDbPath:join(this.dataDirectoryFor(app),'inventory.sqlite3'),hostProjectId:scope,assetIds:args.assetIds,assetIdNamespace});}
          catch(error){throw hostError('host_certificate_asset_identity_unavailable',409,{reason:error.code,retryable:false,outcomeUnknown:false});}
          nativeArgs={...rest,assetIds:binding.desktopAssetIds};
        }else if(assetIdNamespace!==undefined)throw hostError('host_tool_input_invalid',400,{retryable:false});
      }
      const result = await this.certificateTasks.call(this.store, scope, name, nativeArgs, operationId);
      this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
      if(binding){const structuredContent={...result.structuredContent,assetBinding:binding};return{...result,structuredContent,content:[{type:'text',text:JSON.stringify(structuredContent)}]};}
      return result;
    }
    if (app.id===SCANNING_APP_ID && app.manifest.schemaVersion===1 && SCANNING_TOOLS.has(name)) {
      if(name==='scan_results')validateScanningResultsArguments(args);
      this.#current(id,scope,app.revision,grant.revision,grant.accessPolicyRevision,grant.accessRuntimeIdentity);
      let nativeArgs=args,binding;
      if(name==='scan_from_assets'){
        const assets=this.store.application(ASSETS_APP_ID);
        if(!assets.enabled||this.changing.has(assets.id))throw hostError('host_scan_assets_unavailable',409,{retryable:false,outcomeUnknown:false});
        try{binding=resolveDesktopAssetIds({inventoryDbPath:join(this.dataDirectoryFor(assets),'inventory.sqlite3'),hostProjectId:scope,assetIds:args.assetIds,assetIdNamespace:args.assetIdNamespace});}
        catch(error){throw hostError('host_scan_asset_identity_unavailable',409,{reason:error.code,retryable:false,outcomeUnknown:false});}
        nativeArgs={assetIds:binding.desktopAssetIds,...(args.scanMode?{scanMode:args.scanMode}:{}),...(args.maxModelRequests!==undefined?{maxModelRequests:args.maxModelRequests}:{}),...(args.maxModelTokens!==undefined?{maxModelTokens:args.maxModelTokens}:{})};
      }
      const result=await this.scanningTasks.call(this.store,scope,name,nativeArgs,operationId);
      this.#current(id,scope,app.revision,grant.revision,grant.accessPolicyRevision,grant.accessRuntimeIdentity);
      if(binding){const structuredContent={...result.structuredContent,assetBinding:binding};return{...result,structuredContent,content:[{type:'text',text:JSON.stringify(structuredContent)}]};}
      return result;
    }
    const worker = await this.worker(app);
    // Enabling/starting a worker can yield: recheck before it sees arguments.
    this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
    let result;
    try { result = await worker.call(name, args, this.#callContext(app, grant, scope, operationId, tool.permission)); }
    catch (error) { this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity); throw error; }
    this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
    return result;
  }
  #callContext(app, grant, scope, operationId, permission) {
    const base = { hostBoundProjectId: scope, grantRevision: grant.revision, operationId };
    if (app.manifest.schemaVersion !== 2) return base;
    const project = this.store.project(scope);
    if (typeof operationId !== 'string' || !operationId || operationId.length > 128 || /[\u0000-\u001f\u007f]/u.test(operationId)) throw hostError('host_operation_invalid');
    return { ...base, appRevision: app.revision, permission, permissions: grant.permissions,
      project: { id: project.id, name: project.name, status: 'active' } };
  }
  async confirmDiscoveryScope(projectId, args, witness) {
    const id=ASSETS_APP_ID, scope=positiveId(projectId), app=this.store.application(id);
    const grant=this.effectiveGrant(app,scope,witness.runtimeIdentity);
    if(!app.enabled||this.changing.has(id)||!grant.permissions.includes('certificate.collect')
      ||grant.disclosureRuntime!==witness.runtimeIdentity||app.revision!==witness.appRevision||grant.revision!==witness.grantRevision)
      throw hostError('host_tool_authority_changed',403);
    this.#current(id,scope,app.revision,grant.revision,grant.accessPolicyRevision,grant.accessRuntimeIdentity);
    const result=await this.discoveryAssist.confirm(this.store,scope,args,randomUUID());
    this.#current(id,scope,app.revision,grant.revision,grant.accessPolicyRevision,grant.accessRuntimeIdentity);
    return result;
  }
  async callAction(id, name, projectId, args, { expectedAppRevision, expectedGrantRevision, operationId = randomUUID(), pageId, applicationScopeAccepted = false } = {}) {
    if (this.closing) throw hostError('host_app_manager_closing', 409);
    const scope = positiveId(projectId); this.store.project(scope);
    const app = this.store.application(id), grant = this.store.grant(id, scope);
    const action = app.manifest.schemaVersion === 2 ? app.manifest.actions.find(item => item.name === name) : undefined;
    if (!app.enabled || this.changing.has(id) || !action || !grant.permissions.includes(action.permission)) throw hostError('host_action_permission_denied', 403);
    if (!app.manifest.pages.some(page => page.id === pageId)) throw hostError('host_app_page_unavailable', 404);
    if (expectedAppRevision !== app.revision || expectedGrantRevision !== grant.revision) throw hostError('host_action_revision_conflict', 409);
    if (action.scope === 'application' && applicationScopeAccepted !== true) throw hostError('host_application_scope_confirmation_required', 403);
    validateActionArguments(action.inputSchema, args);
    const worker = await this.worker(app);
    this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
    let result;
    try { result = await worker.action(name, args, this.#callContext(app, grant, scope, operationId, action.permission)); }
    catch (error) { this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity); throw error; }
    this.#current(id, scope, app.revision, grant.revision, grant.accessPolicyRevision, grant.accessRuntimeIdentity);
    return result;
  }
  #current(id, projectId, appRevision, grantRevision, accessPolicyRevision, runtimeIdentity) {
    const app = this.store.application(id); const grant = this.effectiveGrant(app, projectId, runtimeIdentity);
    if (this.closing || !app.enabled || app.revision !== appRevision || grant.revision !== grantRevision || grant.accessPolicyRevision !== accessPolicyRevision || this.changing.has(id)) throw hostError('host_tool_authority_changed', 403);
  }
  async uninstall(id) {
    const app = this.store.application(id);
    if (app.builtin || app.uninstallable === false) throw hostError('host_app_builtin', 403);
    if (this.closing || this.changing.has(id)) throw hostError('host_app_changing', 409);
    if (this.startingWorkers.has(id)) throw hostError('host_app_busy', 409);
    if (this.upgradeWorkers?.has(id)) throw hostError('host_app_busy', 409);
    this.changing.add(id);
    try {
      const worker = this.workers.get(id);
      if (this.ownerUnresolved(id, worker)) throw hostError('host_app_owner_unresolved', 409);
      if (worker?.active) throw hostError('host_app_busy', 409);
      await worker?.stop();
      this.nativeOwners.assertSettled(id);
      this.store.db.exec('BEGIN IMMEDIATE');
      try {
        this.nativeOwners.assertSettled(id);
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
  /** Documented blocker: Node worker cannot receive injected legacyBridge over stdio. */
  get legacyBridgeTransportBlocker() { return LEGACY_BRIDGE_TRANSPORT_BLOCKER; }
}

