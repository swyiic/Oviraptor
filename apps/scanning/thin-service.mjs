/**
 * Phase-1 thin service entry for Nest/scanning.
 * Bridges host stdio to local draft/status/journal handlers and the declared
 * Host-only proxy stubs in application.mjs. Does not import or rewrite agent_runtime / multi_agent /
 * homogeneous kernels; native execution remains in oviraptor-scan-service.
 */
import { readFile } from 'node:fs/promises';
import { createApplication } from './application.mjs';

export const PHASE1_TOOLS = Object.freeze(['draft_create', 'draft_cancel', 'task_status', 'journal_read']);
const HOST_PROXY_PERMISSIONS = Object.freeze({
  scan_from_assets: 'investigation.draft.write',
  scan_list: 'investigation.read',
  scan_status: 'investigation.read',
  scan_preflight: 'investigation.read',
  scan_recovery: 'investigation.read',
  scan_results: 'investigation.read',
  scan_start: 'investigation.execute',
  scan_pause: 'investigation.control',
  scan_resume: 'investigation.execute',
});

export function phase1ToolNames() {
  return [...PHASE1_TOOLS];
}

/** Local execution is draft-only; additional tools are fixed Host proxy stubs. */
export function assertPhase1Surface(manifest) {
  const names = (manifest?.tools ?? []).map(tool => tool.name);
  for (const required of ['draft_create', 'task_status', 'journal_read']) {
    if (!names.includes(required)) throw Error(`scanning_thin_surface_missing:${required}`);
  }
  for (const tool of manifest?.tools ?? []) {
    const { name, permission, inputSchema } = tool;
    if (PHASE1_TOOLS.includes(name)) continue;
    // Never accept arbitrary execute-style names or caller-selected host scope.
    // Actual proxy handlers independently refuse direct Node-worker execution.
    if (!Object.hasOwn(HOST_PROXY_PERMISSIONS, name)
      || permission !== HOST_PROXY_PERMISSIONS[name]
      || inputSchema?.type !== 'object' || inputSchema.additionalProperties !== false
      || !inputSchema.properties || Object.hasOwn(inputSchema.properties, 'projectId')) {
      throw Error(`scanning_thin_surface_forbidden:${name}`);
    }
  }
  if (new Set(names).size !== names.length) throw Error('scanning_thin_surface_duplicate');
  return names;
}

export function createThinScanningApplication(manifest, options = {}) {
  assertPhase1Surface(manifest);
  return createApplication(manifest, options);
}

export async function startThinScanningService(options = {}) {
  const { startStdioWorker } = await import('./sdk/worker.mjs');
  const manifest = options.manifest ?? JSON.parse(await readFile(new URL('./manifest.json', import.meta.url), 'utf8'));
  return startStdioWorker(createThinScanningApplication(manifest, options));
}
