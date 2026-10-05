const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, scan, flush, presentation } = require('./sentinel_board/harness.cjs');
const { trace } = require('./agent_trace_harness.cjs');

test('actual task cards and delete dialog explain retention without promising a force delete', async () => {
  const m = await mount();
  try {
    for (const status of ['completed', 'scanning', 'pausing']) {
      const target = scan('delete-a', status);
      m.b.scans.value = [target];
      m.b.tab.value = 'overview';
      m.b.askRemove(target);
      const html = await m.render();
      assert.match(html, /sentinel-task-card/);
      assert.doesNotMatch(html, /强制停止并删除|强制删除|Force delete|会先停止当前进程/);
      assert.match(html, status === 'completed' ? /历史源文件与任务产物文件保留/ : /后端将拒绝删除/);
    }
    assert.equal(m.calls.filter(([name]) => name === 'deleteSentinelScan').length, 0);
  } finally { m.unmount(); }
});

const detailApi = overrides => ({
  listSentinelCheckpoints: async () => [], listSentinelFindings: async id => [{ id: 1, scanId: id, kind: 'source_finding', recordJson: '{}' }],
  listSentinelValidations: async () => [], listInvestigationValidations: async () => [],
  listSentinelScanAttempts: async id => [{ scanId: id, attemptNumber: 1 }],
  listAppSecScanResult: async id => ({ vulnerabilities: [{ scanId: id }], sources: [] }),
  getAgentTrace: async id => trace(id), ...overrides,
});

test('source board rejects late task details instead of relabeling them as the current task', async () => {
  let resolveA;
  const pendingA = new Promise(resolve => { resolveA = resolve; });
  const m = await mount(undefined, detailApi({ listSentinelFindings: async id => id === 'A' ? pendingA : [{ scanId: id, kind: 'sast' }] }));
  try {
    const first = m.b.openScan(scan('A'), false); await flush();
    await m.b.openScan(scan('B'), false);
    resolveA([{ scanId: 'A', kind: 'sast' }]); await first;
    assert.equal(m.b.selected.value.id, 'B');
    assert.equal(m.b.findings.value[0].scanId, 'B');
    assert.equal(m.b.scanAttempts.value[0].scanId, 'B');
    assert.equal(m.b.appsecResult.value.vulnerabilities[0].scanId, 'B');
    assert.equal(m.b.liveTrace.value.summary.scanId, 'B');
    assert.equal(m.b.detailBusy.value, false);
  } finally { resolveA([]); m.unmount(); }
});

test('source board clears prior task evidence and ignores obsolete errors while the new task loads', async () => {
  let rejectA, resolveB;
  const pendingA = new Promise((_, reject) => { rejectA = reject; });
  const pendingB = new Promise(resolve => { resolveB = resolve; });
  const m = await mount(undefined, detailApi({ listSentinelFindings: async id => id === 'A' ? pendingA : pendingB }));
  try {
    m.b.findings.value = [{ scanId: 'previous' }];
    m.b.appsecResult.value = { vulnerabilities: [{ scanId: 'previous' }], sources: [] };
    const first = m.b.openScan(scan('A'), false); await flush();
    const second = m.b.openScan(scan('B'), false); await flush();
    assert.deepEqual(m.b.findings.value, []);
    assert.deepEqual(m.b.appsecResult.value.vulnerabilities, []);
    rejectA(new Error('obsolete private error')); await first;
    assert.equal(m.b.detailBusy.value, true, 'old finally cannot stop the new loading state');
    assert.ok(!m.events.some(event => String(event).includes('obsolete private error')));
    resolveB([{ scanId: 'B', kind: 'sast' }]); await second;
    assert.equal(m.b.findings.value[0].scanId, 'B');
  } finally { rejectA(new Error('cleanup')); resolveB([]); m.unmount(); }
});

test('reopening the same source task does not accept an older detail generation', async () => {
  let resolveOld, reads = 0;
  const pending = new Promise(resolve => { resolveOld = resolve; });
  const m = await mount(undefined, detailApi({ listSentinelFindings: async id => ++reads === 1 ? pending : [{ scanId: id, kind: 'sast', title: 'new' }] }));
  try {
    const old = m.b.openScan(scan('A'), false); await flush();
    await m.b.openScan(scan('A'), false);
    resolveOld([{ scanId: 'A', kind: 'sast', title: 'old' }]); await old;
    assert.equal(m.b.findings.value[0].title, 'new');
    assert.equal(m.b.detailBusy.value, false);
  } finally { resolveOld([]); m.unmount(); }
});

test('background detail refresh cannot overwrite a subsequent source task selection', async () => {
  let resolveOld, delayed = false;
  const pending = new Promise(resolve => { resolveOld = resolve; });
  const m = await mount(undefined, detailApi({ listSentinelFindings: async id => id === 'A' && delayed ? pending : [{ scanId: id, kind: 'sast' }] }));
  try {
    await m.b.openScan(scan('A'), false);
    m.props.active = true; await flush(); delayed = true;
    const refresh = m.b.liveSync(); await flush();
    await m.b.openScan(scan('B'), false);
    resolveOld([{ scanId: 'A', kind: 'sast' }]); await refresh;
    assert.equal(m.b.findings.value[0].scanId, 'B');
    assert.equal(m.b.scanAttempts.value[0].scanId, 'B');
    assert.equal(m.b.appsecResult.value.vulnerabilities[0].scanId, 'B');
    assert.equal(m.b.liveTrace.value.summary.scanId, 'B');
  } finally { resolveOld([]); m.unmount(); }
});

test('source details publish together with AppSec and do not reappear after unmount', async () => {
  let resolveAppSec;
  const pending = new Promise(resolve => { resolveAppSec = resolve; });
  const m = await mount(undefined, detailApi({ listAppSecScanResult: async () => pending }));
  const operation = m.b.openScan(scan('A'), false); await flush();
  assert.deepEqual(m.b.findings.value, [], 'do not publish half of a detail snapshot');
  assert.deepEqual(m.b.scanAttempts.value, []);
  assert.equal(m.b.detailBusy.value, true);
  m.unmount();
  resolveAppSec({ vulnerabilities: [{ scanId: 'A' }], sources: [] }); await operation;
  assert.deepEqual(m.b.findings.value, []);
  assert.deepEqual(m.b.scanAttempts.value, []);
  assert.equal(m.b.liveTrace.value, undefined);
});

test('late traces cannot reenter after leaving and reopening the same source task', async () => {
  let resolveOld, reads = 0;
  const pending = new Promise(resolve => { resolveOld = resolve; });
  const m = await mount(undefined, detailApi({ getAgentTrace: async id => id === 'A' && ++reads === 1
    ? pending : trace(id, 'new') }));
  try {
    const first = m.b.openScan(scan('A'), false); await flush();
    await m.b.openScan(scan('B'), false);
    await m.b.openScan(scan('A'), false);
    resolveOld(trace('A', 'old')); await first;
    assert.equal(m.b.liveTrace.value.summary.model, 'new');
    assert.equal(m.b.liveTraceBusy.value, false);
  } finally { resolveOld({}); m.unmount(); }
});

test('a backend deletion refusal preserves the selected task and offers no fake success', async () => {
  const m = await mount(async () => { throw new Error('仍有未结算的执行'); });
  try {
    const target = scan();
    m.b.selected.value = target;
    m.b.previewScan.value = target;
    m.b.findings.value = [{ id: 1 }];
    m.b.askRemove(target);
    const before = m.calls.length;
    await m.b.remove();
    assert.equal(m.b.selected.value.id, target.id);
    assert.equal(m.b.previewScan.value.id, target.id);
    assert.equal(m.b.pendingDelete.value.id, target.id);
    assert.equal(m.b.findings.value.length, 1);
    assert.equal(m.b.deleting.value, false);
    assert.deepEqual(m.calls.slice(before), [['deleteSentinelScan', target.id]]);
    assert.deepEqual(m.events.filter(([kind]) => kind === 'success'), []);
    assert.match(m.events.at(-1)[1], /删除失败.*未结算/);
  } finally { m.unmount(); }
});

test('administratively closed task cards retain history and hide retry and deletion actions', async (t) => {
  const m = await mount();
  t.after(m.unmount);
  const target = { ...scan('sealed-task', 'cancelled'), administrativeClosureRecorded: true,
    latestAttemptNumber: 1, latestAttemptStatus: 'paused' };
  m.b.scans.value = [target];
  m.b.tab.value = 'overview';
  const html = await m.render();
  assert.match(html, /已人工结案/);
  assert.match(html, /原执行结果未结清/);
  assert.doesNotMatch(html, /重试未完成阶段|button danger compact/);
  assert.match(presentation.scanInterruption(target).action, /新建独立任务/);
});

test('in-flight delete cannot be duplicated, dismissed or retargeted; success states files retained', async () => {
  let resolve;
  const waiting = new Promise((done) => { resolve = done; });
  const m = await mount(() => waiting);
  try {
    const target = scan();
    m.b.selected.value = target;
    m.b.previewScan.value = target;
    m.b.askRemove(target);
    const operation = m.b.remove();
    assert.equal(m.b.deleting.value, true);
    m.b.cancelRemove();
    m.b.askRemove(scan('delete-b'));
    await m.b.remove();
    assert.equal(m.b.pendingDelete.value.id, target.id);
    assert.deepEqual(m.calls.filter(([name]) => name === 'deleteSentinelScan'), [['deleteSentinelScan', target.id]]);
    resolve();
    await operation;
    assert.equal(m.b.selected.value, undefined);
    assert.equal(m.b.previewScan.value, undefined);
    assert.equal(m.b.pendingDelete.value, undefined);
    assert.equal(m.b.deleting.value, false);
    assert.match(m.events.find(([kind]) => kind === 'success')[1], /历史源文件与任务产物文件已保留/);
  } finally { resolve(); m.unmount(); }
});
