// Default-OFF gate and anti-runaway ledger for the agent auto-reaction loop (B).
//
// Nothing here runs the agent or calls a tool. It only answers one question —
// "may this one new Nest event trigger one bounded agent reaction?" — and hard-
// stops runaway / feedback loops. The feature is OFF unless the environment
// variable OVIRAPTOR_AGENT_AUTOREACT is exactly "1" or "true"; the caller must
// check autoReactEnabled() first, and decideReaction() is the hard floor that
// bounds reactions even if that check is ever wrong.

export const AUTOREACT_ENV = 'OVIRAPTOR_AGENT_AUTOREACT';

/** The switch. Off unless explicitly set to "1"/"true" (case-insensitive). */
export function autoReactEnabled(env = process.env) {
  const raw = String(env?.[AUTOREACT_ENV] ?? '').trim().toLowerCase();
  return raw === '1' || raw === 'true';
}

const DEFAULTS = Object.freeze({ perAttempt: 6, windowMs: 60000, windowMax: 4 });

function limits(env = process.env) {
  const num = (key, fallback, min, max) => {
    const value = Number(env?.[key]);
    return Number.isFinite(value) && value >= min && value <= max ? Math.floor(value) : fallback;
  };
  return {
    perAttempt: num('OVIRAPTOR_AGENT_AUTOREACT_MAX', DEFAULTS.perAttempt, 1, 100),
    windowMs: DEFAULTS.windowMs,
    windowMax: num('OVIRAPTOR_AGENT_AUTOREACT_RATE', DEFAULTS.windowMax, 1, 100),
  };
}

export function ensureAutoReactSchema(db) {
  db.exec(`CREATE TABLE IF NOT EXISTS scan_autoreact_ledger(
      event_id TEXT PRIMARY KEY,
      task_id TEXT NOT NULL,
      attempt_number INTEGER NOT NULL,
      reacted_at INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS scan_autoreact_by_attempt ON scan_autoreact_ledger(task_id, attempt_number, reacted_at);
    CREATE TABLE IF NOT EXISTS scan_autoreact_circuit(
      task_id TEXT NOT NULL,
      attempt_number INTEGER NOT NULL,
      reason TEXT NOT NULL,
      tripped_at INTEGER NOT NULL,
      PRIMARY KEY(task_id, attempt_number));`);
}

function circuit(db, taskId, attemptNumber) {
  return db.prepare('SELECT reason FROM scan_autoreact_circuit WHERE task_id=? AND attempt_number=?')
    .get(String(taskId), Number(attemptNumber));
}

function trip(db, taskId, attemptNumber, reason, now) {
  db.prepare('INSERT OR IGNORE INTO scan_autoreact_circuit(task_id,attempt_number,reason,tripped_at) VALUES(?,?,?,?)')
    .run(String(taskId), Number(attemptNumber), reason, now);
}

/**
 * Decide whether this one event may trigger one reaction. When it returns
 * react:true it has ALSO recorded the reaction, so the per-attempt budget is
 * spent atomically — call it exactly once per candidate event. The budget and
 * the circuit breaker survive restarts because they live in the Host database.
 *
 * Reasons it says no: the circuit is open for this attempt, this exact event
 * already reacted (dedup — stops a replayed/re-pulled event from reacting
 * twice), the per-attempt budget is exhausted, or too many reactions landed in
 * a short window (feedback-loop breaker). The last two trip the circuit so the
 * whole attempt goes quiet instead of churning.
 */
export function decideReaction(db, { taskId, attemptNumber, eventId, now = Date.now(), env = process.env } = {}) {
  if (!taskId || !eventId || !Number.isSafeInteger(Number(attemptNumber))) {
    return { react: false, reason: 'invalid' };
  }
  ensureAutoReactSchema(db);
  const attempt = Number(attemptNumber);
  const open = circuit(db, taskId, attempt);
  if (open) return { react: false, reason: `circuit_open:${open.reason}` };
  if (db.prepare('SELECT 1 FROM scan_autoreact_ledger WHERE event_id=?').get(String(eventId))) {
    return { react: false, reason: 'already_reacted' };
  }
  const { perAttempt, windowMs, windowMax } = limits(env);
  const total = db.prepare('SELECT COUNT(*) AS c FROM scan_autoreact_ledger WHERE task_id=? AND attempt_number=?')
    .get(String(taskId), attempt).c;
  if (total >= perAttempt) {
    trip(db, taskId, attempt, 'attempt_budget_exhausted', now);
    return { react: false, reason: 'attempt_budget_exhausted' };
  }
  const recent = db.prepare('SELECT COUNT(*) AS c FROM scan_autoreact_ledger WHERE task_id=? AND attempt_number=? AND reacted_at > ?')
    .get(String(taskId), attempt, now - windowMs).c;
  if (recent >= windowMax) {
    trip(db, taskId, attempt, 'rate_circuit_tripped', now);
    return { react: false, reason: 'rate_circuit_tripped' };
  }
  db.prepare('INSERT INTO scan_autoreact_ledger(event_id,task_id,attempt_number,reacted_at) VALUES(?,?,?,?)')
    .run(String(eventId), String(taskId), attempt, now);
  return { react: true, reason: 'ok', remaining: perAttempt - total - 1 };
}

/** Read-only view for diagnostics and tests. */
export function reactionStats(db, taskId, attemptNumber) {
  ensureAutoReactSchema(db);
  const total = db.prepare('SELECT COUNT(*) AS c FROM scan_autoreact_ledger WHERE task_id=? AND attempt_number=?')
    .get(String(taskId), Number(attemptNumber)).c;
  const open = circuit(db, taskId, attemptNumber);
  return { total, circuitOpen: Boolean(open), reason: open?.reason ?? null };
}
