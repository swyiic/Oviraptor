import { hostError } from './store.mjs';

const timingKeys = ['coldStartMs', 'firstTextMs', 'firstToolMs', 'totalMs', 'hostTotalMs'];
const timing = value => typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null;

/** A bounded observation window. Synthetic/custom runs never establish official CLI performance. */
export function runtimeMetrics(store, limit = 100) {
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > 1000) throw hostError('host_metrics_limit_invalid');
  const total = store.db.prepare('SELECT COUNT(*) AS n FROM runtime_metrics').get().n;
  const rows = store.db.prepare('SELECT run_id,metrics,created_at FROM runtime_metrics ORDER BY rowid DESC LIMIT ?').all(limit);
  const items = rows.map(row => {
    let value; try { value = JSON.parse(row.metrics); } catch { value = {}; }
    return { runId: row.run_id, recordedAt: row.created_at,
      evidence: ['synthetic-or-custom', 'official-cli-unverified'].includes(value.runtimeEvidence) ? value.runtimeEvidence : 'unknown',
      outcome: ['completed', 'failed', 'cancelled', 'refused', 'incomplete'].includes(value.outcome) ? value.outcome : 'unknown',
      outcomeUnknown: value.outcomeUnknown === true,
      timings: Object.fromEntries(timingKeys.map(key => [key, timing(value[key])])) };
  });
  const groups = [...new Set(items.map(item => item.evidence))].map(evidence => {
    const runs = items.filter(item => item.evidence === evidence);
    const successes = runs.filter(item => item.outcome === 'completed' && !item.outcomeUnknown).length;
    return { evidence, attempts: runs.length, successes, successRate: successes / runs.length,
      unknownOutcomes: runs.filter(item => item.outcomeUnknown).length,
      timings: Object.fromEntries(timingKeys.map(key => {
        const observations = runs.map(item => item.timings[key]).filter(value => value !== null);
        return [key, { observations: observations.length, mean: observations.length ? observations.reduce((a, b) => a + b, 0) / observations.length : null }];
      })) };
  });
  return { total, returned: items.length, truncated: total > items.length, limit, order: 'newest-first',
    successDefinition: 'completed with known outcome / all terminal attempts in this returned evidence group',
    officialPerformanceVerified: false, groups, items };
}
