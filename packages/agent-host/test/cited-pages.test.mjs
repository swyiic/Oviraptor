import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { citedHosts, citedPageUrls, importCitedPages, pastedRequest, saveFetchedUrl, savePastedRequest } from '../src/cited-pages.mjs';
import { runWorkspaceTool } from '../src/workspace-tools.mjs';

const sentence = 'https://hzwb.it.chinamobile.com/omsatt/engine/#/login 分析这个地址的前端，给我解密这些';

test('a hash route is fetched without the fragment', () => {
  const [url] = citedPageUrls(sentence);
  assert.equal(url.href, 'https://hzwb.it.chinamobile.com/omsatt/engine/');
});

function fakeFetch(routes) {
  return async input => {
    const href = String(input);
    const route = routes[href];
    if (!route) throw Object.assign(Error('missing'), { code: 'ENOTFOUND' });
    const bytes = Buffer.from(route.body);
    return {
      status: route.status ?? 200,
      headers: new Headers({ 'content-type': route.type || 'text/html; charset=utf-8', location: route.location || '' }),
      body: new ReadableStream({
        start(controller) {
          controller.enqueue(bytes);
          controller.close();
        },
      }),
    };
  };
}

test('saves the page and same-host scripts, and skips a foreign script', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cited-page-'));
  try {
    const html = `<!doctype html><script src="/omsatt/engine/js/app.js"></script><script src="https://cdn.example/vendor.js"></script>`;
    const fetchImpl = fakeFetch({
      'https://hzwb.it.chinamobile.com/omsatt/engine/': { body: html },
      'https://hzwb.it.chinamobile.com/omsatt/engine/js/app.js': { body: 'function decrypt(data){return AES.decrypt(data,key)}', type: 'application/javascript' },
    });
    const result = await importCitedPages(sentence, root, {
      fetchImpl,
      lookupImpl: async () => [{ address: '203.0.113.10', family: 4 }],
    });
    assert.equal(result.skipped.length, 0);
    const page = await readFile(join(root, 'cited-web/hzwb.it.chinamobile.com/page.html'), 'utf8');
    assert.match(page, /app\.js/);
    const files = result.imported.map(item => item.path);
    assert.ok(files.some(path => path.includes('app.js')));
    assert.equal(files.some(path => path.includes('vendor.js')), false);
    assert.match(result.note, /workspace_fetch/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('does not follow a redirect off the named host', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cited-redir-'));
  try {
    const fetchImpl = fakeFetch({
      'https://hzwb.it.chinamobile.com/omsatt/engine/': { status: 302, location: 'https://evil.example/login', body: '' },
    });
    const result = await importCitedPages(sentence, root, {
      fetchImpl,
      lookupImpl: async () => [{ address: '203.0.113.10', family: 4 }],
    });
    assert.equal(result.imported.length, 0);
    assert.equal(result.skipped[0].reason, 'redirect_left_host');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('a pasted request names its host and is saved for analysis', async () => {
  const raw = 'POST /omsatt/engine/login HTTP/1.1\nHost: hzwb.it.chinamobile.com\nContent-Type: application/json\n\n{"password":"cipher"}';
  assert.equal(pastedRequest(raw).host, 'hzwb.it.chinamobile.com');
  assert.deepEqual(citedHosts(raw), ['hzwb.it.chinamobile.com']);
  const root = await mkdtemp(join(tmpdir(), 'cited-req-'));
  try {
    const saved = await savePastedRequest(raw, root);
    assert.equal(saved.path, 'cited-web/hzwb.it.chinamobile.com/user-request.txt');
    assert.match(await readFile(join(root, saved.path), 'utf8'), /cipher/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('workspace_fetch saves a same-host file and refuses a different host', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cited-fetch-'));
  try {
    const fetchImpl = fakeFetch({
      'https://hzwb.it.chinamobile.com/omsatt/engine/js/chunk.js': { body: 'AES.decrypt(data,key)', type: 'application/javascript' },
    });
    const lookupImpl = async () => [{ address: '203.0.113.10', family: 4 }];
    const saved = await saveFetchedUrl(root, 'https://hzwb.it.chinamobile.com/omsatt/engine/js/chunk.js', {
      allowedHosts: ['hzwb.it.chinamobile.com'], fetchImpl, lookupImpl,
    });
    assert.match(saved.path, /^cited-web\/hzwb\.it\.chinamobile\.com\/fetched\//);
    assert.match(await readFile(join(root, saved.path), 'utf8'), /AES\.decrypt/);
    const denied = await runWorkspaceTool(root, 'workspace_fetch', { url: 'https://cdn.example/vendor.js' }, {
      allowedHosts: ['hzwb.it.chinamobile.com'], fetchImpl, lookupImpl,
    }).then(() => null, error => error.code);
    assert.equal(denied, 'workspace_fetch_host_denied');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('workspace_http can post a body to a named host', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cited-post-'));
  try {
    let seen = {};
    const fetchImpl = async (input, init = {}) => {
      seen = { url: String(input), method: init.method, body: init.body };
      return fakeFetch({
        'https://hzwb.it.chinamobile.com/omsatt/engine/login': { body: '{"ok":true}', type: 'application/json' },
      })(input);
    };
    const saved = await saveFetchedUrl(root, 'https://hzwb.it.chinamobile.com/omsatt/engine/login', {
      openNetwork: true,
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{"password":"cipher"}',
      fetchImpl,
      lookupImpl: async () => [{ address: '203.0.113.10', family: 4 }],
    });
    assert.equal(seen.method, 'POST');
    assert.equal(seen.body, '{"password":"cipher"}');
    assert.equal(saved.method, 'POST');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('after material access, a fetch is not limited to the first host', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cited-open-'));
  try {
    const fetchImpl = fakeFetch({
      'https://cdn.example/vendor.js': { body: 'function decrypt(){return 1}', type: 'application/javascript' },
    });
    const saved = await saveFetchedUrl(root, 'https://cdn.example/vendor.js', {
      openNetwork: true,
      allowedHosts: [],
      materialRoot: join(root, 'material'),
      fetchImpl,
      lookupImpl: async () => [{ address: '203.0.113.8', family: 4 }],
    });
    assert.match(saved.path, /cited-web\/cdn\.example\/fetched\//);
    assert.match(await readFile(join(root, saved.path), 'utf8'), /decrypt/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('refuses a named link-local address', async () => {
  const root = await mkdtemp(join(tmpdir(), 'cited-block-'));
  try {
    let called = false;
    const result = await importCitedPages('分析 http://169.254.169.254/latest/meta-data', root, {
      fetchImpl: async () => { called = true; throw Error('should not fetch'); },
      lookupImpl: async () => [{ address: '169.254.169.254', family: 4 }],
    });
    assert.equal(called, false);
    assert.equal(result.skipped[0].reason, 'blocked_address');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
