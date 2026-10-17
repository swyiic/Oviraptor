import test from 'node:test';
import assert from 'node:assert/strict';
import { buildDiscoverySummary, MAX_SUMMARY_BYTES } from '../src/discovery.mjs';

function fakeApps(toolCount, { required = ['taskId', 'note'], description = 'Synthetic tool for discovery byte-limit tests. '.repeat(3) } = {}) {
  const tools = Array.from({ length: toolCount }, (_, i) => ({
    appId: 'org.oviraptor.fixture.discovery',
    name: `org.oviraptor.fixture.discovery::tool_${String(i).padStart(3, '0')}`,
    description,
    inputSchema: { type: 'object', properties: { taskId: { type: 'string' }, note: { type: 'string' } }, required, additionalProperties: false },
    permission: 'fixture.read',
    grantRevision: 1,
  }));
  return {
    capabilities() { return tools; },
    store: {
      application() {
        return { manifest: { paired_skills: [] } };
      },
    },
  };
}

test('discovery summary attaches purpose and required param names under the byte cap', () => {
  const { summary, text } = buildDiscoverySummary({ apps: fakeApps(3), projectId: 1 });
  assert.ok(Buffer.byteLength(text) <= MAX_SUMMARY_BYTES);
  assert.equal(summary.authority, 'none');
  assert.equal(summary.untrusted, true);
  const tool = summary.apps[0].tools[0];
  assert.ok(tool.purpose.length > 0);
  assert.deepEqual(tool.required, ['taskId', 'note']);
  assert.match(tool.name, /__tool_000$/);
});

test('discovery summary truncates tools in order when over the byte limit and stays within MAX_SUMMARY_BYTES', () => {
  const { summary, text } = buildDiscoverySummary({
    apps: fakeApps(200, { description: 'Long purpose text for truncation pressure. '.repeat(4) }),
    projectId: 1,
  });
  assert.ok(Buffer.byteLength(text) <= MAX_SUMMARY_BYTES, `bytes=${Buffer.byteLength(text)}`);
  assert.equal(summary.truncated, true);
  const names = summary.apps.flatMap(app => app.tools.map(tool => tool.name));
  assert.ok(names.length < 200);
  // Ordered truncation pops from the end: first tools of the first app should remain when possible.
  if (names.length > 1) {
    assert.ok(names[0].endsWith('__tool_000') || names[0].includes('tool_'));
    const indexes = names.map(name => Number(name.match(/tool_(\d+)/)?.[1]));
    for (let i = 1; i < indexes.length; i++) assert.ok(indexes[i] > indexes[i - 1], 'remaining tools stay in ascending order after end-truncation');
  }
});

test('discovery summary drops whole apps before collapsing to an empty truncated shell', () => {
  const apps = {
    capabilities() {
      return Array.from({ length: 40 }, (_, appIndex) => Array.from({ length: 20 }, (_, toolIndex) => ({
        appId: `org.oviraptor.fixture.app${appIndex}`,
        name: `org.oviraptor.fixture.app${appIndex}::t${toolIndex}`,
        description: 'x'.repeat(140),
        inputSchema: { type: 'object', properties: { a: { type: 'string' } }, required: ['a'], additionalProperties: false },
        permission: 'p',
        grantRevision: 1,
      }))).flat();
    },
    store: { application: () => ({ manifest: { paired_skills: [] } }) },
  };
  const { summary, text } = buildDiscoverySummary({ apps, projectId: 9 });
  assert.ok(Buffer.byteLength(text) <= MAX_SUMMARY_BYTES);
  assert.equal(summary.truncated, true);
  assert.ok(summary.apps.length < 40);
});
