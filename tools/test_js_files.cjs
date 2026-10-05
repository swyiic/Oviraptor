const assert = require('node:assert/strict');
const test = require('node:test');
const { renderResultComponent } = require('./result_component_harness.cjs');

const render = (jsRows, copyText = async () => {}) => renderResultComponent(
  'SentinelJsFiles.vue', { jsRows, copyText },
  { '@lucide/vue': { ClipboardCopy: () => null, FileJson: () => null } },
);

test('JS file inventory preserves metadata, provenance and error state', async () => {
  const html = await render([{ id: 1, recordJson: JSON.stringify({
    type: 'script', url: 'https://example.test/app.js', size: 1024,
    isMinified: true, discoveredFrom: 'html',
    analysis: { sourceMapReference: true, module: true, moduleCount: 3,
      businessScore: 2, extractionEngine: 'inventory' }, error: 'source map unavailable',
  }) }]);
  for (const value of ['JS 文件', 'app.js', '已压缩', 'HTML', 'Source Map', 'ES Module',
    '模块', '业务信号', 'source map unavailable']) assert.ok(html.includes(value), value);
});

test('empty JS inventory remains explicitly empty', async () => {
  const html = await render([]);
  assert.ok(html.includes('没有 JS 分析记录'));
});
