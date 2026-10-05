const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');

function liveLogHarness(options = {}) {
  let listener, visibility, next = 0, releases = 0;
  const jobs = new Map(), intervals = new Map();
  const document = {
    hidden: false,
    addEventListener(name, callback) { assert.equal(name, 'visibilitychange'); visibility = callback; },
    removeEventListener(name, callback) { assert.equal(name, 'visibilitychange'); if (visibility === callback) visibility = undefined; },
  };
  const window = {
    setTimeout(callback, delay) { assert.equal(delay, 50); jobs.set(++next, callback); return next; },
    clearTimeout(id) { jobs.delete(id); },
    setInterval(callback, delay) { assert.equal(delay, 15000); intervals.set(++next, callback); return next; },
    clearInterval(id) { intervals.delete(id); },
  };
  const source = fs.readFileSync(path.join(__dirname, '../src/composables/useCommittedRefresh.ts'), 'utf8');
  const committed = {};
  new Function('require', 'exports', 'window', 'document', ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
  } }).outputText)(name => {
    if (name === 'vue') return vue;
    if (name === '@tauri-apps/api/event') return { listen: async (channel, callback) => {
      assert.equal(channel, options.channel || 'nest-runner-log');
      if (options.registration) await options.registration;
      if (options.failure) throw Error('private-registration-error');
      listener = callback;
      return () => { releases++; listener = undefined; };
    } };
    throw Error(`Unexpected live log import ${name}`);
  }, committed, window, document);
  const exports = {};
  const wrapper = fs.readFileSync(path.join(__dirname, '../src/features/sentinel/execution/useLiveRunnerLog.ts'), 'utf8');
  new Function('require', 'exports', ts.transpileModule(wrapper, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
  } }).outputText)(name => {
    assert.equal(name, '../../../composables/useCommittedRefresh'); return committed;
  }, exports);
  return { module: exports,
    committed,
    emit(payload) { listener?.({ payload }); },
    fire() { const callbacks = [...jobs.values()]; jobs.clear(); callbacks.forEach(fn => fn()); },
    reconcile() { [...intervals.values()].forEach(fn => fn()); },
    visibility(hidden) { document.hidden = hidden; visibility?.(); },
    get timers() { return jobs.size; }, get intervals() { return intervals.size; },
    get releases() { return releases; }, get listening() { return Boolean(listener); },
    get observing() { return Boolean(visibility); },
  };
}
module.exports = { liveLogHarness };
