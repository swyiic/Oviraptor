const assert = require('node:assert/strict');
const test = require('node:test');
const { mount, flush, deferred } = require('./agent_workbench_harness.cjs');

test('a late identity list from the previous workspace cannot restore selection', async (t) => {
  const pending = deferred();
  const first = await mount({ listBrowserAuthSessions: async (projectId) => {
    if (projectId === 2) return pending.promise;
    return [];
  } });
  t.after(first.unmount);
  first.b.form.projectId = 2;
  await flush();
  const staleScope = first.b.authSessionScopeId.value;
  first.b.form.projectId = 1;
  await flush();
  pending.resolve([{ id: 'stale', status: 'valid' }]);
  await flush();
  assert.notEqual(first.b.authSessionScopeId.value, staleScope);
  assert.deepEqual(first.b.authControls.authSessions.value, []);
  assert.deepEqual(first.b.form.authSessionIds, []);
});

test('late listener installation is disposed after unmount', async () => {
  const registration = deferred();
  let closed = 0;
  const app = await mount({}, {}, { listen: () => registration.promise });
  app.unmount();
  registration.resolve(() => { closed++; });
  await flush();
  assert.equal(closed, 1);
});

test('a stale identity event cannot select an account for another draft scope', async (t) => {
  let listener;
  const app = await mount({}, {}, { listen: async (_name, callback) => {
    listener = callback;
    return () => {};
  } });
  t.after(app.unmount);
  const staleScope = app.b.authSessionScopeId.value;
  app.b.resetIdentitySelection();
  await listener({ payload: { id: 'old', status: 'valid', projectId: 1, draftScopeId: staleScope } });
  assert.deepEqual(app.b.form.authSessionIds, []);
  assert.deepEqual(app.b.authControls.authSessions.value, []);
});

test('a delayed login completion cannot select an old account after workspace switch', async (t) => {
  const pending = deferred();
  const newWorkspaceList = deferred();
  const app = await mount({ finishBrowserAuthSession: () => pending.promise,
    listBrowserAuthSessions: (projectId) => projectId === 2 ? newWorkspaceList.promise : Promise.resolve([]) });
  t.after(app.unmount);
  const completion = app.b.authControls.finishLogin({ id: 'old', status: 'capturing' });
  app.b.form.projectId = 2;
  await flush();
  pending.resolve({ id: 'old', status: 'valid' });
  await flush();
  assert.deepEqual(app.b.form.authSessionIds, []);
  assert.equal(app.b.form.authSessionId, '');
  newWorkspaceList.resolve([]);
  await completion;
});
