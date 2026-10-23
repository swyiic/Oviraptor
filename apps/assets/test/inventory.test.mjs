import test from 'node:test';import assert from 'node:assert/strict';import {mkdtemp,readFile,rm} from 'node:fs/promises';import {tmpdir} from 'node:os';import {join} from 'node:path';import {fileURLToPath} from 'node:url';import {createApplication} from '../application.mjs';import {createDirectoryPackage,validatePackageDirectory} from '../../../packages/app-sdk/src/index.mjs';
const config=JSON.parse(await readFile(new URL('../app.package.json',import.meta.url)));const ctx=(project=91,operation='op-1')=>({hostBoundProjectId:project,grantRevision:1,operationId:operation});
const expectedTools={
 inventory_import:'inventory.write',inventory_stats:'inventory.read',inventory_search:'inventory.read',inventory_schema:'inventory.read',inventory_query:'inventory.read',project_overview:'inventory.read',insight_propose:'inventory.read',insight_commit:'inventory.write',exposure_overview:'inventory.exposure.read',
 certificate_task_start:'certificate.collect',certificate_task_status:'inventory.read',certificate_task_pause:'certificate.collect',certificate_task_resume:'certificate.collect',
 discovery_enrichment_start:'certificate.collect',discovery_enrichment_status:'inventory.read',discovery_enrichment_pause:'certificate.collect',discovery_enrichment_resume:'certificate.collect',
 discovery_assist_start:'inventory.write',discovery_assist_status:'inventory.read',discovery_assist_pause:'inventory.write',discovery_review_read:'inventory.read',discovery_scope_propose:'inventory.read',
 discovery_seed_list:'inventory.read',discovery_seed_detail:'inventory.read',
 exposure_findings:'inventory.exposure.details.read',exposure_finding:'inventory.exposure.details.read',exposure_runs:'inventory.exposure.read',exposure_task_status:'inventory.exposure.read',exposure_task_start:'inventory.exposure.run',exposure_task_resume:'inventory.exposure.run',exposure_task_cancel:'inventory.exposure.run',
};
const hostProxyNames=Object.keys(expectedTools).filter(name=>/^(certificate_task_|discovery_|exposure_(?!overview))/u.test(name));
async function fixture(t){const dir=await mkdtemp(join(tmpdir(),'oviraptor-assets-app-'));const app=createApplication(config.manifest);await app.onStart({dataDir:dir,context:{appId:config.manifest.id,hostApiVersion:'1.0.0'}});t.after(async()=>{await app.onStop();await rm(dir,{recursive:true,force:true});});return{dir,app,call:async(name,args={},context=ctx())=>(await app.tools[name](args,context)).structuredContent};}
test('real app SQLite import is project scoped, idempotent and never weak-host merges or confirms',async t=>{const f=await fixture(t);const args={source:'explicit-upload',records:[{externalId:'1',host:'www.example.test',protocol:'https',title:'old',statusCode:'200'},{externalId:'2',host:'example.test',protocol:'ssh',port:'22'}]};const first=await f.call('inventory_import',args);assert.equal(first.total,2);assert.equal(first.authorization,'not_granted');assert.deepEqual(await f.call('inventory_import',args),first);await assert.rejects(f.call('inventory_import',{...args,source:'different'}),/app_operation_conflict/);await f.call('inventory_import',{source:'explicit-upload',records:[{externalId:'1',host:'www.example.test',protocol:'ssh',port:'22'}]},ctx(91,'new-observation'));const page=await f.call('inventory_search');assert.equal(page.total,2);assert.equal(page.items[0].title,'');assert.equal(page.items[0].protocol,'ssh');assert.equal((await f.call('inventory_stats',{},ctx(92))).total,0);await f.call('inventory_import',args,ctx(92));assert.equal((await f.call('inventory_stats',{},ctx(91))).total,2);});
test('exact counts exceed 20, unknown buckets close and bound cursor cannot cross project/filter',async t=>{const f=await fixture(t);await f.call('inventory_import',{source:'fixture',records:Array.from({length:25},(_,i)=>({externalId:String(i),host:`host-${i}.test`,statusCode:i===0?'200oops':i===1?'':'200',probeOutcome:i===0?'future':''}))});const stats=await f.call('inventory_stats');assert.equal(stats.total,25);assert.equal(Object.values(stats.byHttp).reduce((a,b)=>a+b,0),25);assert.equal(stats.byHttp.unknown,1);assert.equal(stats.byProbe.unknown,1);const first=await f.call('inventory_search');assert.equal(first.returned,20);assert.equal(first.hasMore,true);const next=await f.call('inventory_search',{cursor:first.nextCursor});assert.equal(next.returned,5);assert.equal(next.items[0].id,21);await assert.rejects(f.call('inventory_search',{cursor:first.nextCursor},ctx(92)),/asset_cursor_invalid/);await assert.rejects(f.call('inventory_search',{cursor:first.nextCursor,query:'host-1'}),/asset_cursor_invalid/);});
test('invalid bulk input is atomic and legacy_snapshot_stats is not registered without legacyBridge',async t=>{const f=await fixture(t);await assert.rejects(f.call('inventory_import',{source:'fixture',records:[{externalId:'ok',host:'ok.test'},{externalId:'bad',link:'https://fixture:synthetic@bad.test/'}]}),/asset_link_invalid/);assert.equal((await f.call('inventory_stats')).total,0);assert.equal(f.app.tools.legacy_snapshot_stats,undefined);assert.equal(config.manifest.tools.some(tool=>tool.name==='legacy_snapshot_stats'),false);await assert.rejects(f.call('inventory_stats',{projectId:92}),/app_arguments_invalid/);await assert.rejects(f.call('inventory_stats',{},ctx(0)),/app_context_invalid/);});
test('directory package is generated from bytes with vendored SDK and declared page/tools',async t=>{const root=await mkdtemp(join(tmpdir(),'oviraptor-assets-package-'));t.after(()=>rm(root,{recursive:true,force:true}));const output=join(root,'package');await createDirectoryPackage({sourceDirectory:fileURLToPath(new URL('..',import.meta.url)),outputDirectory:output,...config});const checked=await validatePackageDirectory(output);assert.equal(checked.manifest.id,'org.oviraptor.assets');assert.ok(checked.manifest.files.some(f=>f.path==='sdk/worker.mjs'));assert.ok(checked.manifest.files.some(f=>f.path==='sdk/sqlite-data.mjs'));assert.ok(checked.manifest.files.some(f=>f.path==='sdk/legacy-bridge-stub.mjs'));
assert.ok(checked.manifest.files.some(f=>f.path==='sdk/insight.mjs'));
const app=createApplication(checked.manifest);
assert.deepEqual(checked.manifest.tools.map(tool=>tool.name).sort(),Object.keys(expectedTools).sort());
assert.deepEqual(Object.keys(app.tools).sort(),Object.keys(expectedTools).sort());
for(const tool of checked.manifest.tools){assert.equal(tool.permission,expectedTools[tool.name],tool.name);assert.equal(typeof app.tools[tool.name],'function',tool.name);assert.equal(tool.inputSchema.type,'object',tool.name);assert.equal(tool.inputSchema.additionalProperties,false,tool.name);}
assert.ok((await readFile(join(output,'page.html'),'utf8')).includes('oviraptor.app-ui.v1'));});

test('every declared discovery and certificate proxy refuses Node execution without changing inventory',async t=>{
 const f=await fixture(t);
 await f.call('inventory_import',{source:'fixture',records:[{externalId:'saved',host:'saved.example.test'}]});
 const before=await f.call('inventory_stats');
 for(const name of hostProxyNames)await assert.rejects(f.call(name,{},ctx(91,`direct-${name}`)),{code:name.startsWith('exposure_')?'host_exposure_executor_required':'host_certificate_executor_required'},name);
 const after=await f.call('inventory_stats');
 assert.deepEqual({...after,asOf:before.asOf},before);
});

test('inventory_schema is static whitelist and inventory_query enforces filter whitelist and limit 100',async t=>{
 const f=await fixture(t);
 const schema=await f.call('inventory_schema');
 assert.equal(schema.source,'static_whitelist');
 assert.equal(schema.tables[0].name,'inventory');
 assert.ok((schema.filterFields ?? schema.tables[0].filterFields).includes('protocol'));
 assert.match(schema.note,/does not query sqlite_master/i);
 await f.call('inventory_import',{source:'fixture',records:Array.from({length:5},(_,i)=>({externalId:String(i),host:`h${i}.test`,protocol:i%2?'ssh':'https',statusCode:'200'}))});
 const page=await f.call('inventory_query',{filter:{protocol:'https'},limit:10});
 assert.equal(page.total,3);
 assert.equal(page.returned,3);
 assert.equal(page.limitMax,100);
 await assert.rejects(f.call('inventory_query',{filter:{secretField:'x'}}),/asset_filter_field_denied/);
 await assert.rejects(f.call('inventory_query',{limit:101}),/app_arguments_invalid/);
 const first=await f.call('inventory_query',{limit:2});
 assert.equal(first.returned,2);assert.equal(first.hasMore,true);
 const next=await f.call('inventory_query',{limit:2,cursor:first.nextCursor});
 assert.equal(next.returned,2);
 await assert.rejects(f.call('inventory_query',{limit:2,cursor:first.nextCursor},ctx(92)),/asset_cursor_invalid/);
 const overview=await f.call('project_overview');
 assert.equal(overview.kind,'project_overview');
 assert.equal(overview.total,5);
 assert.equal(overview.legacySnapshot.available,false);
 assert.ok(overview.readTools.includes('inventory_schema'));
});


test('legacy_snapshot_stats registers only when legacyBridge is injected',async t=>{
 const dir=await mkdtemp(join(tmpdir(),'oviraptor-assets-legacy-gate-'));
 t.after(()=>rm(dir,{recursive:true,force:true}));
 const bridge=async()=>({available:true,total:3,unit:'legacy_snapshot'});
 const app=createApplication(config.manifest,{legacyBridge:bridge});
 await app.onStart({dataDir:dir,context:{appId:config.manifest.id,hostApiVersion:'1.0.0'}});
 t.after(()=>app.onStop());
 assert.equal(typeof app.tools.legacy_snapshot_stats,'function');
 const value=(await app.tools.legacy_snapshot_stats({},ctx())).structuredContent;
 assert.equal(value.available,true);
 assert.equal(value.total,3);
 assert.equal(config.manifest.tools.some(tool=>tool.name==='legacy_snapshot_stats'),false);
 assert.equal(app.manifest.tools.some(tool=>tool.name==='legacy_snapshot_stats'),true);
});
test('exposure_overview is aggregate-only: unsupported empty without bridge, strips injected payload rows', async t => {
  const f = await fixture(t);
  const empty = await f.call('exposure_overview');
  assert.equal(empty.kind, 'exposure_overview');
  assert.equal(empty.available, false);
  assert.equal(empty.code, 'legacy_managed_bridge_unavailable');
  assert.equal(empty.runs.total, 0);
  assert.equal(empty.findings.total, 0);
  assert.ok(empty.schema.forbidden.includes('payload'));
  assert.equal(Object.hasOwn(empty, 'payload'), false);
  assert.equal(Object.hasOwn(empty, 'items'), false);
  assert.equal(Object.hasOwn(empty.findings, 'items'), false);

  const dir = await mkdtemp(join(tmpdir(), 'oviraptor-assets-exposure-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const bridge = async ({ name }) => {
    if (name === 'exposure_summary') {
      return {
        total: 2,
        newCount: 1,
        reviewing: 0,
        confirmed: 1,
        dismissed: 0,
        highRisk: 1,
        eligibleAssets: 4,
        payload: { requestBody: 'SECRET_PROBE', credentials: 'tok' },
        items: [
          { severity: 'critical', status: 'new', payload: 'x', evidence_excerpt: 'leak', title: 'should-not-appear', source_url: 'https://evil.test' },
          { severity: 'low', status: 'confirmed', requestContent: 'nope', rawResponse: 'body' },
        ],
      };
    }
    if (name === 'list_exposure_runs') {
      return [
        { id: 1, status: 'completed', error: 'should-not-leak', current_source: 'probe', payload: 'x' },
        { id: 2, status: 'failed', credentials: 'y' },
      ];
    }
    return { available: false, code: 'legacy_managed_bridge_unavailable' };
  };
  const app = createApplication(config.manifest, { legacyBridge: bridge });
  await app.onStart({ dataDir: dir, context: { appId: config.manifest.id, hostApiVersion: '1.0.0' } });
  t.after(() => app.onStop());
  const overview = (await app.tools.exposure_overview({}, ctx())).structuredContent;
  assert.equal(overview.kind, 'exposure_overview');
  assert.equal(overview.available, true);
  assert.equal(overview.code, null);
  assert.equal(overview.runs.total, 2);
  assert.equal(overview.runs.byStatus.completed, 1);
  assert.equal(overview.runs.byStatus.failed, 1);
  assert.equal(overview.findings.total, 2);
  assert.equal(overview.findings.bySeverity.critical, 1);
  assert.equal(overview.findings.bySeverity.low, 1);
  assert.equal(overview.findings.byStatus.new, 1);
  assert.equal(overview.findings.byStatus.confirmed, 1);
  assert.equal(overview.findings.highRisk, 1);
  const text = JSON.stringify(overview);
  for (const leak of ['SECRET_PROBE', 'should-not-appear', 'https://evil.test', 'should-not-leak', 'tok']) {
    assert.equal(text.includes(leak), false, leak);
  }
  const bannedKeys = new Set(['payload', 'items', 'evidence_excerpt', 'evidenceExcerpt', 'requestContent', 'rawResponse', 'credentials', 'source_url', 'sourceUrl', 'title']);
  const walk = (value, path = '') => {
    if (!value || typeof value !== 'object') return;
    if (Array.isArray(value)) { assert.ok(path.includes('.schema'), 'arrays only allowed under schema: ' + path); return; }
    for (const [key, child] of Object.entries(value)) {
      assert.equal(bannedKeys.has(key), false, path + '.' + key);
      walk(child, path + '.' + key);
    }
  };
  walk(overview);
  assert.equal(Array.isArray(overview.runs), false);
});

test('insight_propose is inventory.read and insight_commit persists with userConfirmed gate', async t => {
  const f = await fixture(t);
  assert.equal(typeof f.app.tools.insight_propose, 'function');
  assert.equal(typeof f.app.tools.insight_commit, 'function');
  assert.equal(config.manifest.tools.find(tool => tool.name === 'insight_propose')?.permission, 'inventory.read');
  assert.equal(config.manifest.tools.find(tool => tool.name === 'insight_commit')?.permission, 'inventory.write');

  await assert.rejects(f.call('insight_propose', { inventoryRecordId: '1', fields: { secret: 'x' } }), /field_denied/);
  const proposal = await f.call('insight_propose', {
    inventoryRecordId: '42',
    fields: { note: 'likely ssh', tags: ['ssh'] },
    baseVersion: 0,
  });
  assert.ok(proposal.proposalId);
  assert.equal(proposal.committed, false);
  assert.match(proposal.diffSummary, /inventory 42 @ base v0/);
  assert.match(proposal.diffSummary, /note=likely ssh/);

  await assert.rejects(f.call('insight_commit', {
    proposalId: proposal.proposalId,
    operationId: 'commit-op-1',
    userConfirmed: false,
  }, ctx(91, 'tool-op-deny')), /confirm_required/);

  const committed = await f.call('insight_commit', {
    proposalId: proposal.proposalId,
    operationId: 'commit-op-1',
    userConfirmed: true,
  }, ctx(91, 'tool-op-commit'));
  assert.equal(committed.committed, true);
  assert.equal(committed.persistence, 'inventory.sqlite3');
  assert.equal(committed.version, 1);
  assert.equal(committed.fields.note, 'likely ssh');

  const replay = await f.call('insight_commit', {
    proposalId: proposal.proposalId,
    operationId: 'commit-op-1',
    userConfirmed: true,
  }, ctx(91, 'tool-op-commit-replay'));
  assert.equal(replay.idempotentReplay, true);
  assert.equal(replay.committed, true);

  const schema = await f.call('inventory_schema');
  assert.ok(schema.tables.some(table => table.name === 'inventory_insights'));
});
