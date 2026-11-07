import test from 'node:test';
import assert from 'node:assert/strict';
import { buildTurnPrompt, PROGRESS_NARRATION_GUIDANCE, DISCOVERY_CHAIN_GUIDANCE } from '../src/quality-prompt.mjs';

// Transparency, not a gate: a long autonomous run must narrate "what phase am I in"
// as it goes (instead of leaving the operator staring at "思考中·N 项操作"), and a
// decrypt/reverse/protocol-analysis deliverable must carry its reasoning chain, not
// just the conclusion. Neither is a new stop-and-ask gate — both only add text the
// model is asked to produce while it keeps working.

test('buildTurnPrompt: quality mode carries both transparency guidances; speed mode carries neither', () => {
  const quality = buildTurnPrompt({ userText: 'decrypt this capture', answerMode: 'quality' });
  assert.ok(quality.includes(PROGRESS_NARRATION_GUIDANCE));
  assert.ok(quality.includes(DISCOVERY_CHAIN_GUIDANCE));
  const speed = buildTurnPrompt({ userText: 'decrypt this capture', answerMode: 'speed' });
  assert.ok(!speed.includes(PROGRESS_NARRATION_GUIDANCE));
  assert.ok(!speed.includes(DISCOVERY_CHAIN_GUIDANCE));
});

test('PROGRESS_NARRATION_GUIDANCE never tells the model to pause or wait for the operator', () => {
  assert.ok(!/等(?:你|用户)(?:确认|回复|批准)/.test(PROGRESS_NARRATION_GUIDANCE));
  assert.ok(/不要等待/.test(PROGRESS_NARRATION_GUIDANCE));
  // Gives concrete phase-labeled examples, not just an abstract instruction.
  assert.ok(PROGRESS_NARRATION_GUIDANCE.includes('发现疑似漏洞'));
  assert.ok(PROGRESS_NARRATION_GUIDANCE.includes('确认漏洞'));
});

test('DISCOVERY_CHAIN_GUIDANCE scopes itself to decrypt/reverse/protocol-analysis tasks, not every report', () => {
  assert.ok(/解密、逆向、协议/.test(DISCOVERY_CHAIN_GUIDANCE));
  assert.ok(/普通渗透任务不需要/.test(DISCOVERY_CHAIN_GUIDANCE));
});
