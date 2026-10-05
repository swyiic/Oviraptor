// All failures are inert fixtures; exercise real setup and SFC rendering.
const { assert, test, mount, readingStatus, flush, renderDialog } = require('../agent_dialog_harness.cjs');
const secret = '/private/fixture.sqlite?token=fixture-secret Authorization: Bearer fixture-key';
const cases = [
  ['getAgentDialogSelection', '任务选择读取失败，请重新读取。',
    b => b.loadTaskSelection(), b => b.selectionError],
  ['saveAgentDialogSelection', '任务选择保存失败，请重新读取后再选择。',
    b => b.selectScan('B'), b => b.selectionError],
  ['getAgentDialogTask', '无法读取所选任务，请重新读取任务选择。',
    b => b.selectScan('uncached'), b => b.selectionError],
  ['listSentinelScans', '任务列表读取失败，请重试。',
    b => b.loadScans(), b => b.taskNavigation.listError],
  ['listSentinelScans', '更早任务读取失败，请重试。',
    b => { b.hasMoreScans.value = true; return b.loadMoreScans(); }, b => b.taskNavigation.pageError],
  ['getAgentDialogView', '阅读偏好读取失败，请重新读取。',
    b => b.loadDialogView(), b => b.dialogViewError],
  ['saveAgentDialogView', '阅读偏好保存失败，请重新读取后再操作。',
    b => b.saveDialogView(true), b => b.dialogViewError],
  ['getNativeScanTimelinePage', '历史消息读取失败，请重试。',
    b => b.showEarlierMessages(), b => b.historyError],
];

for (const kind of ['Error', 'string']) for (const [api, message, action, error] of cases) {
  test(`${message} hides ${kind} details without implicit preference replay`, async t => {
    const h = await mount({ getNativeScanStatus: async id => api === 'getNativeScanTimelinePage'
      ? { ...readingStatus(id), timeline: readingStatus(id).timeline.slice(1),
        timelineBeforeSequence: 2, hasEarlierTimeline: true } : readingStatus(id) });
    t.after(h.unmount);
    const timeline = h.b.state.value.timeline;
    const saved = h.b.dialogView.value;
    const cursor = h.b.taskNavigation.olderCursor.value;
    let calls = 0;
    h.api[api] = async () => { calls++; throw kind === 'Error' ? new Error(secret) : secret; };
    await action(h.b); await flush();
    assert.equal(calls, 1);
    assert.equal(error(h.b).value, message);
    const html = await renderDialog(h.b);
    assert.ok(html.includes(message));
    for (const text of ['/private/', 'fixture-secret', 'fixture-key', 'Authorization:']) {
      assert.ok(!html.includes(text));
      assert.ok(!error(h.b).value.includes(text));
    }
    if (api === 'saveAgentDialogSelection' || api === 'saveAgentDialogView') {
      await action(h.b); await flush();
      assert.equal(calls, 1, 'failed preference save stays blocked until explicit reload');
    }
    if (api === 'saveAgentDialogView') assert.equal(h.b.dialogView.value, saved);
    if (api === 'getAgentDialogView') assert.equal(h.b.dialogView.value, undefined);
    if (api === 'getNativeScanTimelinePage') assert.equal(h.b.historyPage.value, undefined);
    if (api !== 'saveAgentDialogSelection') assert.equal(h.b.state.value.timeline, timeline);
    assert.equal(h.b.taskNavigation.olderCursor.value, cursor);
  });
}
