const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');

const transpile = code => ts.transpileModule(code, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const presentation = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(
  __dirname, '../src/features/sentinel/presentation.ts'), 'utf8')))(require, presentation);

async function renderResultComponent(relativePath, props, dependencies = {}) {
  return renderWorkspaceComponent(`src/features/sentinel/components/results/${relativePath}`, props, dependencies);
}

function loadWorkspaceModule(relativePath, dependencies = {}) {
  const cache = new Map();
  function load(filename) {
    if (cache.has(filename)) return cache.get(filename);
    const exports = {}; cache.set(filename, exports);
    const local = name => {
      if (Object.hasOwn(dependencies, name)) return dependencies[name];
      if (name === '../../presentation') return presentation;
      if (!name.startsWith('.')) return require(name);
      const resolved = path.resolve(path.dirname(filename), name);
      if (resolved === path.join(__dirname, '../src/i18n')) return { useI18n: () => ({ tr: zh => zh }) };
      return load(path.extname(resolved) ? resolved : `${resolved}.ts`);
    };
    const source = fs.readFileSync(filename, 'utf8');
    if (!filename.endsWith('.vue')) {
      new Function('require', 'exports', transpile(source))(local, exports);
      return exports;
    }
    const { descriptor } = parse(source, { filename });
    const script = compileScript(descriptor, { id: filename });
    const template = compileTemplate({ source: descriptor.template.content, filename,
      id: filename, compilerOptions: { bindingMetadata: script.bindings } });
    assert.deepEqual(template.errors, []);
    new Function('require', 'exports', transpile(script.content))(local, exports);
    const rendered = {};
    new Function('require', 'exports', transpile(template.code))(local, rendered);
    exports.default.render = rendered.render;
    return exports;
  }
  return load(path.join(__dirname, '..', relativePath));
}

async function renderWorkspaceComponent(relativePath, props, dependencies = {}) {
  return renderToString(vue.createSSRApp(loadWorkspaceModule(relativePath, dependencies).default, props));
}

module.exports = { renderResultComponent, renderWorkspaceComponent, loadWorkspaceModule };
