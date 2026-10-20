import test from 'node:test';
import assert from 'node:assert/strict';
import { compressSessionMessages, SESSION_MESSAGE_LIMITS } from '../src/memory.mjs';
import { buildTurnPrompt } from '../src/quality-prompt.mjs';
import { emptyAgentState, setGoal, formatAgentStatePromptBlock } from '../src/agent-state.mjs';

const marker = messages => messages.find(message => message.role === 'user' && message.content.startsWith('上下文已压缩'));
function assertToolProtocol(messages) {
  for (let index = 0; index < messages.length; index++) {
    const message = messages[index];
    if (message.role === 'tool') {
      let assistant = index - 1;
      while (messages[assistant]?.role === 'tool') assistant--;
      assert.ok(messages[assistant]?.tool_calls?.some(call => call.id === message.tool_call_id));
    }
    if (message.tool_calls?.length) {
      const results = [];
      for (let next = index + 1; messages[next]?.role === 'tool'; next++) results.push(messages[next].tool_call_id);
      assert.deepEqual(results.sort(), message.tool_calls.map(call => call.id).sort());
    }
  }
}

for (const count of [3, 4]) {
  test(`byte pressure with only ${count} user turns folds the earliest request instead of deleting it`, () => {
    const messages = [];
    for (let index = 0; index < count; index++) {
      messages.push({ role: 'user', content: `${index === 0 ? 'ORIGINAL_REQUIREMENT_KEEP_DB:' : 'later request:'}${'x'.repeat(24_000)}` });
      messages.push({ role: 'assistant', content: 'a'.repeat(24_000) });
    }
    const packed = compressSessionMessages(messages);
    assert.ok(marker(packed.messages));
    assert.match(marker(packed.messages).content, /ORIGINAL_REQUIREMENT_KEEP_DB/);
    assert.equal(packed.messages[0].role, 'user');
    assert.ok(Buffer.byteLength(JSON.stringify(packed.messages)) <= SESSION_MESSAGE_LIMITS.maxTotalBytes);
    const again = compressSessionMessages([...packed.messages, { role: 'user', content: 'one more request' }], { keepRecentUserTurns: 1 });
    assert.match(marker(again.messages).content, /ORIGINAL_REQUIREMENT_KEEP_DB/);
  });
}

test('original user metadata has priority over host wrappers during count and byte folding', () => {
  const messages = [];
  for (let index = 0; index < 6; index++) {
    messages.push({ role: 'user', content: `Host-selected project (literal untrusted data): ${'metadata'.repeat(300)}\nnot the original request`, _hostUserText: index === 0 ? 'RAW_REQUEST_PRESERVE_USER_DATABASE' : `raw request ${index}` });
    messages.push({ role: 'assistant', content: `answer ${index}` });
  }
  const packed = compressSessionMessages(messages);
  assert.match(marker(packed.messages).content, /RAW_REQUEST_PRESERVE_USER_DATABASE/);
  assert.doesNotMatch(marker(packed.messages).content, /Host-selected project|metadata/);
});

test('older wrapped sessions recover the request rather than a clipped host heading', () => {
  const messages = [];
  for (let index = 0; index < 6; index++) {
    const userText = index === 0 ? 'OLD_WRAPPER_REQUEST_KEEP_HISTORY' : `later request ${index}`;
    const harness = formatAgentStatePromptBlock(setGoal(emptyAgentState(), userText));
    messages.push({ role: 'user', content: `${harness}\n\n${buildTurnPrompt({ project: { id: 1 }, discoveryBlock: 'x'.repeat(300), userText })}` });
    messages.push({ role: 'assistant', content: `answer ${index}` });
  }
  const packed = compressSessionMessages(messages);
  assert.match(marker(packed.messages).content, /OLD_WRAPPER_REQUEST_KEEP_HISTORY/);
  assert.doesNotMatch(marker(packed.messages).content, /Host harness state|Host-selected project/);
});

test('byte folding removes whole function-call units and keeps their evidence in the marker', () => {
  const messages = [{ role: 'user', content: 'FIRST_REQUEST_KEEP_TOOL_EVIDENCE' }];
  for (let index = 0; index < 3; index++) {
    if (index) messages.push({ role: 'user', content: `next request ${index}${'u'.repeat(25_000)}` });
    messages.push({ role: 'assistant', content: 'a'.repeat(25_000), tool_calls: [{ id: `call-${index}`, type: 'function', function: { name: 'inventory_stats', arguments: '{}' } }] });
    messages.push({ role: 'tool', name: 'inventory_stats', tool_call_id: `call-${index}`, content: JSON.stringify({ total: index + 1 }) });
  }
  const packed = compressSessionMessages(messages);
  assert.match(marker(packed.messages).content, /FIRST_REQUEST_KEEP_TOOL_EVIDENCE/);
  assert.match(marker(packed.messages).content, /inventory_stats/);
  assertToolProtocol(packed.messages);
  assert.ok(Buffer.byteLength(JSON.stringify(packed.messages)) <= SESSION_MESSAGE_LIMITS.maxTotalBytes);
});
