const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const tauri = require('@tauri-apps/api/core');

// Execute the production composable, API and Tauri invoke bridge. Only the
// native IPC endpoint is synthetic; Rust resource behavior has its own tests.
function load(relative, imports) {
  const source = fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  new Function('require', 'exports', compiled)(imports, exports);
  return exports;
}

const skill = { id: 41, name: 'Synthetic guidance', description: 'Reference only', entryPath: 'skills/example/SKILL.md', enabled: true };
const owner = { id: 'local-synthetic', name: 'Synthetic package', skills: [skill] };
// The first two native byte pages stop before a multibyte character. Their
// JavaScript string lengths intentionally differ from the next byte offsets.
const pageTexts = ['a'.repeat(8191), `中😀${'b'.repeat(8184)}`, '中😀tail'];
const text = pageTexts.join('');
const starts = [0, 8191, 16382];
const resource = { path: skill.entryPath, sizeBytes: Buffer.byteLength(text), sha256: 'synthetic-content-digest', text: true, kind: 'instructions' };
const secondResource = { ...resource, path: 'skills/example/references/other.md', sizeBytes: 15, kind: 'reference' };
const flush = async () => { for (let index = 0; index < 12; index++) await vue.nextTick(); };
function deferred() {
  let resolve;
  const promise = new Promise(yes => { resolve = yes; });
  return { promise, resolve };
}
function page(offset) {
  const index = starts.indexOf(offset);
  if (index < 0) throw Error('skill_package_offset_invalid');
  return {
    skillId: skill.id, path: resource.path, content: pageTexts[index], encoding: 'utf8', offset,
    nextOffset: starts[index + 1] ?? null, hasMore: index < starts.length - 1, totalBytes: resource.sizeBytes,
  };
}

function mount(t, overrides = {}) {
  const calls = [];
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    __TAURI_INTERNALS__: {
      async invoke(command, args) {
        calls.push({ command, args: structuredClone(args) });
        if (command === 'list_skill_packages') return [structuredClone(owner)];
        if (command === 'list_skill_package_resources') {
          assert.deepEqual(args, { skillId: skill.id });
          return { skillId: skill.id, packageId: owner.id, resources: [resource, secondResource] };
        }
        assert.equal(command, 'read_skill_package_resource', 'Resource preview must remain read-only');
        assert.deepEqual(Object.keys(args).sort(), ['limit', 'offset', 'path', 'skillId']);
        assert.equal(args.skillId, skill.id);
        // Native store.rs rejects a page larger than 8192 bytes. This strict
        // boundary catches a frontend default that permissive mocks miss.
        if (!Number.isInteger(args.limit) || args.limit < 1 || args.limit > 8192) throw Error('skill_package_page_limit');
        if (overrides.read) return overrides.read(args);
        if (args.path === secondResource.path) return { skillId: skill.id, path: args.path, content: 'other reference', encoding: 'utf8', offset: 0, nextOffset: null, hasMore: false, totalBytes: 15 };
        assert.equal(args.path, resource.path);
        return page(args.offset);
      },
    },
  } });
  const { skillsApi } = load('src/features/skills/api.ts', name => {
    assert.equal(name, '@tauri-apps/api/core');
    return tauri;
  });
  const { useSkillsLibrary } = load('src/features/skills/useSkillsLibrary.ts', name => {
    assert.equal(name, 'vue');
    return vue;
  });
  let state;
  const app = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null })
    .createApp({ setup() { state = useSkillsLibrary({ api: skillsApi, pickDirectory: async () => null }); return () => null; } });
  app.mount({});
  t.after(() => {
    app.unmount();
    if (oldWindow) Object.defineProperty(globalThis, 'window', oldWindow);
    else delete globalThis.window;
  });
  return { state, api: skillsApi, calls, reads: () => calls.filter(call => call.command === 'read_skill_package_resource') };
}

test('production Skills API default passes the native IPC page limit without widening it', async t => {
  const mounted = mount(t);
  const result = await mounted.api.readResource(skill.id, resource.path);
  assert.equal(result.content, pageTexts[0]);
  assert.deepEqual(mounted.reads()[0], { command: 'read_skill_package_resource', args: { skillId: skill.id, path: resource.path, offset: 0, limit: 8192 } });
  await assert.rejects(mounted.api.readResource(skill.id, resource.path, 0, 8193), /skill_package_page_limit/);
});

test('opening and continuing a resource follows native UTF-8 byte offsets without loss or duplication', async t => {
  const mounted = mount(t);
  await flush();
  await mounted.state.openSkill(owner, skill);
  assert.equal(mounted.state.resourceError.value, '');
  assert.equal(mounted.state.resourceText.value, pageTexts[0]);
  await mounted.state.readResource(resource, true);
  assert.notEqual(mounted.state.resourceText.value.length, mounted.state.resourcePage.value.nextOffset);
  await mounted.state.readResource(resource, true);
  assert.equal(mounted.state.resourceText.value, text);
  assert.equal(Buffer.byteLength(mounted.state.resourceText.value), resource.sizeBytes);
  assert.equal(mounted.state.resourceText.value.includes('\uFFFD'), false);
  assert.deepEqual(mounted.reads().map(call => call.args.offset), starts);
  assert.equal(mounted.state.resourcePage.value.hasMore, false);
  assert.equal(mounted.state.resourcePage.value.nextOffset, null);
  await mounted.state.readResource(resource, true);
  assert.equal(mounted.reads().length, 3, 'Completed resource must not request another page');
});

test('failed continuation preserves displayed text and retries the same byte page only on user action', async t => {
  let rejected = false;
  const mounted = mount(t, { read(args) {
    if (args.offset === starts[1] && !rejected) { rejected = true; throw Error('synthetic native read failure'); }
    return page(args.offset);
  } });
  await flush();
  await mounted.state.openSkill(owner, skill);
  await mounted.state.readResource(resource, true);
  assert.match(mounted.state.resourceError.value, /synthetic native read failure/);
  assert.equal(mounted.state.resourceText.value, pageTexts[0]);
  assert.equal(mounted.state.resourcePage.value.nextOffset, starts[1]);
  await flush();
  assert.equal(mounted.reads().length, 2, 'Transport failure must not automatically repeat a read');
  await mounted.state.readResource(resource, true);
  assert.equal(mounted.state.resourceError.value, '');
  assert.equal(mounted.state.resourceText.value, pageTexts[0] + pageTexts[1]);
  assert.deepEqual(mounted.reads().map(call => call.args.offset), [0, starts[1], starts[1]]);
});

test('a continuation arriving after another resource is selected cannot append old Skill content', async t => {
  const late = deferred();
  const mounted = mount(t, { read(args) {
    if (args.path === secondResource.path) return { skillId: skill.id, path: args.path, content: 'other reference', encoding: 'utf8', offset: 0, nextOffset: null, hasMore: false, totalBytes: 15 };
    return args.offset === starts[1] ? late.promise : page(args.offset);
  } });
  await flush();
  await mounted.state.openSkill(owner, skill);
  const pending = mounted.state.readResource(resource, true);
  await mounted.state.readResource(secondResource);
  late.resolve(page(starts[1]));
  await pending;
  assert.equal(mounted.state.selectedResource.value.path, secondResource.path);
  assert.equal(mounted.state.resourceText.value, 'other reference');
  assert.equal(mounted.state.resourcePage.value.path, secondResource.path);
  assert.equal(mounted.state.resourceLoading.value, false);
});
