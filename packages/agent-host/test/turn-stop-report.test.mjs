import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildTurnStopReport } from '../src/turn-stop-report.mjs';

// A stall report on a workspace-heavy turn (write a script, run it, repeat)
// previously fell back to the generic "其他业务操作/读取" placeholder for
// every single line, because `labels` in turn-stop-report.mjs never covered
// the workspace_* tools — by far the most commonly used ones in any real
// analysis/pentest turn. That made a stall report look content-free exactly
// when the user most needed to see what the agent was stuck doing.
test('buildTurnStopReport labels common workspace tools instead of falling back to generic text', () => {
  const taskId = 't1', conversationId = 'c1', runId = 'r1', messageId = 'm1';
  const evidence = [
    { id: 'e1', runId, status: 'completed', readOnly: false, toolName: 'org.oviraptor.workspace__workspace_write' },
    { id: 'e2', runId, status: 'failed', readOnly: false, toolName: 'org.oviraptor.workspace__workspace_run' },
    { id: 'e3', runId, status: 'completed', readOnly: true, toolName: 'org.oviraptor.workspace__workspace_grep' },
  ];
  const task = {
    id: taskId, conversationId, evidence,
    runs: [{ id: runId, assistantMessageId: messageId }],
    outcomeUnknown: false,
  };
  const run = { id: runId, taskId, conversationId, message: { id: messageId } };
  const report = buildTurnStopReport(task, run, 'tool_loop_stalled', new Map());
  assert.ok(report);
  assert.ok(report.text.includes('写入文件'), 'workspace_write should be labeled, not fall back to generic text');
  assert.ok(report.text.includes('脚本执行'), 'workspace_run should be labeled, not fall back to generic text');
  assert.ok(report.text.includes('工作区搜索'), 'workspace_grep should be labeled, not fall back to generic text');
  assert.ok(!report.text.includes('其他业务操作'), 'no evidence line should need the generic fallback for these known tools');
  assert.ok(!report.text.includes('其他业务读取'), 'no evidence line should need the generic fallback for these known tools');
});

// The fallback itself must still work for a genuinely unknown tool name —
// this is a labeling gap fix, not a claim that every tool is now covered.
test('buildTurnStopReport still falls back to generic text for an unlabeled tool', () => {
  const taskId = 't2', conversationId = 'c2', runId = 'r2', messageId = 'm2';
  const evidence = [{ id: 'e1', runId, status: 'completed', readOnly: false, toolName: 'org.oviraptor.somepkg__totally_unknown_tool' }];
  const task = { id: taskId, conversationId, evidence, runs: [{ id: runId, assistantMessageId: messageId }], outcomeUnknown: false };
  const run = { id: runId, taskId, conversationId, message: { id: messageId } };
  const report = buildTurnStopReport(task, run, 'tool_loop_stalled', new Map());
  assert.ok(report);
  assert.ok(report.text.includes('其他业务操作'));
});
