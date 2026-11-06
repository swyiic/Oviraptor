import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { autoReactEnabled, decideReaction, reactionStats, ensureAutoReactSchema, reactionCandidate } from '../src/scan-autoreact.mjs';

const db = () => { const d = new DatabaseSync(':memory:'); ensureAutoReactSchema(d); return d; };

test('the switch is off unless explicitly enabled', () => {
  assert.equal(autoReactEnabled({}), false);
  assert.equal(autoReactEnabled({ OVIRAPTOR_AGENT_AUTOREACT: '0' }), false);
  assert.equal(autoReactEnabled({ OVIRAPTOR_AGENT_AUTOREACT: 'no' }), false);
  assert.equal(autoReactEnabled({ OVIRAPTOR_AGENT_AUTOREACT: 'yes' }), false);
  assert.equal(autoReactEnabled({ OVIRAPTOR_AGENT_AUTOREACT: '1' }), true);
  assert.equal(autoReactEnabled({ OVIRAPTOR_AGENT_AUTOREACT: 'true' }), true);
  assert.equal(autoReactEnabled({ OVIRAPTOR_AGENT_AUTOREACT: 'TRUE' }), true);
});

test('the same event reacts at most once (dedup across re-pull / restart)', () => {
  const d = db();
  const p = { taskId: 't', attemptNumber: 1, eventId: 'e-1' };
  assert.equal(decideReaction(d, p).react, true);
  assert.equal(decideReaction(d, p).react, false);
  assert.equal(decideReaction(d, p).reason, 'already_reacted');
  assert.equal(reactionStats(d, 't', 1).total, 1, 'a re-seen event does not spend budget again');
});

test('the per-attempt budget trips the circuit and silences the attempt', () => {
  const d = db();
  const env = { OVIRAPTOR_AGENT_AUTOREACT_MAX: '3', OVIRAPTOR_AGENT_AUTOREACT_RATE: '100' };
  for (let i = 0; i < 3; i++) assert.equal(decideReaction(d, { taskId: 't', attemptNumber: 1, eventId: `e-${i}`, env }).react, true);
  const over = decideReaction(d, { taskId: 't', attemptNumber: 1, eventId: 'e-x', env });
  assert.equal(over.react, false);
  assert.equal(over.reason, 'attempt_budget_exhausted');
  // Once tripped, even a brand-new event stays silent.
  const after = decideReaction(d, { taskId: 't', attemptNumber: 1, eventId: 'e-y', env });
  assert.equal(after.react, false);
  assert.match(after.reason, /^circuit_open:/);
  assert.equal(reactionStats(d, 't', 1).circuitOpen, true);
});

test('a burst of reactions in a short window breaks the feedback loop', () => {
  const d = db();
  const env = { OVIRAPTOR_AGENT_AUTOREACT_MAX: '100', OVIRAPTOR_AGENT_AUTOREACT_RATE: '3' };
  const now = 1_000_000;
  for (let i = 0; i < 3; i++) assert.equal(decideReaction(d, { taskId: 't', attemptNumber: 1, eventId: `e-${i}`, now, env }).react, true);
  const tripped = decideReaction(d, { taskId: 't', attemptNumber: 1, eventId: 'e-burst', now, env });
  assert.equal(tripped.react, false);
  assert.equal(tripped.reason, 'rate_circuit_tripped');
});

test('different attempts and tasks have independent budgets', () => {
  const d = db();
  const env = { OVIRAPTOR_AGENT_AUTOREACT_MAX: '1', OVIRAPTOR_AGENT_AUTOREACT_RATE: '100' };
  assert.equal(decideReaction(d, { taskId: 't', attemptNumber: 1, eventId: 'a', env }).react, true);
  assert.equal(decideReaction(d, { taskId: 't', attemptNumber: 1, eventId: 'b', env }).react, false);
  assert.equal(decideReaction(d, { taskId: 't', attemptNumber: 2, eventId: 'c', env }).react, true, 'next attempt is fresh');
  assert.equal(decideReaction(d, { taskId: 'u', attemptNumber: 1, eventId: 'd', env }).react, true, 'other task is fresh');
});

test('malformed candidates never react', () => {
  const d = db();
  assert.equal(decideReaction(d, { taskId: '', attemptNumber: 1, eventId: 'e' }).react, false);
  assert.equal(decideReaction(d, { taskId: 't', attemptNumber: 1, eventId: '' }).react, false);
  assert.equal(decideReaction(d, { taskId: 't', attemptNumber: 1.5, eventId: 'e' }).react, false);
});

test('only Nest-origin events react, and only when the switch is on', () => {
  const on = { OVIRAPTOR_AGENT_AUTOREACT: '1' };
  const ok = { kind: 'assignment_state', conversationId: 'c', taskId: 't', eventId: 'e', attemptNumber: 1 };
  // switch off → never eligible, whatever the event
  assert.equal(reactionCandidate(ok, {}).eligible, false);
  assert.equal(reactionCandidate(ok, {}).reason, 'disabled');
  // on + a Nest event → eligible
  assert.equal(reactionCandidate(ok, on).eligible, true);
  assert.equal(reactionCandidate({ ...ok, kind: 'task_result' }, on).eligible, true);
  // the agent's own command outcomes never react (no chasing its own tail)
  for (const kind of ['execution', 'supplement_outcome', 'block', 'verification']) {
    assert.equal(reactionCandidate({ ...ok, kind }, on).reason, 'kind_not_reactable');
  }
  // missing ids → not eligible
  assert.equal(reactionCandidate({ ...ok, taskId: '' }, on).reason, 'incomplete');
  assert.equal(reactionCandidate({ ...ok, attemptNumber: undefined }, on).reason, 'incomplete');
});
