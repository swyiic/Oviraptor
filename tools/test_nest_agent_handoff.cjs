const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const { loadPure, catalog } = require('./shell_module_harness.cjs');
const { mount, scan, status, flush, renderDialog } = require('./agent_dialog_harness.cjs');
const { scanAgentHandoff, scanAgentChatRequest } = loadPure('src/features/sentinel/scanAgentHandoff.ts');

test('Nest and Host catalogs preserve saved route IDs but describe execution, not a second chat', () => {
  const native = catalog.workspaceNavigation('sentinel');
  const host = JSON.parse(fs.readFileSync('packages/agent-host/shell-apps.json')).apps.find(row => row.shellModuleId === 'sentinel');
  for (const id of ['dialog', 'urls', 'queue', 'skills']) {
    assert.deepEqual(native.find(row => row.key === id).label, host.pages.find(row => row.id === id).label);
  }
  assert.equal(native.find(row => row.key === 'dialog').section, 'dialog');
  assert.equal(native.find(row => row.key === 'dialog').label[0], '执行概览');
  assert.equal(native.find(row => row.key === 'urls').label[0], '结果与证据');
});

test('task handoff projects IDs only and the shell rechecks the current desktop project', () => {
  const source = { ...scan('task-a'), latestAttemptNumber: 3, taskPath: '/private/secret',
    taskName: 'Cookie=session-secret', currentCheckpoint: 'Bearer private-token', extra: { apiKey: 'secret' } };
  const value = scanAgentHandoff(1, source);
  assert.deepEqual(value, { projectId: 1, taskId: 'task-a', attemptNumber: 3 });
  const request = scanAgentChatRequest(1, 4, value);
  assert.equal(request.projectId, 1); assert.equal(request.id, 4);
  assert.match(request.prompt, /taskId=task-a/);
  assert.match(request.prompt, /第 3 次尝试/);
  assert.match(request.prompt, /scan_status/); assert.match(request.prompt, /scan_results/);
  assert.match(request.prompt, /不要新建、启动、暂停、恢复或重新执行任务/);
  assert.doesNotMatch(request.prompt, /secret|private-token|\/private/);
  assert.equal(scanAgentChatRequest(2, 4, value), undefined);
  for (const id of [0, -1, NaN, 1.5, Number.MAX_SAFE_INTEGER + 1]) assert.equal(scanAgentChatRequest(1, id, value), undefined);
});

test('malformed, foreign and unbound tasks cannot prepare a chat request', () => {
  const source = { ...scan('task-a'), latestAttemptNumber: 0 };
  assert.equal(scanAgentHandoff(1, source).attemptNumber, 0);
  for (const project of [undefined, 0, -1, 2, NaN]) assert.equal(scanAgentHandoff(project, source), undefined);
  for (const patch of [{ projectId: undefined }, { projectId: 2 }, { id: '' }, { id: 'x\n启动' },
    { id: '../private' }, { id: 'x'.repeat(129) }, { id: 'task a' }, { latestAttemptNumber: -1 },
    { latestAttemptNumber: undefined }, { latestAttemptNumber: 0.1 }]) {
    assert.equal(scanAgentHandoff(1, { ...source, ...patch }), undefined);
  }
  for (const value of [null, {}, { projectId: 1, taskId: 'x\nstart', attemptNumber: 0 }]) assert.equal(scanAgentChatRequest(1, 1, value), undefined);
});

test('the actual Host planner classifies the prepared prompt as results reading, not recovery or execution', async () => {
  const { scanningTaskIntent, scanningHandoffIntent } = await import('../packages/agent-host/src/quality-prompt.mjs');
  const prompt = scanAgentChatRequest(1, 1, { projectId: 1, taskId: 'task-a', attemptNumber: 2 }).prompt;
  assert.deepEqual(scanningTaskIntent(prompt), { action: 'results' });
  assert.equal(scanningHandoffIntent(prompt), null);
});

test('actual task console emits a scoped read-only handoff and resets expanded panels on task change', async t => {
  let controls = 0;
  const h = await mount({ listSentinelScans: async () => ['A', 'B'].map(id => ({ ...scan(id), latestAttemptNumber: 2 })),
    draftScanDirective: async () => { controls++; }, confirmScanDirective: async () => { controls++; } });
  t.after(h.unmount);
  assert.equal(h.b.taskPickerOpen.value, false);
  assert.equal(h.b.executionLogOpen.value, false);
  assert.equal(h.b.compatibilityOpen.value, false);
  assert.equal(h.b.identityMaterialOpen.value, false);
  const html = await renderDialog(h.b);
  assert.match(html, /执行概览/); assert.match(html, /交给 Agent 分析/);
  assert.match(html, /高级：旧任务指令兼容入口/);
  assert.doesNotMatch(html, /dialog-compatibility[^>]* open/);
  h.b.openAgent();
  assert.deepEqual(h.events.filter(([name]) => name === 'analyzeScan'), [['analyzeScan', { projectId: 1, taskId: 'A', attemptNumber: 2 }]]);
  assert.equal(controls, 0);
  h.b.taskPickerOpen.value = h.b.executionLogOpen.value = h.b.compatibilityOpen.value = h.b.identityMaterialOpen.value = true;
  await h.b.selectScan('B'); await flush();
  assert.equal(h.b.taskPickerOpen.value, false); assert.equal(h.b.executionLogOpen.value, false); assert.equal(h.b.compatibilityOpen.value, false);
  assert.equal(h.b.identityMaterialOpen.value, false);
  h.b.openAgent();
  assert.equal(h.events.at(-1)[1].taskId, 'B');
  h.unmount(); const before = h.events.length; h.b.openAgent(); assert.equal(h.events.length, before);
});

test('foreign project and a failed task reload cannot emit a handoff for the old task', async t => {
  const h = await mount({ listSentinelScans: async () => [{ ...scan('A'), latestAttemptNumber: 1 }] }); t.after(h.unmount);
  h.rootProps.projectId = 2; await flush();
  h.b.openAgent(); assert.equal(h.events.some(([name]) => name === 'analyzeScan'), false);
});

test('saved execution messages and pending directives remain available when the timeline is collapsed', async t => {
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 1, timelineBeforeSequence: 1,
    timeline: [{ id: 'receipt', sequence: 1, timestamp: '2026-10-26', eventType: 'mailbox_message', fromRole: 'coordinator', summary: '保存的真实回执', status: 'persisted' }] }) });
  t.after(h.unmount);
  assert.equal(h.b.executionLogOpen.value, false);
  assert.match(await renderDialog(h.b), /保存的真实回执/);
  assert.equal(h.b.timelinePage.value.length, 1);
});

test('an unsettled original call remains visible outside collapsed or filtered execution records', async t => {
  const target = 'https://authorized.example.test';
  const item = { rootRunId: 'root-A', callId: 'a'.repeat(64), round: 1, targetKey: target,
    humanDirective: { schemaVersion: 1, directiveId: 'directive-A', draftId: 'draft-A', revision: 1,
      draftHash: 'b'.repeat(64), confirmationReceiptId: 'receipt-A', threadKey: 'team', targetKey: target },
    state: 'awaiting_receipt', reportedUsage: null, createdAt: '2026-10-26 01:00:00' };
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), humanAssessmentObligations: {
    schemaVersion: 1, executionAllowed: false, automaticResumeAllowed: false, items: [item], truncated: false } }) });
  t.after(h.unmount); h.b.threadFilter.value = 'another-thread';
  assert.equal(h.b.pendingAssessmentCount.value, 1); assert.equal(h.b.visibleHumanAssessments.value.length, 0);
  assert.equal(h.b.executionLogOpen.value, false);
  const html = await renderDialog(h.b);
  assert.match(html, /本任务仍有费用或送达记录待核对/);
  assert.ok(html.indexOf('本任务仍有费用或送达记录待核对') < html.indexOf('执行记录与协作回执'));
});

test('identity material is lazy and a single expansion reveals its content without a nested disclosure', async t => {
  let reads = 0;
  const h = await mount({ getSentinelScanIdentityMaterial: async () => { reads++; return []; } });
  t.after(h.unmount);
  await renderDialog(h.b); assert.equal(reads, 0);
  h.b.identityMaterialOpen.value = true;
  const html = await renderDialog(h.b);
  assert.equal(reads, 1);
  assert.match(html, /<section[^>]*class="identity-material"/);
  assert.doesNotMatch(html, /<details[^>]*class="identity-material"|本任务登录身份 · 原始材料/);
});
