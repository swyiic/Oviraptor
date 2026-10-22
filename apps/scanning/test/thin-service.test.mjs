import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { assertPhase1Surface, createThinScanningApplication, phase1ToolNames } from '../thin-service.mjs';

test('thin service accepts current Host proxies but every direct Node proxy invocation is refused', async () => {
  const manifest = JSON.parse(await readFile(new URL('../app.package.json', import.meta.url))).manifest;
  assert.deepEqual(phase1ToolNames(), ['draft_create', 'draft_cancel', 'task_status', 'journal_read']);
  const proxies=['scan_from_assets','scan_list','scan_status','scan_preflight','scan_results','scan_start','scan_pause','scan_resume'];
  assert.deepEqual(assertPhase1Surface(manifest).sort(), [...phase1ToolNames(),...proxies].sort());
  const app = createThinScanningApplication(manifest);
  assert.equal(typeof app.tools.task_status, 'function');
  for(const name of proxies)await assert.rejects(app.tools[name]({},{}),/host_executor_required/,name);
});

test('phase1 surface rejects execute-style tool declarations', () => {
  assert.throws(() => assertPhase1Surface({ tools: [{ name: 'draft_create' }, { name: 'task_status' }, { name: 'journal_read' }, { name: 'scan_start' }] }), /scanning_thin_surface_forbidden:scan_start/);
});

test('thin service rejects unknown native tools, widened Host proxy permissions and caller project scope',async()=>{
 const manifest=JSON.parse(await readFile(new URL('../app.package.json',import.meta.url))).manifest;
 const changed=patch=>({...manifest,tools:manifest.tools.map(tool=>tool.name==='scan_start'?{...tool,...patch}:tool)});
 for(const patch of [{name:'scan_retry'},{name:'scan_workbench_start'},{permission:'investigation.read'},
  {inputSchema:{type:'object',properties:{},additionalProperties:true}},
  {inputSchema:{type:'object',properties:{projectId:{type:'integer'}},additionalProperties:false}}]){
  assert.throws(()=>createThinScanningApplication(changed(patch)),/scanning_thin_surface_forbidden/);
 }
 assert.throws(()=>assertPhase1Surface({...manifest,tools:[...manifest.tools,manifest.tools[0]]}),/scanning_thin_surface_duplicate/);
});
