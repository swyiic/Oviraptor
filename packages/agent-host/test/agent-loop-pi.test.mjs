// Tests for the Pi-inspired loop mechanics:
//   Task 1 — finishTurn completion policy (bounded force-continue).
//   Task 2 — mid-conversation tool-set-change announcement.
// Part 1 unit-tests the pure decision functions. Part 2 drives the REAL
// provider turn loop (OpenAiCompatibleRuntime.prompt) through a stubbed model
// stream + a stubbed Host, asserting the loop actually forces/does-not-force a
// continue and actually injects the announcement.
import test from 'node:test';
import assert from 'node:assert/strict';
import { OpenAiCompatibleRuntime, providerToolName } from '../src/provider.mjs';
import {
  finishTurnDecision,
  FORCED_CONTINUE_BUDGET,
  deliverableExpected,
  didAnalysisWork,
  producedDeliverable,
  looksLikeQuestionToUser,
  toolSetAnnouncement,
  deliverableSelfCheckDecision,
  latestDeliverableWritePath,
  buildSuggestedNext,
  emptyAgentState,
  setGoal,
} from '../src/agent-state.mjs';

// ---------------------------------------------------------------------------
// Part 1 — pure decision functions
// ---------------------------------------------------------------------------

const AUDIT_GOAL = 'audit the application source for an injection vulnerability and write the findings report';
const workDone = [{ name: 'workspace_read', ok: true, arguments: { path: 'a.mjs' } }];

test('finishTurnDecision (a): deliverable task that did analysis work with no deliverable and no question → forces continue', () => {
  const decision = finishTurnDecision({
    goalText: AUDIT_GOAL,
    usedToolThisPrompt: true,
    toolResultsThisPrompt: workDone,
    assistantText: '我读取了脚本，发现一处可疑的字符串拼接查询，继续分析。',
    forcedContinuesUsed: 0,
  });
  assert.equal(decision.continue, true);
  assert.equal(decision.reason, 'incomplete_no_deliverable');
  assert.ok(decision.hint && decision.hint.length > 0);
});

test('finishTurnDecision (b): model ended by asking the user a question → does NOT force', () => {
  const decision = finishTurnDecision({
    goalText: AUDIT_GOAL,
    usedToolThisPrompt: true,
    toolResultsThisPrompt: workDone,
    assistantText: '我分析了代码。需要你确认：要我继续扫描生产环境吗？',
    forcedContinuesUsed: 0,
  });
  assert.equal(decision.continue, false);
  assert.equal(decision.reason, 'asked_user_question');
});

test('finishTurnDecision (c): budget exhausted → stops cleanly', () => {
  const decision = finishTurnDecision({
    goalText: AUDIT_GOAL,
    usedToolThisPrompt: true,
    toolResultsThisPrompt: workDone,
    assistantText: '继续分析中。',
    forcedContinuesUsed: FORCED_CONTINUE_BUDGET,
  });
  assert.equal(decision.continue, false);
  assert.equal(decision.reason, 'budget_exhausted');
});

test('finishTurnDecision (d): simple Q&A (no tools, no deliverable-expected goal) → unaffected', () => {
  const plain = finishTurnDecision({ goalText: 'hello, introduce yourself', usedToolThisPrompt: false, toolResultsThisPrompt: [], assistantText: 'I am an assistant.' });
  assert.equal(plain.continue, false);
  assert.equal(plain.reason, 'no_deliverable_expected');
  // Even a deliverable-ish goal with no work done this turn is a plain answer.
  const noWork = finishTurnDecision({ goalText: AUDIT_GOAL, usedToolThisPrompt: false, toolResultsThisPrompt: [], assistantText: 'Here is my view.' });
  assert.equal(noWork.continue, false);
  assert.equal(noWork.reason, 'no_work_simple_qa');
});

test('finishTurnDecision (e): cancellation always wins over a would-be continue', () => {
  const decision = finishTurnDecision({
    goalText: AUDIT_GOAL,
    usedToolThisPrompt: true,
    toolResultsThisPrompt: workDone,
    assistantText: '继续分析中。',
    forcedContinuesUsed: 0,
    cancelled: true,
  });
  assert.equal(decision.continue, false);
  assert.equal(decision.reason, 'cancelled');
});

test('finishTurnDecision: a produced deliverable (record_finding / report) is not force-continued', () => {
  const finding = finishTurnDecision({ goalText: AUDIT_GOAL, usedToolThisPrompt: true, toolResultsThisPrompt: [{ name: 'org.app::record_finding', ok: true }], assistantText: '完成。' });
  assert.equal(finding.continue, false);
  assert.equal(finding.reason, 'deliverable_produced');
  const report = finishTurnDecision({ goalText: AUDIT_GOAL, usedToolThisPrompt: true, toolResultsThisPrompt: [{ name: 'workspace_write', ok: true, arguments: { path: 'reports/findings.md' } }], assistantText: '报告已写。' });
  assert.equal(report.continue, false);
  assert.equal(report.reason, 'deliverable_produced');
});

test('finishTurnDecision: read-only turn, non-quality mode, and scope boundary each decline a force', () => {
  const base = { goalText: AUDIT_GOAL, usedToolThisPrompt: true, toolResultsThisPrompt: workDone, assistantText: 'x' };
  assert.equal(finishTurnDecision({ ...base, readOnlyTurn: true }).reason, 'read_only_turn');
  assert.equal(finishTurnDecision({ ...base, answerMode: 'fast' }).reason, 'not_quality_mode');
  assert.equal(finishTurnDecision({ ...base, scopeBoundary: true }).reason, 'scope_boundary');
});

test('finishTurnDecision: writing a helper script (not a report) is work, not a deliverable → still forces', () => {
  const decision = finishTurnDecision({
    goalText: AUDIT_GOAL,
    usedToolThisPrompt: true,
    toolResultsThisPrompt: [{ name: 'workspace_write', ok: true, arguments: { path: 'tools/probe.mjs' } }],
    assistantText: '脚本已写好，但还没跑出结论。',
  });
  assert.equal(decision.continue, true);
  assert.equal(decision.reason, 'incomplete_no_deliverable');
});

test('deliverable/analysis/question/announcement helpers', () => {
  assert.equal(deliverableExpected(AUDIT_GOAL), true);
  assert.equal(deliverableExpected('Review saved references'), false);
  assert.equal(didAnalysisWork([{ name: 'org::workspace_run', ok: true }]), true);
  assert.equal(didAnalysisWork([{ name: 'inventory_stats', ok: true, advisory: true }]), false);
  assert.equal(producedDeliverable([{ name: 'workspace_write', ok: true, arguments: { path: 'src/x.mjs' } }]), false);
  assert.equal(looksLikeQuestionToUser('Should I proceed?'), true);
  assert.equal(looksLikeQuestionToUser('I found three issues and logged them.'), false);
  // null baseline (first step) never announces.
  assert.equal(toolSetAnnouncement(null, ['a', 'b']), null);
  const ann = toolSetAnnouncement(['workspace_read'], ['workspace_read', 'workspace_run', 'record_finding']);
  assert.deepEqual(ann.added, ['workspace_run', 'record_finding']);
  assert.deepEqual(ann.executionAdded, ['workspace_run', 'record_finding']);
  assert.ok(ann.message.includes('workspace_run'));
  // exclude removes a name already announced via another path this step.
  const excluded = toolSetAnnouncement(['workspace_read'], ['workspace_read', 'app::record_finding'], { exclude: ['app::record_finding'] });
  assert.equal(excluded, null);
});

test('latestDeliverableWritePath: finds a report-like write, ignores a non-report write and record_finding', () => {
  assert.equal(latestDeliverableWritePath([{ name: 'workspace_write', ok: true, arguments: { path: 'reports/findings.md' } }]), 'reports/findings.md');
  assert.equal(latestDeliverableWritePath([{ name: 'workspace_write', ok: true, arguments: { path: 'src/x.mjs' } }]), '');
  assert.equal(latestDeliverableWritePath([{ name: 'org.app::record_finding', ok: true, arguments: { title: 'x' } }]), '');
  // Picks the LAST matching write, not the first.
  const path = latestDeliverableWritePath([
    { name: 'workspace_write', ok: true, arguments: { path: 'burp_out/decrypted_report.md' } },
    { name: 'workspace_read', ok: true, arguments: { path: 'a.mjs' } },
    { name: 'workspace_edit', ok: true, arguments: { path: 'burp_out/decrypted_report_v2.md' } },
  ]);
  assert.equal(path, 'burp_out/decrypted_report_v2.md');
});

test('deliverableSelfCheckDecision: a freshly written report forces exactly one self-check pass naming the path', () => {
  const decision = deliverableSelfCheckDecision({
    toolResultsThisPrompt: [{ name: 'workspace_write', ok: true, arguments: { path: 'burp_out/decrypted_report.md' } }],
    assistantText: '报告已写好。',
  });
  assert.equal(decision.continue, true);
  assert.equal(decision.reason, 'self_check_fresh_deliverable');
  assert.equal(decision.path, 'burp_out/decrypted_report.md');
  assert.ok(decision.hint.includes('burp_out/decrypted_report.md'));
  assert.ok(decision.hint.includes('workspace_edit'));
});

test('deliverableSelfCheckDecision: a correction-sounding reply with an earlier deliverable path forces a check against THAT file', () => {
  const decision = deliverableSelfCheckDecision({
    toolResultsThisPrompt: [], // no write this turn — this is a later, separate turn
    assistantText: '口径修正：queryKey 实际返回的是 128 字节二进制块，key/iv 是从前端逻辑反推出来的。',
    lastDeliverablePath: 'burp_out/decrypted_report.md',
  });
  assert.equal(decision.continue, true);
  assert.equal(decision.reason, 'self_check_correction_signal');
  assert.equal(decision.path, 'burp_out/decrypted_report.md');
});

test('deliverableSelfCheckDecision: no deliverable and no correction signal → no check needed', () => {
  assert.equal(deliverableSelfCheckDecision({ toolResultsThisPrompt: [], assistantText: '好的，我继续。' }).reason, 'no_check_needed');
  // A correction-sounding reply with nothing to check it against is also a no-op.
  assert.equal(deliverableSelfCheckDecision({ toolResultsThisPrompt: [], assistantText: '口径修正：其实是这样的。', lastDeliverablePath: '' }).reason, 'no_check_needed');
});

test('deliverableSelfCheckDecision: budget exhaustion, already-checked, non-quality mode and cancellation each decline', () => {
  const fresh = { toolResultsThisPrompt: [{ name: 'workspace_write', ok: true, arguments: { path: 'r.md' } }], assistantText: 'x' };
  assert.equal(deliverableSelfCheckDecision({ ...fresh, forcedContinuesUsed: FORCED_CONTINUE_BUDGET }).reason, 'budget_exhausted');
  assert.equal(deliverableSelfCheckDecision({ ...fresh, selfCheckDone: true }).reason, 'already_checked');
  assert.equal(deliverableSelfCheckDecision({ ...fresh, answerMode: 'fast' }).reason, 'not_quality_mode');
  assert.equal(deliverableSelfCheckDecision({ ...fresh, cancelled: true }).reason, 'cancelled');
});

test('buildSuggestedNext: exactly two options, continue carries the next open intent as a concrete prompt', () => {
  const plain = buildSuggestedNext(emptyAgentState());
  assert.deepEqual(plain.map(item => item.id), ['continue_current', 'end_topic']);
  assert.equal(plain[0].prompt, undefined);
  const withIntent = buildSuggestedNext(setGoal(emptyAgentState(), '有多少资产'));
  assert.ok(withIntent[0].prompt?.includes('需工具核实'));
});

// ---------------------------------------------------------------------------
// Part 2 — real provider turn-loop integration (stubbed model + Host)
// ---------------------------------------------------------------------------

function chatToolCall(name, args, index) {
  const delta = { tool_calls: [{ index: 0, id: `call-${index}`, type: 'function', function: { name: providerToolName(name), arguments: JSON.stringify(args) } }] };
  return new Response(`data: ${JSON.stringify({ choices: [{ delta, finish_reason: 'tool_calls' }] })}\n\ndata: [DONE]\n\n`);
}
function chatText(text) {
  return new Response(`data: ${JSON.stringify({ choices: [{ delta: { content: text }, finish_reason: 'stop' }] })}\n\ndata: [DONE]\n\n`);
}
const toolObj = name => ({ name, inputSchema: { type: 'object' } });

// Build a runtime whose model is `script(bodies)` and whose Host lists `tools`
// (array or thunk) and records every tools/call in `executed`.
function harness({ tools, script, onCall }) {
  const bodies = [], executed = [];
  const runtime = new OpenAiCompatibleRuntime({
    config: { authentication: 'none', provider: 'mock', baseUrl: 'http://127.0.0.1:1', model: 'mock' },
    maxSteps: 8,
    fetch: async (_url, init) => { bodies.push(JSON.parse(init.body)); return script(bodies); },
  });
  runtime.bindHost({
    mcp: async (_token, _conversation, method, params) => {
      if (method === 'tools/list') return { tools: (typeof tools === 'function' ? tools() : tools).map(toolObj) };
      executed.push(params);
      const value = onCall ? await onCall(params) : { ok: true };
      return { content: [{ type: 'text', text: JSON.stringify(value ?? { ok: true }) }], structuredContent: value ?? { ok: true } };
    },
  });
  return { runtime, bodies, executed };
}
const mcpServers = [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: 'fixture' }, { name: 'OVIRAPTOR_CONVERSATION', value: 'conversation' }] }];

test('loop (a+c): autonomous audit with no deliverable forces one more turn up to the budget, then stops cleanly', async () => {
  const { runtime, bodies } = harness({
    tools: ['workspace_read', 'workspace_write', 'record_finding'],
    script: steps => {
      if (steps.length === 1) return chatToolCall('workspace_read', { path: 'a.mjs' }, 1);
      return chatText('我读取了脚本，发现一处可疑的拼接查询，继续分析中。'); // no tool, no deliverable, no question
    },
  });
  const { sessionId } = await runtime.newSession({ mcpServers });
  try {
    const result = await runtime.prompt({ sessionId, prompt: AUDIT_GOAL, userText: AUDIT_GOAL });
    // 1 tool step + 2 forced continues + 1 final = 4 model requests.
    assert.equal(bodies.length, 4);
    assert.equal(result.stopReason, 'end_turn');
    assert.equal(result.metrics.forcedContinues, FORCED_CONTINUE_BUDGET);
    assert.equal(result.metrics.forcedContinueReason, 'incomplete_no_deliverable');
  } finally { await runtime.close(); }
});

test('loop (b): a turn that ends by asking the user a question is NOT force-continued', async () => {
  const { runtime, bodies } = harness({
    tools: ['workspace_read', 'record_finding'],
    script: steps => (steps.length === 1
      ? chatToolCall('workspace_read', { path: 'a.mjs' }, 1)
      : chatText('我分析了代码。需要你确认：要我继续扫描生产环境吗？')),
  });
  const { sessionId } = await runtime.newSession({ mcpServers });
  try {
    const result = await runtime.prompt({ sessionId, prompt: AUDIT_GOAL, userText: AUDIT_GOAL });
    assert.equal(bodies.length, 2); // tool step + the ending question, no forced continue
    assert.equal(result.stopReason, 'end_turn');
    assert.equal(result.metrics.forcedContinues, 0);
  } finally { await runtime.close(); }
});

test('loop (b2): producing a deliverable (record_finding) ends the turn without forcing, and carries suggestedNext', async () => {
  const { runtime, bodies, executed } = harness({
    tools: ['workspace_read', 'record_finding'],
    script: steps => (steps.length === 1
      ? chatToolCall('record_finding', { title: 'sqli', severity: 'high' }, 1)
      : chatText('已记录发现，分析完成。')),
  });
  const { sessionId } = await runtime.newSession({ mcpServers });
  try {
    const result = await runtime.prompt({ sessionId, prompt: AUDIT_GOAL, userText: AUDIT_GOAL });
    // record_finding has no report file to self-check against, so this is unchanged.
    assert.equal(bodies.length, 2);
    assert.equal(executed[0].name, 'record_finding');
    assert.equal(result.metrics.forcedContinues, 0);
    assert.ok(Array.isArray(result.suggestedNext) && result.suggestedNext.length === 2, 'a truly-finished deliverable turn carries suggestedNext');
    assert.deepEqual(result.suggestedNext.map(item => item.id), ['continue_current', 'end_topic']);
  } finally { await runtime.close(); }
});

test('loop (b3): writing a report file forces one self-check pass before the turn ends, then still carries suggestedNext', async () => {
  const { runtime, bodies, executed } = harness({
    tools: ['workspace_write', 'record_finding'],
    script: steps => {
      if (steps.length === 1) return chatToolCall('workspace_write', { path: 'reports/findings.md', content: 'queryKey returns the AES key/iv directly.' }, 1);
      return chatText('报告已写好，分析完成。'); // self-check pass: no tool call, no correction — clean end next.
    },
  });
  const { sessionId } = await runtime.newSession({ mcpServers });
  try {
    const result = await runtime.prompt({ sessionId, prompt: AUDIT_GOAL, userText: AUDIT_GOAL });
    // write + 1 forced self-check pass + the final clean end = 3 model requests.
    assert.equal(bodies.length, 3);
    assert.equal(executed[0].name, 'workspace_write');
    assert.equal(result.metrics.forcedContinues, 1);
    assert.equal(result.metrics.forcedContinueReason, 'self_check_fresh_deliverable');
    // The self-check hint must reference the written path so the model knows what to re-open.
    const secondRequestText = JSON.stringify(bodies[1].messages);
    assert.ok(secondRequestText.includes('reports/findings.md'));
    assert.ok(Array.isArray(result.suggestedNext) && result.suggestedNext.length === 2);
  } finally { await runtime.close(); }
});

test('loop (d): simple Q&A with no tools is unaffected by the finishTurn policy', async () => {
  const { runtime, bodies } = harness({
    tools: [],
    script: () => chatText('I am the Oviraptor assistant.'),
  });
  const { sessionId } = await runtime.newSession({ mcpServers });
  try {
    const result = await runtime.prompt({ sessionId, prompt: 'hello, introduce yourself', userText: 'hello, introduce yourself' });
    assert.equal(bodies.length, 1);
    assert.equal(result.stopReason, 'end_turn');
    assert.equal(result.metrics.forcedContinues, 0);
  } finally { await runtime.close(); }
});

test('loop (e): cancellation during a tool call wins — no forced continue, stopReason cancelled', async () => {
  let sid = null;
  const h = harness({
    tools: ['workspace_read', 'record_finding'],
    script: () => chatToolCall('workspace_read', { path: 'a.mjs' }, 1),
    onCall: async () => { if (sid) await h.runtime.cancel(sid); return { ok: true }; },
  });
  const { runtime, bodies } = h;
  const { sessionId } = await runtime.newSession({ mcpServers });
  sid = sessionId;
  try {
    const result = await runtime.prompt({ sessionId, prompt: AUDIT_GOAL, userText: AUDIT_GOAL });
    assert.equal(result.stopReason, 'cancelled');
    assert.equal(bodies.length, 1); // only the first model request; policy never ran
    assert.equal(result.metrics?.forcedContinues ?? 0, 0);
  } finally { await runtime.close(); }
});

test('loop (Task 2): a mid-turn tool-set grant is announced to the model before the next request', async () => {
  let listCalls = 0;
  const { runtime, bodies } = harness({
    tools: () => { listCalls += 1; return listCalls === 1 ? ['workspace_read'] : ['workspace_read', 'workspace_run', 'record_finding']; },
    script: steps => (steps.length === 1
      ? chatToolCall('workspace_read', { path: 'a.mjs' }, 1)
      // End with a question so Task 1 does not also force a continue here.
      : chatText('读完了。需要你确认：要我继续吗？')),
  });
  const { sessionId } = await runtime.newSession({ mcpServers });
  try {
    const result = await runtime.prompt({ sessionId, prompt: AUDIT_GOAL, userText: AUDIT_GOAL });
    assert.equal(result.metrics.toolSetAnnouncements, 1);
    // The announcement must have been delivered to the model in request #2.
    const secondTurnText = JSON.stringify(bodies[1].messages);
    assert.ok(secondTurnText.includes('workspace_run'), 'second request names the newly-available workspace_run');
    assert.ok(secondTurnText.includes('现在可用'), 'second request carries the Host announcement');
  } finally { await runtime.close(); }
});
