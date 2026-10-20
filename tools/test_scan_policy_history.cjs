const test = require('node:test');
const assert = require('node:assert/strict');
const { mount, scan, presentation } = require('./sentinel_board/harness.cjs');
const code = 'scan_agent_policy_history_unknown_requires_fresh_attempt';

test('unknown historical policy has a readable explanation in errors and retained checkpoints', () => {
  for (const error of [code, new Error(code), `web start rejected: ${code}`]) {
    const message = presentation.scanOperationError(error);
    assert.match(message, /没有可验证的 Nest 策略快照/);
    assert.match(message, /新建独立任务/);
    assert.match(message, /不会自动重放/);
    assert.doesNotMatch(message, /scan_agent_policy_/);
  }
  assert.match(presentation.humanizeScanCheckpoint(code), /没有可验证的 Nest 策略快照/);
  const interruption = presentation.scanInterruption({ ...scan('legacy-policy', 'paused'), currentCheckpoint: code });
  assert.equal(interruption.title, '历史策略快照缺失');
  assert.match(interruption.action, /新建独立任务/);
  assert.doesNotMatch(interruption.action, /只处理尚未完成/);
  assert.equal(presentation.scanOperationError('unrelated failure'), 'unrelated failure');
  assert.equal(presentation.scanOperationError(`${code}_different`), `${code}_different`);
});

for (const action of ['confirm', 'resumeScan', 'rescan', 'continueAfterAuthRecovery']) {
  test(`actual board ${action} explains unknown policy without replay, fallback, or success`, async () => {
    const calls = [];
    const reject = async id => { calls.push(id); throw new Error(code); };
    const task = { ...scan('historic-policy', 'paused'), scanType: 'web', attemptCount: 1 };
    const h = await mount(undefined, {
      confirmSentinelScan: reject, resumeSentinelScan: reject, rescanSentinelScan: reject,
      listSentinelScanAuthSessions: async () => [],
    });
    try {
      h.b.scans.value = [task];
      h.b.selected.value = task;
      if (action === 'continueAfterAuthRecovery') h.b.authRecoveryScan.value = task;
      await h.b[action](task);
      assert.deepEqual(calls, [task.id]);
      const failures = h.events.filter(([kind]) => kind === 'error');
      assert.equal(failures.length, 1);
      assert.match(failures[0][1], /新建独立任务/);
      assert.equal(h.events.some(([kind]) => kind === 'success'), false);
      assert.equal(h.b.scanControlBusy.value, '');
      assert.equal(h.b.selected.value.id, task.id);
      assert.equal(h.b.selected.value.status, 'paused');
      if (action === 'continueAfterAuthRecovery') {
        assert.equal(h.b.authRecoveryScan.value.id, task.id);
        assert.equal(h.b.authRecoveryAutoContinuing.value, false);
      }
    } finally { h.unmount(); }
  });
}
