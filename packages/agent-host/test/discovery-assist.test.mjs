import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, realpath, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { CertificateTasks } from '../src/certificate-tasks.mjs';
import { DiscoveryAssist, DISCOVERY_ASSIST_TOOLS, discoveryConversationAction } from '../src/discovery-assist.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';
import { serveHost } from '../src/server.mjs';
import { discoveryAssistIntent, certificateTaskIntent, discoveryEnrichmentIntent } from '../src/quality-prompt.mjs';

async function fixture(t) {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'ovi-ai-channel-'));
  const host = new AgentHost({ dataDir: root, runtimeSelection: 'none' });
  let phase = 'preparing', scopes = ['old.example.test'], groups = [{ root: 'new.example.test', scopes: ['new.example.test'], evidenceGroups: 90, reasons: ['同证书单位证据'], verification:{status:'bound_local_evidence',sources:[{kind:'icp',host:'new.example.test',organization:'明确目标单位'}]} }], verificationConflicts=0, reviewRun={requests:2,inputTokens:100,outputTokens:30}; const calls = [];
  const native = createServer(async (req, res) => {
    let raw = ''; for await (const chunk of req) raw += chunk;
    const body = JSON.parse(raw); calls.push({ route: req.url, auth: req.headers.authorization, ...body });
    if (body.action === 'confirm') { scopes = ['old.example.test', ...body.args.hosts]; phase = 'ready'; }
    if (body.action === 'pause') phase = 'paused';
    const result = body.action === 'prepare_seeds' ? {projectId:91,ready:true,items:[],total:0}
      : body.action === 'seeds' ? {projectId:91,ready:true,items:[{id:17,kind:'domain',value:'historical.example.test',firstSeenRound:null,firstSeenAt:null,historicalBaseline:true}],total:1}
      : body.action === 'seed_detail' ? {projectId:91,ready:true,seed:{id:body.args.seedId,kind:'domain',value:'historical.example.test'},observations:[{round:null,queryId:'original-query',page:2,costUnknown:true,spentPoints:null}],observationsTotal:1}
      : body.action === 'confirm' ? { projectId: 91, roots: scopes, scopeSaved: true, enrichmentStarted: true, hunterStarted: false }
      : { projectId: 91, runId: 'review-real', phase, enabled: phase !== 'paused', organization: '明确目标单位', includeIsolated: false,
        review: { run: reviewRun, items: [] },
        proposal: phase === 'awaiting_scope' ? { token: 'evidence-token', roots: scopes, groups, verificationConflicts } : null };
    res.end(JSON.stringify({ ok: true, result }));
  });
  await new Promise(resolve => native.listen(0, '127.0.0.1', resolve));
  host.apps.discoveryAssist = new DiscoveryAssist({ origin: `http://127.0.0.1:${native.address().port}`, token: 'b'.repeat(64) });
  const sourceDirectory = fileURLToPath(new URL('../../../apps/assets/', import.meta.url));
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const pack = await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory: join(root, 'package') });
  await host.apps.install(pack.directory); await host.apps.setEnabled(spec.manifest.id, true);
  const project = host.store.createProject('Real channel project', { externalId: '91' }), other = host.store.createProject('Other', { externalId: '92' });
  const conversation = host.store.createConversation(project.id, '明确目标分析');
  const grant = host.apps.setGrants(spec.manifest.id, project.id, ['inventory.read', 'inventory.write', 'certificate.collect'], 0, host.runtimeIdentity());
  t.after(async () => { await host.close(); await new Promise(resolve => native.close(resolve)); await rm(root, { recursive: true, force: true }); });
  return { host, project, other, conversation, grant, calls, setPhase: value => { phase = value; }, setGroups: value => { groups = value; }, setVerificationConflicts:value=>{verificationConflicts=value;},setReviewRun:value=>{reviewRun=value;} };
}

test('chat analysis starts the same project owner; proposal is advisory and only a human HTTP action confirms', async t => {
  const f = await fixture(t), { host, project, conversation, grant, calls } = f, id = 'org.oviraptor.assets';
  const start = await host.apps.call(id, 'discovery_assist_start', project.id, { organization: '明确目标单位', includeIsolated: false }, grant.revision, 'analysis-once', undefined, host.runtimeIdentity());
  assert.equal(calls[0].route, '/discovery-assist'); assert.equal(calls[0].projectId, 91); assert.equal(calls[0].auth, 'Bearer ' + 'b'.repeat(64));
  assert.equal(start.structuredContent.projectId, project.id);
  assert.equal(DISCOVERY_ASSIST_TOOLS.has('discovery_scope_confirm'), false);
  await assert.rejects(host.apps.call(id, 'discovery_scope_confirm', project.id, { userConfirmed: true }, grant.revision));
  f.setPhase('awaiting_scope');
  const proposed = await host.apps.call(id, 'discovery_scope_propose', project.id, {}, grant.revision);
  assert.equal(proposed.structuredContent.proposal.userConfirmationRequired, true);
  const before = calls.length;
  const view = await discoveryConversationAction(host, conversation.id, project.id);
  assert.equal(view.proposal.groups[0].scopes[0], 'new.example.test');
  assert.equal(calls.slice(before).some(c => c.action === 'confirm'), false);
  const input = { ...view.confirmationWitness, action: 'confirm', runId: view.runId, proposalToken: view.proposal.token,
    expectedRoots: view.proposal.roots, hosts: ['new.example.test'], userConfirmed: true };
  await assert.rejects(discoveryConversationAction(host, conversation.id, project.id, { ...input, userConfirmed: false }));
  await assert.rejects(discoveryConversationAction(host, conversation.id, f.other.id, input));
  const http = await serveHost(host); t.after(() => http.close());
  const response = await fetch(`${http.origin}/api/conversations/${conversation.id}/discovery`, { method: 'POST',
    headers: { 'X-Oviraptor-Session': http.token, 'Content-Type': 'application/json' }, body: JSON.stringify({ ...input, projectId: project.id }) });
  assert.equal(response.status, 200); assert.equal((await response.json()).scopeSaved, true);
  assert.equal(calls.filter(c => c.action === 'confirm').length, 1);
  assert.equal((await discoveryConversationAction(host, conversation.id, project.id)).phase, 'ready');
  assert.equal(calls.some(c => /hunter/i.test(c.route + c.action)), false);
});

test('scope consent is invalidated by runtime/policy changes and cannot cross projects', async t => {
  const f = await fixture(t), { host, project, conversation } = f;
  f.setPhase('awaiting_scope'); const view = await discoveryConversationAction(host, conversation.id, project.id);
  const input = { ...view.confirmationWitness, action: 'confirm', userConfirmed: true, runId: view.runId, proposalToken: view.proposal.token, expectedRoots: view.proposal.roots, hosts: ['new.example.test'] };
  host.setAccessPolicy({ projectId: project.id, mode: 'ask', expectedRevision: 0, expectedRuntimeIdentity: host.runtimeIdentity() });
  await assert.rejects(discoveryConversationAction(host, conversation.id, project.id, input), { code: 'host_discovery_confirmation_stale' });
  assert.equal(f.calls.some(c => c.action === 'confirm'), false);
  await assert.rejects(host.apps.call('org.oviraptor.assets', 'discovery_assist_start', f.other.id, { organization: '明确目标单位' }, undefined), { code: 'host_tool_permission_denied' });
});

test('local seed preparation is human-session-only and requires current witness and inventory write', async t => {
  const f = await fixture(t), { host, project, conversation, calls } = f, appId = 'org.oviraptor.assets';
  const action = { action: 'prepare_seeds' };
  await assert.rejects(discoveryConversationAction(host, conversation.id, project.id, action), { code: 'host_discovery_confirmation_stale' });
  let saved = host.store.grant(appId, project.id);
  host.apps.setGrants(appId, project.id, [], saved.revision, host.runtimeIdentity());
  const ungranted = host.apps.effectiveGrant(host.store.application(appId), project.id, host.runtimeIdentity());
  const ungrantedWitness = { appRevision: host.store.application(appId).revision, grantRevision: ungranted.revision,
    policyRevision: host.accessPolicy.view(project.id).revision, runtimeIdentity: host.runtimeIdentity() };
  await assert.rejects(discoveryConversationAction(host, conversation.id, project.id, { ...ungrantedWitness, ...action }), { code: 'host_tool_permission_denied' });
  saved = host.store.grant(appId, project.id);
  host.apps.setGrants(appId, project.id, ['inventory.read', 'certificate.collect'], saved.revision, host.runtimeIdentity());
  let current = await discoveryConversationAction(host, conversation.id, project.id);
  await assert.rejects(discoveryConversationAction(host, conversation.id, project.id, { ...current.confirmationWitness, ...action }), { code: 'host_tool_permission_denied' });
  saved = host.store.grant(appId, project.id);
  host.apps.setGrants(appId, project.id, ['inventory.read', 'inventory.write'], saved.revision, host.runtimeIdentity());
  await assert.rejects(discoveryConversationAction(host, conversation.id, project.id, { ...current.confirmationWitness, ...action }), { code: 'host_discovery_confirmation_stale' });
  current = await discoveryConversationAction(host, conversation.id, project.id);
  assert.equal([...DISCOVERY_ASSIST_TOOLS.values()].includes('prepare_seeds'), false);
  assert.equal(DISCOVERY_ASSIST_TOOLS.has('human_seed_prepare'), false);
  await assert.rejects(host.apps.call(appId, 'human_seed_prepare', project.id, {}, saved.revision, 'model-cannot-prepare', undefined, host.runtimeIdentity()));
  const web = await serveHost(host); t.after(() => web.close());
  const endpoint = `${web.origin}/api/conversations/${conversation.id}/discovery`;
  const payload = JSON.stringify({ ...current.confirmationWitness, ...action, projectId: project.id });
  const anonymous = await fetch(endpoint, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: payload });
  assert.equal(anonymous.status, 403);
  assert.equal(calls.some(call => call.action === 'prepare_seeds'), false);
  const response = await fetch(endpoint, { method: 'POST', headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' }, body: payload });
  assert.equal(response.status, 200);
  const value = await response.json();
  assert.equal(value.ready, true); assert.equal(value.projectId, project.id); assert.equal(value.desktopProjectId, 91);
  assert.deepEqual(calls.filter(call => call.action === 'prepare_seeds').map(call => ({ route: call.route, args: call.args, projectId: call.projectId })), [{ route: '/discovery-assist', args: {}, projectId: 91 }]);
  assert.equal(calls.some(call => ['start', 'confirm', 'hunter'].includes(call.action)), false);
});

test('local seed preparation rechecks human authority after the single native request', async t => {
  const f = await fixture(t), { host, project, conversation, calls } = f;
  const current = await discoveryConversationAction(host, conversation.id, project.id);
  const prepare = host.apps.discoveryAssist.prepareSeeds.bind(host.apps.discoveryAssist);
  host.apps.discoveryAssist.prepareSeeds = async (...args) => {
    const value = await prepare(...args);
    host.runtimeRevision++;
    return value;
  };
  await assert.rejects(discoveryConversationAction(host, conversation.id, project.id, { ...current.confirmationWitness, action: 'prepare_seeds' }), { code: 'host_tool_authority_changed' });
  assert.equal(calls.filter(call => call.action === 'prepare_seeds').length, 1);
});

test('discovery controls cannot reuse disclosure from an earlier runtime even with current revision witnesses', async t => {
  const f = await fixture(t), { host, project, conversation, grant, calls } = f;
  host.runtimeRevision++;
  const app = host.store.application('org.oviraptor.assets'), identity = host.runtimeIdentity();
  const effective = host.apps.effectiveGrant(app, project.id, identity);
  assert.notEqual(effective.disclosureRuntime, identity);
  const witness = { appRevision: app.revision, grantRevision: effective.revision,
    policyRevision: host.accessPolicy.view(project.id).revision, runtimeIdentity: identity };
  await assert.rejects(discoveryConversationAction(host, conversation.id, project.id), { code: 'host_tool_permission_denied' });
  for (const action of ['start', 'pause', 'prepare_seeds']) {
    await assert.rejects(discoveryConversationAction(host, conversation.id, project.id, {
      ...witness, action, organization: '明确目标单位', includeIsolated: false,
    }), { code: 'host_tool_permission_denied' });
  }
  assert.equal(calls.length, 0, 'no native analysis/control is admitted before renewed disclosure');
  host.apps.setGrants(app.id, project.id, grant.permissions, grant.revision, identity);
  const current = await discoveryConversationAction(host, conversation.id, project.id);
  await discoveryConversationAction(host, conversation.id, project.id, {
    ...current.confirmationWitness, action: 'start', organization: '明确目标单位', includeIsolated: false,
  });
  assert.equal(calls.filter(call => call.action === 'start').length, 1);
});

test('whole-project AI intent cannot be mistaken for scope-only certificate collection', () => {
  const text = '开始 AI 发现，按中国移动从整个项目证书提炼种子';
  assert.equal(discoveryAssistIntent(text).action, 'start');
  assert.equal(certificateTaskIntent(text), null); assert.equal(discoveryEnrichmentIntent(text), null);
  assert.equal(discoveryAssistIntent('查询 AI 发现进度').action, 'status');
  assert.equal(discoveryAssistIntent('不要启动 AI 发现，只读查看进度').action, 'status');
});

test('one legitimate large family confirmation reaches the owner without the former 64KiB HTTP rejection', async t => {
  const f = await fixture(t), { host, project, conversation } = f;
  const hosts = Array.from({ length: 500 }, (_, n) => `n${n}.${'a'.repeat(60)}.${'b'.repeat(60)}.example.test`);
  f.setGroups([{ root: 'example.test', shared: true, scopes: hosts, evidenceGroups: 500, reasons: ['仅具体租户'] }]); f.setPhase('awaiting_scope');
  const view = await discoveryConversationAction(host, conversation.id, project.id), web = await serveHost(host); t.after(() => web.close());
  const input = { ...view.confirmationWitness, action: 'confirm', projectId: project.id, userConfirmed: true, runId: view.runId, proposalToken: view.proposal.token, expectedRoots: view.proposal.roots, hosts };
  assert.ok(Buffer.byteLength(JSON.stringify(input)) > 65536);
  const response = await fetch(`${web.origin}/api/conversations/${conversation.id}/discovery`, { method: 'POST', headers: { 'X-Oviraptor-Session': web.token, 'Content-Type': 'application/json' }, body: JSON.stringify(input) });
  assert.equal(response.status, 200); assert.equal((await response.json()).scopeSaved, true);
  assert.equal(f.calls.filter(c => c.action === 'confirm').at(-1).args.hosts.length, 500);
});

test('slow discovery and private confirmation use a longer single-request window without replaying an unknown control', async t => {
  const calls = [];
  const native = createServer(async (req, res) => {
    let raw = ''; for await (const chunk of req) raw += chunk;
    const input = JSON.parse(raw); calls.push(input);
    await new Promise(resolve => setTimeout(resolve, 80));
    res.end(JSON.stringify({ ok: true, result: { projectId: 91, phase: 'ready', scopeSaved: input.action === 'confirm' } }));
  });
  await new Promise(resolve => native.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => native.close(resolve)));
  const options = { origin: `http://127.0.0.1:${native.address().port}`, token: 'c'.repeat(64) };
  const store = { project: () => ({ externalId: '91' }) };
  assert.equal(new CertificateTasks(options).requestTimeoutMs, 15000, 'ordinary TLS keeps its existing wait window');
  const short = new CertificateTasks({ ...options, tools: DISCOVERY_ASSIST_TOOLS,
    route: '/discovery-assist', kind: 'discovery', requestTimeoutMs: 15 });
  await assert.rejects(short.call(store, 1, 'discovery_assist_start', {}, 'short-once'), error =>
    error.code === 'host_discovery_task_outcome_unknown' && error.details.outcomeUnknown === true);
  const longer = new CertificateTasks({ ...options, tools: DISCOVERY_ASSIST_TOOLS,
    route: '/discovery-assist', kind: 'discovery', requestTimeoutMs: 2000 });
  assert.equal((await longer.call(store, 1, 'discovery_assist_start', {}, 'long-once')).structuredContent.phase, 'ready');
  const assist = new DiscoveryAssist(options);
  assert.equal(assist.requestTimeoutMs, 120000);
  assert.equal((await assist.call(store, 1, 'discovery_assist_status', {}, 'status-once')).structuredContent.phase, 'ready');
  const confirmed = await assist.confirm(store, 1, { userConfirmed: true }, 'confirm-once');
  assert.equal(confirmed.scopeSaved, true);
  assert.deepEqual(calls.map(call => call.operationId), ['short-once', 'long-once', 'status-once', 'confirm-once'],
    'each control/read has one HTTP request; timeout never initiates a retry');
});


test('model summaries distinguish bound local sources from old AI advice and never claim an online company lookup', async t => {
  const f = await fixture(t);
  f.setGroups([
    {root:'bank.example',scopes:['bank.example'],reasons:['contains target domain'],evidenceGroups:1},
    {root:'target.example',scopes:['service.target.example'],reasons:['model inference'],evidenceGroups:1,
      verification:{status:'bound_local_evidence',sources:[{kind:'icp',host:'service.target.example',organization:'Target group',assetId:1}]}}
  ]);
  f.setPhase('awaiting_scope');
  const result = await f.host.apps.call('org.oviraptor.assets','discovery_scope_propose',f.project.id,{},f.grant.revision);
  const proposal=result.structuredContent.proposal;
  assert.equal(proposal.familyCount,1);assert.equal(proposal.scopeCount,1);
  assert.equal(proposal.heldForVerification,1);assert.equal(proposal.networkVerified,false);
  assert.equal(proposal.modelAdviceIsNotOwnershipVerification,true);
  assert.deepEqual(proposal.preview.map(v=>v.root),['target.example']);
  assert.equal(proposal.preview[0].verification.sources[0].organization,'Target group');
  assert.equal(proposal.preview[0].reasons,undefined);
  assert.deepEqual(proposal.preview[0].aiReasons,['model inference']);
  assert.equal(f.calls.some(c=>c.action==='confirm'),false);
});


test('unverified banks and raw model classifications never become a fabricated human review queue',async t=>{
 const f=await fixture(t);f.setPhase('awaiting_scope');f.setGroups([{root:'bank.example',scopes:['bank.example'],evidenceGroups:1,reasons:['含目标组织名称，尚未绑定域名证据']}]);f.setVerificationConflicts(0);f.setReviewRun({requests:2,inputTokens:100,outputTokens:30,counts:{retain:222,conflict:9,evidence:11000},queueCounts:{retain:222,conflict:9,evidence:11000}});
 const result=await f.host.apps.call('org.oviraptor.assets','discovery_assist_status',f.project.id,{},f.grant.revision),view=result.structuredContent;
 assert.equal(view.proposal.familyCount,0);assert.equal(view.proposal.scopeCount,0);assert.equal(view.proposal.userConfirmationRequired,false);assert.equal(view.proposal.verificationConflicts,0);
 assert.equal(view.humanReview.scopeFamilyCount,0);assert.equal(view.humanReview.verificationConflicts,0);assert.equal(view.humanReview.totalGroups,0);assert.equal(view.humanReview.required,false);assert.equal(view.humanReview.automaticEvidenceRequiresHumanReview,false);assert.equal(view.humanReview.rawModelClassificationsAreNotHumanReview,true);
 assert.equal(view.review.run.queueCounts.retain,222);assert.equal(view.review.run.queueCounts.conflict,9);assert.equal(view.proposal.preview.length,0);assert.equal(f.calls.some(call=>call.action==='confirm'||call.action==='start'),false);
});
test('actual domain verification conflicts are counted separately from scope confirmation and model conflicts',async t=>{
 const f=await fixture(t);f.setPhase('awaiting_scope');f.setGroups([]);f.setVerificationConflicts(2);f.setReviewRun({requests:0,inputTokens:0,outputTokens:0,queueCounts:{retain:222,conflict:9}});
 const view=(await f.host.apps.call('org.oviraptor.assets','discovery_scope_propose',f.project.id,{},f.grant.revision)).structuredContent;
 assert.equal(view.proposal.userConfirmationRequired,false);assert.equal(view.humanReview.scopeFamilyCount,0);assert.equal(view.humanReview.verificationConflicts,2);assert.equal(view.humanReview.totalGroups,2);assert.equal(view.humanReview.required,true);
});
test('an older response without actual verification counts does not invent a zero or raw-conflict workload',async t=>{
 const f=await fixture(t);f.setPhase('awaiting_scope');f.setGroups([]);f.setVerificationConflicts(undefined);f.setReviewRun({requests:0,inputTokens:0,outputTokens:0,queueCounts:{conflict:9}});
 const view=(await f.host.apps.call('org.oviraptor.assets','discovery_assist_status',f.project.id,{},f.grant.revision)).structuredContent;
 assert.equal(view.proposal.userConfirmationRequired,false);assert.equal(view.humanReview.verificationConflicts,null);assert.equal(view.humanReview.totalGroups,null);assert.equal(view.humanReview.required,null);
});

test('project seed reads preserve historical unknowns and original costs with read-only current grants',async t=>{
 const f=await fixture(t),app='org.oviraptor.assets';
 const grant=f.host.apps.setGrants(app,f.project.id,['inventory.read'],f.grant.revision,f.host.runtimeIdentity());
 const list=(await f.host.apps.call(app,'discovery_seed_list',f.project.id,{page:1,pageSize:50},grant.revision)).structuredContent;
 assert.equal(list.items[0].firstSeenRound,null);assert.equal(list.items[0].historicalBaseline,true);assert.equal(list.humanReview,undefined);
 const detail=(await f.host.apps.call(app,'discovery_seed_detail',f.project.id,{seedId:17,page:2,pageSize:20},grant.revision)).structuredContent;
 assert.equal(detail.observations[0].spentPoints,null);assert.equal(detail.observations[0].costUnknown,true);assert.equal(detail.observations[0].queryId,'original-query');
 assert.deepEqual(f.calls.map(c=>c.action),['seeds','seed_detail']);assert.equal(f.calls.every(c=>c.projectId===91),true);
 const before=f.calls.length;
 await assert.rejects(f.host.apps.call(app,'discovery_seed_list',f.project.id,{projectId:f.other.id},grant.revision));
 await assert.rejects(f.host.apps.call(app,'discovery_seed_detail',f.project.id,{seedId:-1},grant.revision));
 await assert.rejects(f.host.apps.call(app,'discovery_seed_detail',f.other.id,{seedId:17}));
 assert.equal(f.calls.length,before,'invalid or cross-project seed reads cannot reach the native executor');
});
