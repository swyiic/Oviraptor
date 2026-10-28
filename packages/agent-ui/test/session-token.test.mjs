import test from 'node:test';
import assert from 'node:assert/strict';
import { sessionToken } from '../src/security.js';

function page(hash) {
  const storage = new Map();
  const window = {
    location: { hash, pathname: '/', search: '' },
    history: { replaceState: (_state, _title, url) => { window.location.hash = ''; window.replaced = url; } },
    sessionStorage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) },
  };
  const document = { querySelector: () => null };
  return { window, document };
}

test('the session token comes from the URL fragment once and is removed from the address bar', () => {
  const token = 'ab'.repeat(16);
  const { window, document } = page(`#session=${token}`);
  assert.equal(sessionToken(document, window), token);
  assert.equal(window.location.hash, '');
  assert.equal(window.replaced, '/');
  assert.equal(sessionToken(document, window), token, 'a reload in the same tab keeps working');
});

test('a malformed fragment is ignored', () => {
  const { window, document } = page('#session=not-a-token');
  assert.equal(sessionToken(document, window), '');
});
