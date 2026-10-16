/**
 * Phase-1 thin service entry for Nest/scanning.
 * Bridges host stdio to the existing draft/status/journal surface in
 * application.mjs. Does not import or rewrite agent_runtime / multi_agent /
 * homogeneous kernels; native execution remains in oviraptor-scan-service.
 */
import { readFile } from 'node:fs/promises';
import { createApplication } from './application.mjs';

export const PHASE1_TOOLS = Object.freeze(['draft_create', 'draft_cancel', 'task_status', 'journal_read', 'legacy_saved_scans']);

export function phase1ToolNames() {
  return [...PHASE1_TOOLS];
}

/** Validate a Phase-1 manifest exposes only the non-sensitive surface. */
export function assertPhase1Surface(manifest) {
  const names = (manifest?.tools ?? []).map(tool => tool.name);
  for (const required of ['draft_create', 'task_status', 'journal_read']) {
    if (!names.includes(required)) throw Error(`scanning_thin_surface_missing:${required}`);
  }
  for (const name of names) {
    if (/^(scan_start|scan_resume|scan_retry|scan_workbench|start_scan)/u.test(name)) {
      throw Error(`scanning_thin_surface_forbidden:${name}`);
    }
  }
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
