import test from 'node:test';
import assert from 'node:assert/strict';
import { buildTurnPrompt, PROGRESS_NARRATION_GUIDANCE, DISCOVERY_CHAIN_GUIDANCE, DECRYPT_ARTIFACT_GUIDANCE } from '../src/quality-prompt.mjs';

// Transparency, not a gate: a long autonomous run must narrate "what phase am I in"
// as it goes (instead of leaving the operator staring at "思考中·N 项操作"), any
// confirmed-finding deliverable (pentest or decrypt alike) must carry its reasoning
// chain, not just the conclusion, and a decrypt task should actively pull the target's
// own JS rather than rely only on what the user pasted, plus offer a local-proxy
// deliverable form. Neither is a new stop-and-ask gate — all three only add text the
// model is asked to produce while it keeps working.

test('buildTurnPrompt: quality mode carries all three transparency guidances; speed mode carries none', () => {
  const quality = buildTurnPrompt({ userText: 'decrypt this capture', answerMode: 'quality' });
  assert.ok(quality.includes(PROGRESS_NARRATION_GUIDANCE));
  assert.ok(quality.includes(DISCOVERY_CHAIN_GUIDANCE));
  assert.ok(quality.includes(DECRYPT_ARTIFACT_GUIDANCE));
  const speed = buildTurnPrompt({ userText: 'decrypt this capture', answerMode: 'speed' });
  assert.ok(!speed.includes(PROGRESS_NARRATION_GUIDANCE));
  assert.ok(!speed.includes(DISCOVERY_CHAIN_GUIDANCE));
  assert.ok(!speed.includes(DECRYPT_ARTIFACT_GUIDANCE));
});

test('PROGRESS_NARRATION_GUIDANCE never tells the model to pause or wait for the operator', () => {
  assert.ok(!/等(?:你|用户)(?:确认|回复|批准)/.test(PROGRESS_NARRATION_GUIDANCE));
  assert.ok(/不要等待/.test(PROGRESS_NARRATION_GUIDANCE));
  // Gives concrete phase-labeled examples, not just an abstract instruction.
  assert.ok(PROGRESS_NARRATION_GUIDANCE.includes('发现疑似漏洞'));
  assert.ok(PROGRESS_NARRATION_GUIDANCE.includes('确认漏洞'));
});

test('DISCOVERY_CHAIN_GUIDANCE triggers on a confirmed finding OR a decrypt/reverse task, not only decrypt — the user tried both', () => {
  assert.ok(DISCOVERY_CHAIN_GUIDANCE.includes('record_finding'));
  assert.ok(/解密、逆向、协议/.test(DISCOVERY_CHAIN_GUIDANCE));
  // Explicitly NOT restricted to decrypt-type tasks any more — a plain SQLi pentest
  // report needs the same chain.
  assert.ok(/不限定任务类型/.test(DISCOVERY_CHAIN_GUIDANCE));
  assert.ok(/SQL 注入/.test(DISCOVERY_CHAIN_GUIDANCE));
  assert.ok(!/普通渗透任务不需要/.test(DISCOVERY_CHAIN_GUIDANCE));
  // A read-only task that found nothing still doesn't need to force this section.
  assert.ok(/没有任何确认发现的任务不需要/.test(DISCOVERY_CHAIN_GUIDANCE));
});

test('DECRYPT_ARTIFACT_GUIDANCE requires actively fetching the target\'s own JS, not just user-supplied files, and offers a standalone proxy script', () => {
  assert.ok(/主动用已有工具把目标站点自己的前端 JS 全部抓下来/.test(DECRYPT_ARTIFACT_GUIDANCE));
  assert.ok(/workspace_fetch/.test(DECRYPT_ARTIFACT_GUIDANCE));
  assert.ok(/本地代理/.test(DECRYPT_ARTIFACT_GUIDANCE));
  // Must disambiguate: a standalone script for the user's own machine, not a claim
  // about Oviraptor's own native capabilities.
  assert.ok(/不是 Oviraptor 内置能力/.test(DECRYPT_ARTIFACT_GUIDANCE));
});
