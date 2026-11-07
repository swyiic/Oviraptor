import test from 'node:test';
import assert from 'node:assert/strict';
import { finishTurnDecision, looksLikeSoftConfirmation } from '../src/agent-state.mjs';

const analysis = { name: 'workspace_run', ok: true };
const base = {
  answerMode: 'quality',
  goalText: '解密这个 burp 文件里的响应',
  usedToolThisPrompt: true,
  toolResultsThisPrompt: [analysis],
  forcedContinuesUsed: 0,
};

test('soft confirmation-punt on in-scope work FORCES continue (the user-reported stop)', () => {
  for (const text of [
    '第一步已完成，方案可行。下一步需你确认后再继续。',
    '可行，要不要我继续？',
    '确认无误后再继续执行。',
    '我可以继续吗？',
    'This is feasible — shall I continue?',
    'Need your confirmation to proceed.',
  ]) {
    const d = finishTurnDecision({ ...base, assistantText: text });
    assert.equal(d.continue, true, text);
    assert.equal(d.reason, 'soft_confirmation_override', text);
  }
});

test('a GENUINE request (specific missing info / a choice) still stops', () => {
  for (const text of [
    '我需要你提供 10.255.134.90 的登录凭据才能继续。',
    '请提供目标的 API 文档。',
    'Which option do you want: full scan or quick probe?',
  ]) {
    const d = finishTurnDecision({ ...base, assistantText: text });
    assert.equal(d.continue, false, text);
    assert.equal(d.reason, 'asked_user_question', text);
  }
});

test('a real authorization/scope boundary wins over a soft punt (stops)', () => {
  const d = finishTurnDecision({ ...base, scopeBoundary: true, assistantText: '要不要我继续？' });
  assert.equal(d.continue, false);
  assert.equal(d.reason, 'scope_boundary');
});

test('forced-continue budget caps the override (no infinite loop)', () => {
  const d = finishTurnDecision({ ...base, forcedContinuesUsed: 3, assistantText: '需你确认后再继续' });
  assert.equal(d.continue, false);
  assert.equal(d.reason, 'budget_exhausted');
});

test('looksLikeSoftConfirmation: punts yes, genuine asks no', () => {
  assert.equal(looksLikeSoftConfirmation('需你确认后再继续'), true);
  assert.equal(looksLikeSoftConfirmation('要不要我继续往下做'), true);
  assert.equal(looksLikeSoftConfirmation('shall I continue?'), true);
  assert.equal(looksLikeSoftConfirmation('请提供目标 URL'), false);
  assert.equal(looksLikeSoftConfirmation('which option: A or B?'), false);
  assert.equal(looksLikeSoftConfirmation(''), false);
});
