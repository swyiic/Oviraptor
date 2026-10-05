// Real AgentDialog setup/render regressions: tasks.
require('./agent_dialog/navigation_privacy.cjs');
const { assert, test, renderDialog, deferred, flush, scan, status, longTimeline,
  directive, mount, readingView, readingStatus, selectionRecord } = require('./agent_dialog_harness.cjs');

test('malformed task selection replies do not restore tasks or trigger preference writes', async (t) => {
  for (const record of [
    { ...selectionRecord(null, 2), selectionUnavailable: false },
    { ...selectionRecord('B'), revision: Number.MAX_SAFE_INTEGER + 1 },
    { ...selectionRecord('B'), selectedScanId: 'foreign' },
    { ...selectionRecord(null), selectedScan: { ...scan(null), projectId: 1 }, selectionUnavailable: false },
    { ...selectionRecord('B'), selectedScan: { id: 'B', projectId: 1 } },
    selectionRecord('B', 1, 2),
  ]) {
    let writes = 0;
    const h = await mount({ listSentinelScans: async () => [scan('A')],
      getAgentDialogSelection: async () => record,
      saveAgentDialogSelection: async () => { writes++; throw new Error('unexpected write'); },
    }, { props: { initialScanId: undefined } });
    t.after(h.unmount);
    assert.equal(h.b.selectionError.value, '任务选择读取失败，请重新读取。');
    assert.equal(h.b.scanId.value, 'A');
    assert.equal(writes, 0);
  }
});

test('saved task beyond 300 rows restores directly without paging writing or resuming', async (t) => {
  const rows = Array.from({ length: 300 }, (_, i) => ({ ...scan(`recent-${i}`), updatedAt: '2026' }));
  const calls = []; let writes = 0;
  const saved = selectionRecord('old', 7); saved.selectedScan.updatedAt = '2000';
  const h = await mount({
    listSentinelScans: async (...args) => { calls.push(args); return rows; },
    getAgentDialogSelection: async () => saved,
    saveAgentDialogSelection: async () => { writes++; throw new Error('unexpected write'); },
    getAgentDialogTask: async () => { throw new Error('saved task already returned'); },
  }, { props: { initialScanId: '' }, expectedScanId: 'old' }); t.after(h.unmount);
  assert.equal(h.b.scans.value.length, 301);
  assert.equal(h.b.taskNavigation.olderCursor.value.id, rows[299].id);
  assert.equal(calls.length, 1); assert.equal(calls[0][2], undefined);
  assert.equal(writes, 0);
  await h.b.loadScans(); await h.b.loadStatus();
  assert.equal(h.b.scanId.value, 'old'); assert.equal(writes, 0);
});

test('failed initial preference read drops queued choices before explicit reload', async (t) => {
  const pending = deferred(); let writes = 0;
  const h = await mount({ getAgentDialogSelection: () => pending.promise,
    saveAgentDialogSelection: async () => { writes++; throw new Error('unexpected write'); },
  }, { props: { initialScanId: undefined }, expectedScanId: '' });
  t.after(h.unmount);
  await h.b.selectScan('B');
  pending.reject(new Error('read-error'));
  await flush();
  assert.equal(h.b.selectionError.value, '任务选择读取失败，请重新读取。');
  await h.b.selectScan('A');
  h.api.getAgentDialogSelection = async () => selectionRecord('B', 9);
  await h.b.loadTaskSelection();
  await flush();
  assert.equal(writes, 0, 'reload must not commit a choice queued before the read failure');
  assert.equal(h.b.scanId.value, 'A');
  assert.equal(h.b.selectionError.value, '');
});

test('explicit task navigation outranks saved task and resolves outside the first page', async (t) => {
  const reads = []; const writes = [];
  const h = await mount({
    getAgentDialogSelection: async () => selectionRecord('B', 4),
    getAgentDialogTask: async (id, projectId) => { reads.push([id, projectId]); return { ...scan(id), projectId }; },
    saveAgentDialogSelection: async (input) => { writes.push(input); return selectionRecord(input.scanId, input.expectedRevision + 1); },
  }, { props: { initialScanId: 'older' }, expectedScanId: 'older' }); t.after(h.unmount);
  assert.deepEqual(reads, [['older', 1]]);
  assert.deepEqual(writes, [{ projectId: 1, expectedRevision: 4, scanId: 'older' }]);
  h.rootProps.initialScanId = 'another-older'; await flush();
  assert.equal(h.b.scanId.value, 'another-older');
  assert.equal(writes[1].expectedRevision, 5);
});

test('manual task choice survives component remount without an automatic write', async (t) => {
  let saved = selectionRecord(null, 0); let writes = 0;
  const api = { getAgentDialogSelection: async () => structuredClone(saved),
    saveAgentDialogSelection: async (input) => { writes++; saved = selectionRecord(input.scanId, input.expectedRevision + 1); return structuredClone(saved); },
  };
  const first = await mount(api); await first.b.selectScan('B'); first.unmount();
  const before = writes;
  const second = await mount(api, { props: { initialScanId: '' }, expectedScanId: 'B' }); t.after(second.unmount);
  assert.equal(writes, before);
  assert.equal(second.b.taskNavigation.taskSelection.value.selectedScanId, 'B');
});

test('rapid task choices serialize and coalesce without rewinding the visible task', async (t) => {
  const h = await mount(); t.after(h.unmount);
  const pending = deferred(); const calls = [];
  h.api.saveAgentDialogSelection = (input) => { calls.push(input); return calls.length === 1
    ? pending.promise : Promise.resolve(selectionRecord(input.scanId, input.expectedRevision + 1)); };
  const old = h.b.selectScan('B');
  await h.b.selectScan('A'); await h.b.selectScan('C');
  assert.equal(calls.length, 1); assert.equal(h.b.scanId.value, 'C');
  pending.resolve(selectionRecord('B', calls[0].expectedRevision + 1)); await old;
  assert.deepEqual(calls.map((item) => item.scanId), ['B', 'C']);
  assert.equal(calls[1].expectedRevision, calls[0].expectedRevision + 1);
  assert.equal(h.b.taskNavigation.taskSelection.value.selectedScanId, 'C');
  assert.equal(h.b.scanId.value, 'C');
});

test('task selection conflict never retries on reload or stops navigation', async (t) => {
  const h = await mount(); t.after(h.unmount);
  let calls = 0;
  h.api.saveAgentDialogSelection = async () => { calls++; throw new Error('dialog_selection_revision_conflict'); };
  await h.b.selectScan('B');
  assert.equal(h.b.scanId.value, 'B');
  assert.equal(h.b.selectionError.value, '任务选择保存失败，请重新读取后再选择。');
  assert.equal(h.b.error.value, '');
  await h.b.selectScan('A');
  h.api.getAgentDialogSelection = async () => selectionRecord('B', 9);
  await h.b.loadTaskSelection(); await flush();
  assert.equal(calls, 1); assert.equal(h.b.scanId.value, 'A');
  assert.equal(h.b.taskNavigation.taskSelection.value.revision, 9);
});

test('deleted saved task and preference failures remain visible on an empty task list', async (t) => {
  for (const failure of [false, true]) {
    const h = await mount({ listSentinelScans: async () => [],
      getAgentDialogSelection: async () => { if (failure) throw new Error('read-error'); return selectionRecord(null, 2); },
    }, { props: { initialScanId: '' }, expectedScanId: '' }); t.after(h.unmount);
    const html = await renderDialog(h.b);
    assert.ok(html.includes(failure ? '任务选择读取失败，请重新读取。' : '上次查看的任务已删除'));
    assert.equal(h.b.state.value, undefined);
  }
});

test('foreign task response is rejected and cannot replace the current task', async (t) => {
  const h = await mount(); t.after(h.unmount);
  h.api.getAgentDialogTask = async (id) => ({ ...scan(id), projectId: 2 });
  await h.b.selectScan('foreign');
  assert.equal(h.b.scanId.value, 'A');
  assert.ok(!h.b.scans.value.some((item) => item.id === 'foreign'));
  assert.equal(h.b.selectionError.value, '无法读取所选任务，请重新读取任务选择。');
});

test('stale task preference reads and writes cannot publish across project round trips or unmount', async (t) => {
  for (const action of ['read', 'write']) for (const failure of [false, true]) {
    const h = await mount(); t.after(h.unmount);
    const pending = deferred();
    const name = action === 'read' ? 'getAgentDialogSelection' : 'saveAgentDialogSelection';
    const original = h.api[name]; h.api[name] = () => pending.promise;
    const old = action === 'read' ? h.b.loadTaskSelection() : h.b.selectScan('B');
    h.api[name] = original;
    h.rootProps.projectId = 2; await flush(); h.rootProps.projectId = 1; await flush();
    const before = JSON.stringify(h.b.taskNavigation.taskSelection.value);
    if (failure) pending.reject(new Error('old-project'));
    else pending.resolve(selectionRecord('B', 99));
    await old;
    assert.equal(JSON.stringify(h.b.taskNavigation.taskSelection.value), before);
    assert.equal(h.b.selectionError.value, '');
    assert.equal(h.b.selectionBusy.value, false);
    const late = deferred(); h.api[name] = () => late.promise;
    const operation = action === 'read' ? h.b.loadTaskSelection() : h.b.selectScan('B');
    h.unmount(); late.reject(new Error('after-unmount')); await operation;
    assert.equal(h.b.selectionError.value, '');
  }
});

const invalidTaskPages = [
  ['foreign project', [scan('foreign', 2)]],
  ['missing project', [{ ...scan('unbound'), projectId: undefined }]],
  ['duplicate identity', [scan('duplicate'), scan('duplicate')]],
  ['missing identity', [{ ...scan('missing'), id: undefined }]],
  ['missing timestamp', [{ ...scan('missing'), updatedAt: undefined }]],
  ['missing status', [{ ...scan('missing'), status: undefined }]],
  ['invalid second row', [scan('partial'), null]],
  ['non-array page', { rows: [scan('wrong-envelope')] }],
  ['oversized page', Array.from({ length: 301 }, (_, i) => scan(`oversized-${i}`))],
];

for (const mode of ['refresh', 'older']) for (const [label, response] of invalidTaskPages) {
  test(`${mode} task page rejects ${label} without changing navigation or its retry cursor`, async (t) => {
    const h = await mount(); t.after(h.unmount);
    if (mode === 'older') h.b.hasMoreScans.value = true;
    const navigation = h.b.taskNavigation;
    const beforeRows = structuredClone(h.b.scans.value.map((row) => ({ ...row })));
    const beforeCursor = { ...navigation.olderCursor.value };
    const beforeHasMore = h.b.hasMoreScans.value;
    const calls = []; let writes = 0;
    h.api.saveAgentDialogSelection = async () => { writes++; throw new Error('unexpected preference write'); };
    h.api.listSentinelScans = async (...args) => { calls.push(args); return response; };
    const load = mode === 'older' ? h.b.loadMoreScans : h.b.loadScans;
    await load();
    const error = mode === 'older' ? navigation.pageError : navigation.listError;
    const message = mode === 'older' ? '更早任务读取失败，请重试。' : '任务列表读取失败，请重试。';
    assert.equal(error.value, message);
    assert.deepEqual(h.b.scans.value, beforeRows);
    assert.deepEqual(navigation.olderCursor.value, beforeCursor);
    assert.equal(h.b.hasMoreScans.value, beforeHasMore);
    assert.equal(h.b.scanId.value, 'A');
    assert.equal(h.b.loadingMoreScans.value, false);
    assert.equal(writes, 0);
    assert.ok((await renderDialog(h.b)).includes(message));
    h.api.listSentinelScans = async (...args) => { calls.push(args); return [scan('recovered')]; };
    await load();
    assert.equal(error.value, '');
    assert.deepEqual(calls[1], calls[0], 'retry must use the same project, page size and cursor');
    assert.ok(h.b.scans.value.some((row) => row.id === 'recovered'));
    assert.equal(h.b.scanId.value, 'A');
    assert.equal(writes, 0);
  });
}

test('an invalid initial task page cannot trigger task status or preference reads and writes', async (t) => {
  const calls = [];
  const h = await mount({
    listSentinelScans: async () => [scan('foreign', 2)],
    getNativeScanStatus: async () => { calls.push('status'); throw new Error('unexpected status read'); },
    getAgentDialogSelection: async () => { calls.push('selection'); return selectionRecord(null, 0); },
    saveAgentDialogSelection: async () => { calls.push('write'); throw new Error('unexpected preference write'); },
  }, { props: { initialScanId: '' }, expectedScanId: '' });
  t.after(h.unmount);
  assert.deepEqual(calls, []);
  assert.deepEqual(h.b.scans.value, []);
  assert.equal(h.b.taskNavigation.olderCursor.value, undefined);
  assert.equal(h.b.error.value, '任务列表读取失败，请重试。');
});

test('global task browsing preserves mixed projects and legacy empty timestamps', async (t) => {
  const rows = [scan('project-one', 1), scan('project-two', 2),
    { ...scan('unassigned', null), updatedAt: '', status: '' }];
  const h = await mount({ listSentinelScans: async () => rows }, {
    props: { projectId: undefined, initialScanId: 'project-one' }, expectedScanId: 'project-one',
  });
  t.after(h.unmount);
  assert.equal(h.b.error.value, '');
  assert.equal(h.b.scans.value.length, 3);
  await h.b.selectScan('project-two');
  assert.equal(h.b.scanId.value, 'project-two');
  assert.equal(h.b.selectionError.value, '');
});
