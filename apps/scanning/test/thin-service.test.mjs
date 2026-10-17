import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { assertPhase1Surface, createThinScanningApplication, phase1ToolNames } from '../thin-service.mjs';

test('phase1 thin surface declares draft, status and journal only', async () => {
  const manifest = JSON.parse(await readFile(new URL('../app.package.json', import.meta.url))).manifest;
  assert.deepEqual(phase1ToolNames(), ['draft_create', 'draft_cancel', 'task_status', 'journal_read']);
  assert.deepEqual(assertPhase1Surface(manifest).sort(), phase1ToolNames().slice().sort());
  assert.equal(manifest.tools.some(tool => tool.name === 'scan_start'), false);
  const app = createThinScanningApplication(manifest);
  assert.equal(typeof app.tools.task_status, 'function');
  assert.equal(app.tools.scan_start, undefined);
});

test('phase1 surface rejects execute-style tool declarations', () => {
  assert.throws(() => assertPhase1Surface({ tools: [{ name: 'draft_create' }, { name: 'task_status' }, { name: 'journal_read' }, { name: 'scan_start' }] }), /scanning_thin_surface_forbidden:scan_start/);
});
