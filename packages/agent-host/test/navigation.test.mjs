import test from 'node:test';
import assert from 'node:assert/strict';
import { PageNavigation, NAVIGATION_LIMITS, externalNavigationUrl, handoffPayload } from '../src/navigation.mjs';

const SOURCE = 'org.oviraptor.assets', TARGET = 'org.oviraptor.scanning';
const sourceScope = () => ({ projectId: 1, pageId: 'inventory', version: '2.0.0', expectedAppRevision: 3, expectedGrantRevision: 2 });
const targetScope = () => ({ projectId: 1, pageId: 'workbench', version: '2.1.0', expectedAppRevision: 5, expectedGrantRevision: 4 });
function fixture(options = {}) {
  const applications = new Map([
    [SOURCE, { id: SOURCE, version: '2.0.0', revision: 3, enabled: true, digest: 'a'.repeat(64), manifest: { schemaVersion: 2, pages: [{ id: 'inventory' }], permissions: [{ id: 'ui.external.open' }, { id: 'ui.handoff' }] } }],
    [TARGET, { id: TARGET, version: '2.1.0', revision: 5, enabled: true, digest: 'b'.repeat(64), manifest: { schemaVersion: 2, pages: [{ id: 'workbench' }], permissions: [{ id: 'ui.handoff' }] } }],
  ]);
  const grants = new Map([[`${SOURCE}:1`, { revision: 2, permissions: ['ui.external.open', 'ui.handoff'] }], [`${TARGET}:1`, { revision: 4, permissions: ['ui.handoff'] }], [`${SOURCE}:2`, { revision: 2, permissions: ['ui.external.open', 'ui.handoff'] }], [`${TARGET}:2`, { revision: 4, permissions: ['ui.handoff'] }]]);
  const projects = new Map([[1, { id: 1, name: 'One' }], [2, { id: 2, name: 'Two' }]]);
  const unavailable = () => { throw Object.assign(Error('host_app_unavailable'), { code: 'host_app_unavailable' }); };
  const apps = { store: { application: id => applications.get(id) ?? unavailable(), project: id => projects.get(id) ?? unavailable(), grant: (id, project) => grants.get(`${id}:${project}`) ?? { revision: 0, permissions: [] } }, changing: new Set(), upgradeWorkers: new Map(), workers: new Map(), unresolved: new Set(), ownerUnresolved(id) { return this.unresolved.has(id); }, call() { throw Error('Worker must not be called'); }, worker() { throw Error('Worker must not be started'); } };
  let now = 100000;
  const navigation = new PageNavigation(apps, { now: () => now, ...options });
  return { navigation, apps, applications, grants, projects, advance: n => { now += n; } };
}
const error = (fn, code) => assert.throws(fn, failure => failure.code === code);
const external = f => f.navigation.prepareExternal(SOURCE, sourceScope(), { url: 'https://example.com/path?q=one#two' });
const handoff = (f, payload = { kind: 'scan.targets.v1', targets: ['https://example.com/'] }) => f.navigation.prepareHandoff(SOURCE, sourceScope(), { targetAppId: TARGET, targetPageId: 'workbench', payload });

test('external preparation and single-use confirmation only offer the canonical URL', () => {
  const f = fixture(), prepared = f.navigation.prepareExternal(SOURCE, sourceScope(), { url: 'https://例え.テスト/a' });
  assert.deepEqual(Object.keys(prepared).sort(), ['expiresAt', 'kind', 'projectId', 'token', 'url']);
  assert.match(prepared.token, /^[a-f0-9]{64}$/u); assert.equal(prepared.url, 'https://xn--r8jz45g.xn--zckzah/a');
  assert.deepEqual(f.navigation.confirm(prepared.token, sourceScope()), { kind: 'external', url: prepared.url });
  error(() => f.navigation.confirm(prepared.token, sourceScope()), 'host_navigation_ticket_invalid');
});

test('handoff requires source and target consent and returns a token-free draft-only clone', () => {
  const f = fixture(), payload = { kind: 'scan.targets.v1', targets: ['https://example.com/a?one=1'], nested: { value: 'original' } };
  const prepared = handoff(f, payload); payload.nested.value = 'caller-mutated'; prepared.preview.nested.value = 'preview-mutated';
  assert.deepEqual(Object.keys(prepared).sort(), ['expiresAt', 'kind', 'preview', 'projectId', 'targetAppId', 'targetPageId', 'token']);
  const confirmed = f.navigation.confirm(prepared.token, sourceScope());
  assert.deepEqual(Object.keys(confirmed).sort(), ['deliveryToken', 'kind', 'projectId', 'targetAppId', 'targetPageId']);
  const received = f.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope());
  assert.deepEqual(received, { protocol: 'oviraptor.handoff.v1', purpose: 'draft-only', sourceAppId: SOURCE, projectId: 1, payload: { kind: 'scan.targets.v1', targets: ['https://example.com/a?one=1'], nested: { value: 'original' } } });
  assert.deepEqual(Object.keys(received).sort(), ['payload', 'projectId', 'protocol', 'purpose', 'sourceAppId']);
  error(() => f.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope()), 'host_navigation_ticket_invalid');
});

test('source scope is closed, exact and rejects coercion or model-selected extra authority', () => {
  for (const bad of [{ ...sourceScope(), projectId: '1' }, { ...sourceScope(), projectId: 0 }, { ...sourceScope(), expectedAppRevision: true }, { ...sourceScope(), expectedGrantRevision: -1 }, { ...sourceScope(), pageId: 'missing' }, { ...sourceScope(), permission: 'ui.handoff' }, { ...sourceScope(), operationId: 'one' }, { ...sourceScope(), version: '2.0.0\n' }]) {
    const f = fixture(); assert.throws(() => f.navigation.prepareExternal(SOURCE, bad, { url: 'https://example.com' }));
  }
});

test('navigation input is closed and rejects target project selection', () => {
  const f = fixture();
  error(() => f.navigation.prepareExternal(SOURCE, sourceScope(), { url: 'https://example.com', opened: true }), 'host_navigation_input_invalid');
  error(() => f.navigation.prepareHandoff(SOURCE, sourceScope(), { targetAppId: TARGET, targetPageId: 'workbench', payload: {}, projectId: 2 }), 'host_navigation_input_invalid');
});

test('external URL rejects non-http URLs, credentials, ambiguous slash/whitespace/control and overlong wire bytes', () => {
  for (const url of ['file:///tmp/data', 'javascript:alert(1)', 'data:text/plain,hi', '//example.com', ' https://example.com', 'https://example.com ', 'https://a:b@example.com', 'https://user@example.com', 'https://example.com/\n', 'https://example.com/%0a', 'https://example.com/%1F', 'https://example.com/%7f', 'https://example.com/\\evil', 'https://example.com/\u0085', 'https://example.com/' + 'a'.repeat(2048)]) error(() => externalNavigationUrl(url), 'host_navigation_url_invalid');
  assert.equal(externalNavigationUrl('http://127.0.0.1:8080/a?q=1'), 'http://127.0.0.1:8080/a?q=1');
});

test('UTF8 URL canonical expansion is bounded after URL normalization', () => {
  error(() => externalNavigationUrl('https://example.com/' + '漢'.repeat(300)), 'host_navigation_url_invalid');
  assert.equal(Buffer.byteLength(externalNavigationUrl('https://example.com/' + 'a'.repeat(2028))), 2048);
});

for (const [name, mutate] of [
  ['source grant revision', f => { f.grants.get(`${SOURCE}:1`).revision++; }],
  ['source permission revoked without revision', f => { f.grants.get(`${SOURCE}:1`).permissions = []; }],
  ['source permission undeclared', f => { f.applications.get(SOURCE).manifest.permissions = []; }],
  ['source application revision', f => { f.applications.get(SOURCE).revision++; }],
  ['source package version', f => { f.applications.get(SOURCE).version = '2.0.1'; }],
  ['source package digest', f => { f.applications.get(SOURCE).digest = 'c'.repeat(64); }],
  ['source page removed', f => { f.applications.get(SOURCE).manifest.pages = []; }],
  ['source disabled', f => { f.applications.get(SOURCE).enabled = false; }],
  ['source uninstalled', f => { f.applications.delete(SOURCE); }],
  ['source changing', f => { f.apps.changing.add(SOURCE); }],
  ['source upgrade pending', f => { f.apps.upgradeWorkers.set(SOURCE, {}); }],
  ['source worker stopped', f => { f.apps.workers.set(SOURCE, { stopped: true }); }],
  ['source owner unresolved', f => { f.apps.unresolved.add(SOURCE); }],
  ['project removed', f => { f.projects.delete(1); }],
  ['project inactive', f => { f.projects.get(1).status = 'archived'; }],
]) test(`confirmation rejects ${name} and cannot revive its one-use ticket`, () => {
  const f = fixture(), prepared = external(f); mutate(f);
  assert.throws(() => f.navigation.confirm(prepared.token, sourceScope()));
  error(() => f.navigation.confirm(prepared.token, sourceScope()), 'host_navigation_ticket_invalid');
});

for (const [name, mutate] of [
  ['target grant revision', f => { f.grants.get(`${TARGET}:1`).revision++; }],
  ['target receive permission', f => { f.grants.get(`${TARGET}:1`).permissions = []; }],
  ['target receive declaration', f => { f.applications.get(TARGET).manifest.permissions = []; }],
  ['target application revision', f => { f.applications.get(TARGET).revision++; }],
  ['target package digest', f => { f.applications.get(TARGET).digest = 'c'.repeat(64); }],
  ['target disabled', f => { f.applications.get(TARGET).enabled = false; }],
  ['target uninstalled', f => { f.applications.delete(TARGET); }],
  ['target page removed', f => { f.applications.get(TARGET).manifest.pages = []; }],
  ['target changing', f => { f.apps.changing.add(TARGET); }],
  ['target worker stopped', f => { f.apps.workers.set(TARGET, { stopped: true }); }],
  ['target owner unresolved', f => { f.apps.unresolved.add(TARGET); }],
]) test(`handoff rechecks ${name} before confirmation and again before delivery`, () => {
  const first = fixture(), prepared = handoff(first); mutate(first); assert.throws(() => first.navigation.confirm(prepared.token, sourceScope()));
  const second = fixture(), confirmed = second.navigation.confirm(handoff(second).token, sourceScope()); mutate(second);
  assert.throws(() => second.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope()));
  error(() => second.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope()), 'host_navigation_ticket_invalid');
});

test('delivery also rechecks original source grant after confirmation', () => {
  const f = fixture(), confirmed = f.navigation.confirm(handoff(f).token, sourceScope());
  f.grants.get(`${SOURCE}:1`).revision++;
  error(() => f.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope()), 'host_navigation_authority_changed');
});

test('source and target scopes cannot switch project or page even when another grant exists', () => {
  const f = fixture(), prepared = handoff(f);
  error(() => f.navigation.confirm(prepared.token, { ...sourceScope(), projectId: 2 }), 'host_navigation_authority_changed');
  const prepared2 = handoff(f), confirmed = f.navigation.confirm(prepared2.token, sourceScope());
  error(() => f.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, { ...targetScope(), projectId: 2 }), 'host_navigation_authority_changed');
  const confirmed2 = f.navigation.confirm(handoff(f).token, sourceScope());
  error(() => f.navigation.consumeHandoff(confirmed2.deliveryToken, SOURCE, sourceScope()), 'host_navigation_authority_changed');
});

test('both source and target must be native v2 and declare and hold their reserved permission', () => {
  for (const id of [SOURCE, TARGET]) {
    const f = fixture(); f.applications.get(id).manifest.schemaVersion = 1;
    error(() => handoff(f), 'host_navigation_unavailable');
  }
  const f = fixture(); f.grants.get(`${SOURCE}:1`).permissions = ['ui.external.open'];
  error(() => handoff(f), 'host_navigation_permission_denied');
});

test('cancel is explicit, one-use and still cleans up after permission revocation', () => {
  const f = fixture(), prepared = external(f); f.grants.get(`${SOURCE}:1`).permissions = [];
  assert.deepEqual(f.navigation.cancel(prepared.token, sourceScope()), { cancelled: true });
  error(() => f.navigation.confirm(prepared.token, sourceScope()), 'host_navigation_ticket_invalid');
  error(() => f.navigation.cancel(prepared.token, sourceScope()), 'host_navigation_ticket_invalid');
});

test('cancellation cannot consume another scope ticket', () => {
  const f = fixture(), prepared = external(f);
  error(() => f.navigation.cancel(prepared.token, { ...sourceScope(), projectId: 2 }), 'host_navigation_ticket_invalid');
  assert.equal(f.navigation.confirm(prepared.token, sourceScope()).kind, 'external');
});

test('TTL expires at the exact boundary and confirmation does not extend delivery TTL', () => {
  const f = fixture({ ttlMs: 1000 }), prepared = handoff(f); f.advance(999);
  const confirmed = f.navigation.confirm(prepared.token, sourceScope()); f.advance(1);
  error(() => f.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope()), 'host_navigation_ticket_invalid');
  const prepared2 = external(f); f.advance(1000);
  error(() => f.navigation.confirm(prepared2.token, sourceScope()), 'host_navigation_ticket_invalid');
});

test('combined pending and confirmed deliveries are limited, release on consume and expire', () => {
  const f = fixture({ maxPending: 2 }), first = handoff(f), second = external(f);
  const confirmed = f.navigation.confirm(first.token, sourceScope());
  error(() => external(f), 'host_navigation_limit');
  f.navigation.cancel(second.token, sourceScope()); const third = external(f);
  f.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope()); const fourth = external(f);
  assert.notEqual(third.token, fourth.token); f.advance(30000); assert.equal(external(f).kind, 'external');
});

test('close and manager shutdown reject prepare, confirm and consume and clear tokens', () => {
  const f = fixture(), prepared = external(f), confirmed = f.navigation.confirm(handoff(f).token, sourceScope()); f.navigation.close();
  error(() => external(f), 'host_navigation_unavailable');
  error(() => f.navigation.confirm(prepared.token, sourceScope()), 'host_navigation_unavailable');
  error(() => f.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope()), 'host_navigation_unavailable');
  const second = fixture(); second.apps.closing = true; error(() => external(second), 'host_navigation_unavailable');
});

test('payload preserves complete 100-item selection without an artificial 32-item limit', () => {
  const payload = { kind: 'scan.targets.v1', targets: Array.from({ length: 100 }, (_, i) => 'https://example.com/' + i + '/' + 'a'.repeat(2000)) };
  const f = fixture(), prepared = handoff(f, payload), confirmed = f.navigation.confirm(prepared.token, sourceScope());
  assert.deepEqual(prepared.preview, payload);
  assert.deepEqual(f.navigation.consumeHandoff(confirmed.deliveryToken, TARGET, targetScope()).payload, payload);
});

test('payload wire bytes accept exact 512KiB and reject 512KiB+1 without truncation', () => {
  const payload = { data: Array(127).fill('a'.repeat(4096)) }; payload.data.push('');
  const remaining = NAVIGATION_LIMITS.payloadBytes - Buffer.byteLength(JSON.stringify(payload));
  assert.ok(remaining > 0 && remaining < 4096); payload.data[127] = 'a'.repeat(remaining);
  assert.equal(Buffer.byteLength(JSON.stringify(handoffPayload(payload))), NAVIGATION_LIMITS.payloadBytes);
  payload.data[127] += 'a'; error(() => handoffPayload(payload), 'host_navigation_payload_too_large');
});

test('payload bounds UTF8 scalar bytes, array size, object width, depth and total nodes', () => {
  error(() => handoffPayload({ text: '😀'.repeat(1025) }), 'host_navigation_payload_too_large');
  error(() => handoffPayload({ items: Array(1025).fill(1) }), 'host_navigation_payload_too_large');
  error(() => handoffPayload(Object.fromEntries(Array.from({ length: 65 }, (_, i) => ['key' + i, 1]))), 'host_navigation_payload_too_large');
  let nested = 'x'; for (let i = 0; i < 9; i++) nested = { nested };
  error(() => handoffPayload(nested), 'host_navigation_payload_too_large');
  error(() => handoffPayload(Object.fromEntries(Array.from({ length: 64 }, (_, i) => ['key' + i, Array(128).fill(1)]))), 'host_navigation_payload_too_large');
});

test('payload rejects cycles, exotic values, sparse arrays, nonfinite, prototype keys and unsafe controls', () => {
  const cyclic = {}; cyclic.self = cyclic;
  for (const payload of [cyclic, { x: undefined }, { x: Infinity }, { x: BigInt(1) }, { x: new Date() }, { x: new Map() }, { x: new Uint8Array(1) }, { x: [,,] }, { x: () => 1 }, { x: 'a\0b' }, JSON.parse('{"__proto__":{"polluted":true}}'), { constructor: 'x' }, { prototype: 'x' }]) error(() => handoffPayload(payload), 'host_navigation_payload_invalid');
  assert.deepEqual(handoffPayload({ text: 'one\ntwo\tthree\r\n' }), { text: 'one\ntwo\tthree\r\n' });
  assert.equal({}.polluted, undefined);
});

test('validation never executes getters or toJSON callbacks', () => {
  let invoked = 0; const getter = {}; Object.defineProperty(getter, 'value', { enumerable: true, get() { invoked++; return 'secret'; } });
  error(() => handoffPayload(getter), 'host_navigation_payload_invalid');
  error(() => handoffPayload({ toJSON() { invoked++; return {}; } }), 'host_navigation_payload_invalid');
  const f = fixture(), input = {}; Object.defineProperty(input, 'url', { enumerable: true, get() { invoked++; return 'https://example.com'; } });
  error(() => f.navigation.prepareExternal(SOURCE, sourceScope(), input), 'host_navigation_input_invalid');
  assert.equal(invoked, 0);
});

test('malformed tokens and invalid constructor/clock limits fail closed', () => {
  const f = fixture();
  for (const token of [undefined, '', 'a'.repeat(63), 'A'.repeat(64), 'x'.repeat(100000)]) error(() => f.navigation.confirm(token, sourceScope()), 'host_navigation_ticket_invalid');
  for (const options of [{ ttlMs: 60001 }, { ttlMs: 0 }, { maxPending: 65 }, { maxPending: true }, { now: 'now' }]) error(() => new PageNavigation(f.apps, options), 'host_navigation_config_invalid');
  const bad = new PageNavigation(f.apps, { now: () => NaN }); error(() => bad.prepareExternal(SOURCE, sourceScope(), { url: 'https://example.com' }), 'host_navigation_clock_invalid');
});
