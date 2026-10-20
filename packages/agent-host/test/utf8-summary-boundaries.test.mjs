import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeAgentState } from '../src/agent-state.mjs';
import { normalizeMemory } from '../src/memory.mjs';
import { buildDiscoverySummary } from '../src/discovery.mjs';

for (const prefix of ['', 'a', 'aa', 'aaa']) {
  test(`Chinese and emoji remain readable at task, memory and tool-purpose byte boundaries (${prefix.length})`, () => {
    const input = prefix + '中文任务🦖'.repeat(100);
    const state = normalizeAgentState({ goal: input, confirmedFacts: [input] });
    const memory = normalizeMemory({ facts: [input] });
    const apps = {
      capabilities: () => [{ appId: 'org.oviraptor.fixture', name: 'org.oviraptor.fixture::read', description: input, inputSchema: {} }],
      store: { application: () => ({ manifest: { paired_skills: [] } }) },
    };
    const { summary } = buildDiscoverySummary({ apps, projectId: 1 });
    for (const [value, maximum] of [[state.goal, 320], [state.confirmedFacts[0], 240], [memory.facts[0], 240], [summary.apps[0].tools[0].purpose, 160]]) {
      assert(!value.includes('\uFFFD'), `broken UTF-8: ${value}`);
      assert(Buffer.byteLength(value) <= maximum);
      assert(value.endsWith('…'));
    }
  });
}
