const test = require('node:test');
const { assert, flush, deferred, mountAssetAccuracy } = require('./asset_accuracy_ui_harness.cjs');
const file = 'src/features/agentHost/components/DiscoveryAssistPanel.vue';
const verification = (host, kind = 'tls_subject') => ({ status: 'bound_local_evidence', sources: [{ kind, host, organization: '目标集团独立单位', assetId: 31 }] });
const view = (token = 'one') => ({ runId: 'saved-review', phase: 'awaiting_scope', enabled: true, organization: '明确目标', includeIsolated: false, reason: '项目提炼已完成',
  confirmationWitness: { appRevision: 1, grantRevision: 2, policyRevision: 3, runtimeIdentity: 'model:1' },
  review: { run: { processedAssets: 98989, totalAssets: 98989, groups: 12478, requests: 230, inputTokens: 500, outputTokens: 100 } },
  proposal: { token, roots: ['old.test'], removedNoLiveEvidence: 21, networkVerified: false, groups: [{ root: 'one.test', scopes: ['one.test'], evidenceGroups: 90, reasons: ['独立单位证据'], verification: verification('one.test') }, { root: 'shared.test', shared: true, scopes: ['tenant.shared.test'], evidenceGroups: 80, reasons: ['具体租户'], verification: verification('tenant.shared.test', 'icp') }] },
  enrichment: { run: { counts: { done: 7, retry: 8, failed: 2 } } } });
async function mount(t, overrides = {}, options = {}) {
  const calls = [];
  const client = { discoveryStatus: async (...args) => { calls.push(['read', ...args]); return view(); },
    confirmDiscoveryScope: async (...args) => { calls.push(['confirm', ...args]); return { scopeSaved: true, enrichmentStarted: true }; },
    discoveryControl: async (...args) => { calls.push(['control', ...args]); }, ...overrides };
  const h = await mountAssetAccuracy(file, { client, projectId: 1, conversationId: 'c1', refreshKey: 0 }, options);
  t.after(() => { h.unmount(); assert.deepEqual(h.warnings, []); });
  return { ...h, clientCalls: calls };
}
test('chat shows durable project progress, actual model usage and one grouped consent instead of per-certificate clicks', async t => {
  const h = await mount(t);
  assert.match(h.text(), /98,989/); assert.match(h.text(), /模型 230 次/); assert.match(h.text(), /600 Token/); assert.match(h.text(), /已省略 21 项/); assert.match(h.text(), /tenant.shared.test/);
  assert.match(h.text(), /等待重试 8/); assert.match(h.text(), /失败 2/); assert.match(h.text(), /Hunter 0 积分/);
  assert.equal(h.clientCalls.some(c => c[0] === 'confirm'), false);
  await h.click('data-testid', 'chat-discovery-confirm');
  const call = h.clientCalls.find(c => c[0] === 'confirm');
  assert.deepEqual(call.slice(1, 3), ['c1', 1]); assert.deepEqual(call[3].hosts, ['one.test', 'tenant.shared.test']);
  assert.equal(call[3].userConfirmed, true); assert.equal(call[3].policyRevision, 3);
});
test('chat shows bound local sources separately from model inference and does not claim online ownership verification', async t => {
  const h = await mount(t);
  assert.match(h.text(), /已核验的范围建议/); assert.match(h.text(), /本次仅核验本地证据，未联网查证企业归属/);
  assert.match(h.text(), /TLS 证书 Subject · one.test · 目标集团独立单位/);
  assert.match(h.text(), /备案单位 · tenant.shared.test/); assert.match(h.text(), /资产 #31/);
  assert.match(h.text(), /AI 推断（不作核验依据）/);
});
test('chat never preselects or confirms legacy AI-only families, even if a stale selection is retained', async t => {
  const current = view(); delete current.proposal.groups[0].verification; delete current.proposal.groups[1].verification;
  let writes = 0;
  const h = await mount(t, { discoveryStatus: async () => current, confirmDiscoveryScope: async () => { writes++; } });
  assert.deepEqual(h.state().selected.value, []); assert.deepEqual(h.state().hosts.value, []);
  assert.equal(h.matching('data-testid', 'chat-discovery-confirm').length, 0);
  assert.match(h.text(), /2 个域名家族缺少逐域绑定依据/); assert.match(h.text(), /无需逐项确认/);
  h.state().selected.value = ['one.test', 'shared.test']; await h.state().confirm(true); assert.equal(writes, 0);
});
test('chat offers one consent only for bound groups and summarizes all blocked families without checkboxes', async t => {
  const current = view(); current.proposal.groups.push({ root: 'grcbank.com', scopes: ['grcbank.com'], evidenceGroups: 2, reasons: ['含10086.cn'] });
  current.proposal.heldForVerification = 105; current.proposal.removedUnverified = 157;
  const h = await mount(t, { discoveryStatus: async () => current });
  assert.deepEqual(h.state().selected.value, ['one.test', 'shared.test']);
  assert.equal(h.text().includes('grcbank.com'), false); assert.equal(h.text().includes('含10086.cn'), false);
  assert.match(h.text(), /105 个域名家族/); assert.match(h.text(), /拦截 157 项/);
  await h.click('data-testid', 'chat-discovery-confirm');
  assert.deepEqual(h.clientCalls.find(c => c[0] === 'confirm')[3].hosts, ['one.test', 'tenant.shared.test']);
});
test('chat retains local-only verification disclosure when all families are held and the owner is paused', async t => {
  const current = view(); current.phase = 'paused'; current.enabled = false; current.proposal.groups = []; current.proposal.heldForVerification = 109;
  const h = await mount(t, { discoveryStatus: async () => current });
  assert.match(h.text(), /109 个域名家族缺少逐域绑定依据/); assert.match(h.text(), /本次仅核验本地证据，未联网查证企业归属/);
  assert.equal(h.matching('data-testid', 'chat-discovery-confirm').length, 0);
});
test('one consent suppresses double clicks; a saved scope with failed launch is not described as rollback', async t => {
  const gate = deferred(); let writes = 0;
  const h = await mount(t, { confirmDiscoveryScope: () => { writes++; return gate.promise; } });
  const pending = h.state().confirm(true); await h.state().confirm(true); assert.equal(writes, 1);
  gate.resolve({ scopeSaved: true, enrichmentStarted: false, enrichmentError: 'queue unavailable' }); await pending;
  assert.match(h.text(), /范围已保存/); assert.match(h.text(), /无需重复确认范围/); assert.match(h.text(), /queue unavailable/);
});
test('project switch discards late responses and never keeps an old proposal or consent', async t => {
  const gate = deferred(); const h = await mount(t, { discoveryStatus: (_c, p) => p === 1 ? gate.promise : Promise.resolve({ ...view('new'), runId: 'project-two' }) });
  h.props.projectId = 2; h.props.conversationId = 'c2'; await flush(); gate.resolve(view('old')); await flush();
  assert.equal(h.state().data.value.runId, 'project-two'); assert.equal(h.state().proposal.value.token, 'new');
  assert.equal(h.state().busy.value, false);
});
test('revised evidence preserves unchecked families and a rejected consent stays visibly failed after refresh', async t => {
  let current = view();
  const h = await mount(t, { discoveryStatus: async () => current, confirmDiscoveryScope: async () => { current = view('third'); throw Error('evidence changed'); } });
  h.state().selected.value = ['one.test']; current = view('second'); await h.state().load();
  assert.deepEqual(h.state().selected.value, ['one.test']); await h.state().confirm(true);
  assert.match(h.text(), /evidence changed/); assert.deepEqual(h.state().selected.value, ['one.test']);
});

function pollingHost() {
  const timers = new Map(), listeners = new Map(); let id = 0;
  const document = { hidden: false, addEventListener: (name, callback) => listeners.set(name, callback), removeEventListener: name => listeners.delete(name) };
  const window = { setTimeout: (callback, delay) => { const key = ++id; timers.set(key, { callback, delay }); return key; }, clearTimeout: key => timers.delete(key), addEventListener() {}, removeEventListener() {}, getSelection: () => null, setInterval, clearInterval };
  return { host: { document, window }, timers, listeners,
    async tick() { const [key, value] = timers.entries().next().value; timers.delete(key); value.callback(); await flush(); },
    async visibility(hidden) { document.hidden = hidden; listeners.get('visibilitychange')?.(); await flush(); } };
}

test('free evidence continues refreshing independently of a blocked AI and stops while hidden or unmounted', async t => {
  const clock = pollingHost(); let reads = 0; const current = view();
  current.phase = 'blocked'; current.enabled = false; current.enrichment.run.status = 'running';
  const h = await mount(t, { discoveryStatus: async () => { reads++; return structuredClone(current); } }, clock);
  assert.equal(clock.timers.size, 1); current.enrichment.run.counts.done = 19;
  await clock.tick(); assert.equal(reads, 2); assert.match(h.text(), /补证完成 19/);
  await clock.visibility(true); assert.equal(clock.timers.size, 0);
  await clock.visibility(false); assert.equal(reads, 3); assert.equal(clock.timers.size, 1);
  h.unmount(); assert.equal(clock.timers.size, 0); assert.equal(clock.listeners.size, 0);
});

test('an initial transient failure has compact visible feedback and retries without a model or write', async t => {
  const clock = pollingHost(); let reads = 0, mutations = 0;
  const h = await mount(t, { discoveryStatus: async () => { if (++reads === 1) throw Error('agent_host_http_503'); return view(); }, discoveryControl: async () => { mutations++; } }, clock);
  assert.match(h.text(), /正在自动重试/); assert.match(h.text(), /agent_host_http_503/);
  assert.equal(h.matching('data-testid', 'chat-discovery').length, 0);
  assert.equal(clock.timers.size, 1); await clock.tick();
  assert.equal(reads, 2); assert.equal(mutations, 0); assert.equal(h.matching('data-testid', 'chat-discovery').length, 1);
  assert.equal(h.text().includes('agent_host_http_503'), false);
});

test('missing permission is a compact explanation with no empty polling; manual retry uses the actual control', async t => {
  const clock = pollingHost(); let permitted = false, reads = 0;
  const h = await mount(t, { discoveryStatus: async () => { reads++; if (!permitted) throw Error('host_tool_permission_denied'); return view(); } }, clock);
  assert.equal(h.matching('data-testid', 'chat-discovery').length, 0);
  assert.match(h.text(), /资产读取权限或更新内置组件/); assert.equal(clock.timers.size, 0);
  permitted = true; await h.click('data-testid', 'chat-discovery-retry');
  assert.equal(reads, 2); assert.equal(h.matching('data-testid', 'chat-discovery').length, 1);
});

test('scope confirmation forces a new snapshot and late pre-write reads cannot restore old candidates', async t => {
  const clock = pollingHost(), oldRead = deferred(); let reads = 0;
  const old = view(); old.enabled = false; old.enrichment.run.status = 'completed';
  const fresh = { ...view(), phase: 'ready', enabled: false, proposal: null, enrichment: { run: { status: 'running', counts: { done: 19 } } } };
  const h = await mount(t, { discoveryStatus: async () => ++reads === 1 ? old : reads === 2 ? oldRead.promise : fresh }, clock);
  h.state().selected.value = ['one.test'];
  const pending = h.state().load(); await h.state().confirm(true);
  assert.equal(reads, 3); assert.equal(h.state().proposal.value, null);
  oldRead.resolve(old); await pending; await flush();
  assert.equal(h.state().proposal.value, null); assert.match(h.text(), /范围已保存/); assert.match(h.text(), /补证完成 19/);
  assert.equal(clock.timers.size, 1);
});

test('revoked access hides cached candidates and restoring access preserves explicitly unchecked families', async t => {
  const clock = pollingHost(); let current = view(), fail = false;
  const h = await mount(t, { discoveryStatus: async () => { if (fail) throw Error('host_tool_authority_changed'); return current; } }, clock);
  h.state().selected.value = ['one.test']; fail = true; await h.state().load(true);
  assert.equal(h.matching('data-testid', 'chat-discovery').length, 0); assert.equal(clock.timers.size, 0);
  fail = false; current = view('new-evidence'); await h.click('data-testid', 'chat-discovery-retry');
  assert.deepEqual(h.state().selected.value, ['one.test']);
});

test('an empty project gives collection guidance without starting AI or scheduling idle polling', async t => {
  const clock = pollingHost(); let writes = 0;
  const h = await mount(t, { discoveryStatus: async () => ({ ...view(), runId: null, phase: 'empty', enabled: false, proposal: null, review: { run: null }, enrichment: null }), discoveryControl: async () => { writes++; } }, clock);
  assert.match(h.text(), /系统会先采集 FOFA，再自动接续/); assert.equal(clock.timers.size, 0); assert.equal(writes, 0);
  assert.equal(h.matching('data-testid', 'chat-discovery-confirm').length, 0);
});

test('native reconnects can recover automatically but a persistently missing old executor stops idle retries', async t => {
  const clock = pollingHost(); let reads = 0;
  const h = await mount(t, { discoveryStatus: async () => { reads++; throw Error('host_discovery_executor_unavailable'); } }, clock);
  assert.equal(clock.timers.size, 1); await clock.tick(); assert.equal(reads, 2); assert.equal(clock.timers.size, 1);
  await clock.tick(); assert.equal(reads, 3); assert.equal(clock.timers.size, 0);
  assert.match(h.text(), /内置组件/);
});

test('an unsupported old bridge is explained once without retry polling or showing the full workbench', async t => {
  const clock = pollingHost();
  const h = await mount(t, { discoveryStatus: async () => { throw Error('host_discovery_task_rejected'); } }, clock);
  assert.equal(clock.timers.size, 0); assert.equal(h.matching('data-testid', 'chat-discovery').length, 0);
  assert.match(h.text(), /内置组件/);
});

test('chat sees the same FOFA owner before review creation and resumes only its saved Hunter consent',async t=>{
 const current={...view(),phase:'paused',enabled:false,runId:null,review:{run:null},proposal:null,enrichment:null,pipeline:{phase:'paused',round:1,collectionRunId:44,allowHunter:true,counts:{collectedAssets:24},hunter:{queries:0,pages:0,spentPoints:0,costUnknown:false},reason:'首采暂停'}};
 const h=await mount(t,{discoveryStatus:async()=>current});assert.equal(h.matching('data-testid','chat-discovery').length,1);assert.match(h.text(),/FOFA 任务 #44/);await h.state().control('start');
 const c=h.clientCalls.find(c=>c[0]==='control');assert.equal(c[3].allowHunter,true);assert.equal(c[3].policyRevision,3);assert.equal('collectFirst' in c[3],false);assert.equal(h.clientCalls.filter(c=>c[0]==='control').length,1);
});
test('chat displays actual Hunter points and unresolved charge status without the legacy zero-cost claim',async t=>{
 const current={...view(),pipeline:{phase:'blocked',round:3,allowHunter:true,counts:{hunterAdded:8},hunter:{queries:4,pages:11,spentPoints:53,costUnknown:true},reason:'请求结果未知'}};
 const h=await mount(t,{discoveryStatus:async()=>current});assert.match(h.text(),/已知消耗 53 积分/);assert.match(h.text(),/部分扣费或请求结果未知，已暂停/);assert.equal(h.text().includes('Hunter 0 积分'),false);await h.state().control('start');assert.equal(h.clientCalls.some(c=>c[0]==='control'),false);
});
test('chat offers recovery for a blocked first collection instead of showing only Pause',async t=>{
 const current={...view(),phase:'collecting',enabled:true,runId:null,review:{run:null},proposal:null,enrichment:null,pipeline:{phase:'blocked',round:1,collectionRunId:44,allowHunter:false,counts:{},hunter:{queries:0,pages:0,spentPoints:0,costUnknown:false},reason:'首采可恢复'}};
 const h=await mount(t,{discoveryStatus:async()=>current});assert.match(h.text(),/需要处理错误/);assert.match(h.text(),/继续 AI 发现/);assert.equal(h.state().pipelineStopped.value,true);
});

test('storage backoff in chat retains the original queue and refresh never dispatches a replacement owner',async t=>{
 const current={...view(),phase:'ready',enabled:true,proposal:null,pipeline:{phase:'waiting_storage',round:2,collectionRunId:44,enrichmentRunId:'same-free-queue',allowHunter:false,counts:{freeCompleted:8},hunter:{queries:0,pages:0,spentPoints:0,costUnknown:false},reason:'本地存储繁忙'},enrichment:{run:{id:'same-free-queue',status:'waiting_storage',enabled:true,counts:{done:8,pending:24}}}};
 const h=await mount(t,{discoveryStatus:async()=>current});assert.match(h.text(),/等待本地存储，自动重试/);assert.equal(h.state('src/features/assets/components/DiscoveryPipelineProgress.vue').phaseIndex.value,-1);
 await h.state().load(true);await h.state().load(true);assert.equal(h.state().data.value.pipeline.enrichmentRunId,'same-free-queue');assert.equal(h.clientCalls.some(c=>c[0]==='control'||c[0]==='confirm'),false);assert.equal(h.text().includes('本轮回流已保存'),false);
});

test('a free-only storage wait keeps polling while the model owner is blocked and cannot dispatch another Start',async t=>{
 const clock=pollingHost();const current={...view(),phase:'blocked',enabled:false,proposal:null,enrichment:{run:{id:'saved-free-only',status:'waiting_storage',enabled:true,counts:{done:8,pending:24}}}};
 const h=await mount(t,{discoveryStatus:async()=>current},clock);assert.equal(h.state().freeWorking.value,true);assert.equal(clock.timers.size,1);assert.equal(h.text().includes('继续 AI 发现'),false);await clock.tick();assert.equal(clock.timers.size,1);assert.equal(h.clientCalls.some(c=>c[0]==='control'||c[0]==='confirm'),false);
});

test('unknown Hunter charges offer one explicit free continuation with the current witness and never replay the paid query', async t => {
  const current = { ...view(), enabled: false, phase: 'blocked', proposal: null,
    pipeline: { phase: 'blocked', round: 3, allowHunter: true, counts: {}, hunter: { queries: 4, pages: 11, spentPoints: 53, costUnknown: true }, reason: '等待原回执' } };
  const h = await mount(t, { discoveryStatus: async () => structuredClone(current) });
  assert.equal(h.state().mainAction.value, 'free');
  await h.state().control('start'); assert.equal(h.clientCalls.some(c => c[0] === 'control'), false);
  await h.click('data-testid', 'chat-discovery-free');
  const calls = h.clientCalls.filter(c => c[0] === 'control'); assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].slice(1, 3), ['c1', 1]);
  assert.deepEqual(calls[0][3], { ...current.confirmationWitness, action: 'start', organization: current.organization, includeIsolated: false, allowHunter: false });
  assert.match(h.text(), /已知消耗 53 积分/); assert.match(h.text(), /原查询与费用回执保留/);
  assert.equal(h.text().includes('Hunter 0 积分'), false);
});

test('saved free-only continuation stays usable while an older Hunter charge remains unresolved', async t => {
  const current = { ...view(), enabled: false, phase: 'paused', proposal: null,
    pipeline: { phase: 'paused', round: 3, allowHunter: false, counts: {}, hunter: { queries: 4, pages: 11, spentPoints: 53, costUnknown: true }, reason: '已暂停' } };
  const h = await mount(t, { discoveryStatus: async () => current });
  await h.state().control('start');
  assert.equal(h.clientCalls.filter(c => c[0] === 'control').length, 1);
  assert.equal(h.clientCalls.find(c => c[0] === 'control')[3].allowHunter, false);
  assert.match(h.text(), /当前仅免费发现/); assert.match(h.text(), /另有扣费待核对/);
});

test('blocked paid work can switch to free discovery while an existing free queue is still running', async t => {
  const current = { ...view(), enabled: false, phase: 'blocked', proposal: null,
    pipeline: { phase: 'blocked', round: 3, allowHunter: true, counts: {}, hunter: { queries: 4, pages: 11, spentPoints: 53, costUnknown: true }, reason: '等待原回执' },
    enrichment: { run: { status: 'running', counts: { done: 7 } } } };
  const h = await mount(t, { discoveryStatus: async () => current });
  assert.equal(h.state().mainAction.value, 'free');
  assert.equal(h.matching('data-testid', 'chat-discovery-pause-secondary').length, 1);
  await h.click('data-testid', 'chat-discovery-free');
  assert.equal(h.clientCalls.find(c => c[0] === 'control')[3].allowHunter, false);
  assert.equal(h.clientCalls.filter(c => c[0] === 'control').length, 1);
});

test('501 selected scope entries are visibly rejected before any confirmation request', async t => {
  const current = view(); current.proposal.groups = [{ root: 'shared.test', shared: true, scopes: Array.from({ length: 501 }, (_, n) => `tenant${n}.shared.test`), evidenceGroups: 501, verification: verification('tenant0.shared.test') }];
  const h = await mount(t, { discoveryStatus: async () => current });
  assert.equal(h.matching('data-testid', 'chat-discovery-confirm')[0].props.disabled, true);
  assert.match(h.text(), /超过 500 项/); await h.state().confirm(true);
  assert.equal(h.clientCalls.some(c => c[0] === 'confirm'), false);
  h.state().selected.value = []; await flush(); assert.equal(h.matching('data-testid', 'chat-discovery-confirm')[0].props.disabled, true);
});

test('a failed refresh retains the old result but forbids blind start or consent until a fresh witness arrives', async t => {
  let fail = false, current = view();
  const h = await mount(t, { discoveryStatus: async () => { if (fail) throw Error('agent_host_http_503'); return structuredClone(current); } });
  fail = true; await h.state().load(true);
  assert.equal(h.state().data.value.runId, 'saved-review'); assert.equal(h.state().statusFresh.value, false);
  assert.equal(h.state().mainAction.value, 'refresh'); assert.equal(h.matching('data-testid', 'chat-discovery-confirm').length, 0);
  await h.state().control('start', false); await h.state().confirm(true);
  assert.equal(h.clientCalls.some(c => c[0] === 'control' || c[0] === 'confirm'), false);
  fail = false; current = view('fresh'); current.confirmationWitness.policyRevision = 7;
  await h.click('data-testid', 'chat-discovery-retry');
  assert.equal(h.state().statusFresh.value, true); await h.click('data-testid', 'chat-discovery-confirm');
  assert.equal(h.clientCalls.find(c => c[0] === 'confirm')[3].policyRevision, 7);
});

test('a project-mismatched snapshot or missing witness never exposes a mutable workbench', async t => {
  const clock = pollingHost(); let current = { ...view(), projectId: 2 };
  const h = await mount(t, { discoveryStatus: async () => current }, clock);
  assert.equal(h.matching('data-testid', 'chat-discovery').length, 0); assert.equal(clock.timers.size, 0);
  current = view(); delete current.confirmationWitness;
  await h.click('data-testid', 'chat-discovery-retry');
  assert.equal(h.state().statusFresh.value, false); assert.equal(clock.timers.size, 0);
  await h.state().control('start', false); await h.state().confirm(true);
  assert.equal(h.clientCalls.some(c => c[0] === 'control' || c[0] === 'confirm'), false);
});

test('archived project evidence is readable without write controls or idle polling', async t => {
  const clock = pollingHost(), current = { ...view(), readOnly: true };
  const h = await mount(t, { discoveryStatus: async () => current }, clock);
  assert.match(h.text(), /已归档 · 只读/); assert.match(h.text(), /98,989/);
  assert.equal(h.state().mainAction.value, ''); assert.equal(h.matching('data-testid', 'chat-discovery-confirm').length, 0); assert.equal(clock.timers.size, 0);
  await h.state().control('start', false); await h.state().control('pause'); await h.state().confirm(true);
  assert.equal(h.clientCalls.some(c => c[0] === 'control' || c[0] === 'confirm'), false);
});

test('missing Hunter account offers explicit free work and never presents saved configuration as a working connection', async t => {
  const current = { ...view(), enabled: false, phase: 'blocked', proposal: null, readiness: { hunter: { keyConfigured: false, contractVerified: false } },
    pipeline: { phase: 'blocked', round: 1, allowHunter: true, counts: {}, hunter: { queries: 0, pages: 0, spentPoints: 0, costUnknown: false }, reason: '尚未保存 Key' } };
  const h = await mount(t, { discoveryStatus: async () => current });
  assert.equal(h.state().mainAction.value, 'free'); assert.match(h.text(), /账户尚未准备好/);
  assert.match(h.text(), /已保存账户不代表网络已连通/);
  await h.state().control('start'); assert.equal(h.clientCalls.some(c => c[0] === 'control'), false);
  await h.click('data-testid', 'chat-discovery-free');
  assert.equal(h.clientCalls.find(c => c[0] === 'control')[3].allowHunter, false);
});

test('chat keeps one primary decision and collapses detailed process while pause and errors take priority over old held evidence', async t => {
  const h = await mount(t);
  assert.equal(h.matching('data-testid', 'chat-discovery-details')[0].props.open, false);
  assert.equal(h.nodes().filter(n => n.type === 'button' && String(n.props.class).includes('primary')).length, 1);
  let current = { ...view(), phase: 'paused', enabled: false }; current.proposal.groups = []; current.proposal.heldForVerification = 310;
  const paused = await mount(t, { discoveryStatus: async () => current });
  assert.match(paused.state().summary.value, /已暂停/);
  current = { ...current, phase: 'blocked', reason: '原任务的账户配置需要修正' }; await paused.state().load(true);
  assert.match(paused.state().summary.value, /账户配置需要修正/);
  assert.equal(paused.matching('data-testid', 'chat-discovery-details')[0].props.open, false);
});

test('a retained confirmation handler cannot approve a newer proposal or witness which its button never displayed', async t => {
  let current = view('shown');
  const h = await mount(t, { discoveryStatus: async () => structuredClone(current) });
  const shownClick = h.matching('data-testid', 'chat-discovery-confirm')[0].props.onClick;
  current = view('new-proposal'); current.proposal.roots = ['changed-root.test'];
  current.proposal.groups[0].scopes = ['new.one.test']; current.confirmationWitness.policyRevision = 9;
  await h.state().load(true); await shownClick(); await flush();
  assert.equal(h.clientCalls.some(c => c[0] === 'confirm'), false);
  assert.match(h.text(), /建议、选择或授权状态已更新/);
  const beforeWitnessChangeClick = h.matching('data-testid', 'chat-discovery-confirm')[0].props.onClick;
  current.confirmationWitness.grantRevision = 12;
  await h.state().load(true); await beforeWitnessChangeClick(); await flush();
  assert.equal(h.clientCalls.some(c => c[0] === 'confirm'), false);
  await h.click('data-testid', 'chat-discovery-confirm');
  const sent = h.clientCalls.find(c => c[0] === 'confirm')[3];
  assert.equal(sent.proposalToken, 'new-proposal'); assert.equal(sent.policyRevision, 9); assert.equal(sent.grantRevision, 12);
  assert.deepEqual(sent.expectedRoots, ['changed-root.test']); assert.deepEqual(sent.hosts, ['new.one.test', 'tenant.shared.test']);
});

test('a retained confirmation handler cannot submit a changed selection or another conversation', async t => {
  const h = await mount(t);
  const shownClick = h.matching('data-testid', 'chat-discovery-confirm')[0].props.onClick;
  h.state().selected.value = ['one.test']; await flush(); await shownClick(); await flush();
  assert.equal(h.clientCalls.some(c => c[0] === 'confirm'), false);
  const oldConversationClick = h.matching('data-testid', 'chat-discovery-confirm')[0].props.onClick;
  h.props.projectId = 2; h.props.conversationId = 'c2'; await flush(); await oldConversationClick(); await flush();
  assert.equal(h.clientCalls.some(c => c[0] === 'confirm'), false);
  await h.click('data-testid', 'chat-discovery-confirm');
  const call = h.clientCalls.find(c => c[0] === 'confirm');
  assert.deepEqual(call.slice(1, 3), ['c2', 2]); assert.equal(call[3].userConfirmed, true);
});
