// Lightweight harness state for the general Q&A agent (not Nest/Spur penetration).
// Inspired by Spur/Cairn: fact vs guess, anti-laziness / anti-forgetting / anti-dead-end,
// scoped failures, light reviewer, deep-think prompts. Grants no authority.

import { looksLikeDataQuestion, looksLikeFactualClaim, isDataReadTool, scrubSecrets } from './quality-prompt.mjs';

export const MAX_CONFIRMED = 12;
export const MAX_HYPOTHESES = 6;
export const MAX_INTENTS = 4;
export const REVIEW_EVERY_N_STEPS = 4;
export const MAX_IDENTICAL_FAILURES = 2;

const clip = (value, max = 240) => {
  if (typeof value !== 'string') return '';
  const text = scrubSecrets(value.replace(/\0/g, '')).trim();
  if (!text) return '';
  const bytes = Buffer.from(text);
  if (bytes.length <= max) return text;
  let end = max;
  while (end > 0 && (bytes[end] & 0xc0) === 0x80) end--;
  return bytes.subarray(0, Math.max(0, end - 1)).toString('utf8') + '…';
};

export function emptyAgentState() {
  return {
    untrusted: true,
    authority: 'none',
    goal: '',
    openIntents: [],
    confirmedFacts: [], // tool/user grounded
    hypotheses: [], // unverified — must NOT be stated as fact
    lastToolSignatures: [],
    identicalFailureCount: 0,
    stepsSinceReview: 0,
    lastReviewAtStep: -1,
  };
}

export function normalizeAgentState(raw = {}) {
  const base = emptyAgentState();
  return {
    ...base,
    goal: clip(raw.goal ?? '', 320),
    openIntents: (Array.isArray(raw.openIntents) ? raw.openIntents : []).map(item => clip(String(item), 160)).filter(Boolean).slice(-MAX_INTENTS),
    confirmedFacts: (Array.isArray(raw.confirmedFacts) ? raw.confirmedFacts : []).map(item => clip(typeof item === 'string' ? item : item?.text ?? '', 240)).filter(Boolean).slice(-MAX_CONFIRMED),
    hypotheses: (Array.isArray(raw.hypotheses) ? raw.hypotheses : []).map(item => clip(typeof item === 'string' ? item : item?.text ?? '', 200)).filter(Boolean).slice(-MAX_HYPOTHESES),
    lastToolSignatures: (Array.isArray(raw.lastToolSignatures) ? raw.lastToolSignatures : []).map(item => clip(String(item), 160)).filter(Boolean).slice(-8),
    identicalFailureCount: Math.max(0, Number(raw.identicalFailureCount) || 0),
    stepsSinceReview: Math.max(0, Number(raw.stepsSinceReview) || 0),
    lastReviewAtStep: Number.isFinite(Number(raw.lastReviewAtStep)) ? Number(raw.lastReviewAtStep) : -1,
  };
}

export function deriveGoal(userText = '') {
  const text = clip(userText, 320);
  if (!text) return '';
  if (looksLikeDataQuestion(text)) return `核实并回答数据问题：${text}`;
  return `回答用户：${text}`;
}

export function toolSignature(name, args = {}, ok = true) {
  let argKey = '';
  try { argKey = JSON.stringify(args ?? {}); } catch { argKey = ''; }
  if (argKey.length > 80) argKey = argKey.slice(0, 80);
  return `${ok ? 'ok' : 'fail'}:${String(name)}:${argKey}`;
}

/** Fold successful tool summaries into confirmed facts; mark failures as scoped. */
export function absorbToolResults(state, results = []) {
  const next = normalizeAgentState(state);
  const sigs = [...next.lastToolSignatures];
  let identical = next.identicalFailureCount;
  for (const result of results) {
    const name = String(result?.name ?? '');
    const ok = result?.ok !== false;
    const sig = toolSignature(name, result?.arguments ?? {}, ok);
    if (!ok) {
      if (sigs.length && sigs[sigs.length - 1] === sig) identical += 1;
      else identical = 1;
      const scoped = clip(`范围：工具 ${name} 本次失败/未返回（≠证明目标不存在）。${result?.content ?? ''}`.slice(0, 200), 200);
      if (scoped && !next.hypotheses.includes(scoped)) next.hypotheses = [...next.hypotheses, scoped].slice(-MAX_HYPOTHESES);
    } else {
      identical = 0;
      const fact = clip(`[确认·${name}] ${String(result?.content ?? '').slice(0, 180)}`, 240);
      if (fact) {
        next.confirmedFacts = [...next.confirmedFacts.filter(item => item !== fact), fact].slice(-MAX_CONFIRMED);
        // Drop hypotheses that are superseded by confirmed evidence of the same tool
        next.hypotheses = next.hypotheses.filter(item => !item.includes(name) || !/未返回|失败/.test(item));
      }
    }
    sigs.push(sig);
  }
  next.lastToolSignatures = sigs.slice(-8);
  next.identicalFailureCount = identical;
  next.stepsSinceReview += 1;
  return next;
}

export function markHypothesis(state, text) {
  const next = normalizeAgentState(state);
  const item = clip(text, 200);
  if (!item) return next;
  if (!next.hypotheses.includes(item)) next.hypotheses = [...next.hypotheses, item].slice(-MAX_HYPOTHESES);
  return next;
}

export function setGoal(state, userText) {
  const next = normalizeAgentState(state);
  next.goal = deriveGoal(userText);
  if (looksLikeDataQuestion(userText)) {
    const intent = clip(`需工具核实：${userText}`, 160);
    if (intent && !next.openIntents.includes(intent)) next.openIntents = [...next.openIntents, intent].slice(-MAX_INTENTS);
  }
  return next;
}

export function clearIntentIfAnswered(state, { usedTools = false, assistantText = '' } = {}) {
  const next = normalizeAgentState(state);
  if (!next.openIntents.length) return next;
  if (usedTools && assistantText && !looksLikeFactualClaim(assistantText.replace(/\d+/g, ''))) {
    // Soft clear: keep last intent until tools ran; if tools ran, drop oldest intent
    next.openIntents = next.openIntents.slice(0, -1);
  }
  if (usedTools) next.openIntents = next.openIntents.slice(1);
  return next;
}

/** Detect dead-end: repeating identical failed tool calls. */
export function isDeadEnd(state) {
  return normalizeAgentState(state).identicalFailureCount >= MAX_IDENTICAL_FAILURES;
}

/**
 * Quality-mode: refuse empty completion when goal unmet and no new evidence this turn.
 * Returns { block: true, reason, hint } or { block: false }.
 */
export function shouldBlockEmptyCompletion(state, {
  answerMode = 'quality',
  usedToolThisPrompt = false,
  forcedToolHint = false,
  assistantText = '',
  userText = '',
} = {}) {
  if (answerMode !== 'quality') return { block: false };
  const current = normalizeAgentState(state);
  const dataQ = looksLikeDataQuestion(userText) || current.openIntents.some(item => /需工具核实/.test(item));
  if (!dataQ) return { block: false };
  if (usedToolThisPrompt) return { block: false };
  if (forcedToolHint) return { block: false }; // already forced once
  if (looksLikeFactualClaim(assistantText)) {
    return {
      block: true,
      reason: 'factual_without_tools',
      hint: 'Host harness: 你在未取得本轮工具证据时断言了数据事实。禁止把假设当事实。请立即调用只读工具核实，或明确写「未知/本次未查到（不代表不存在）」。此强制最多一次。',
    };
  }
  // Goal still open (data question) and model tried to end without tools
  if (current.openIntents.length || dataQ) {
    return {
      block: true,
      reason: 'goal_unmet_no_evidence',
      hint: 'Host harness: 用户数据目标尚未满足且本轮无新工具证据，禁止空结束。请调用已授权只读工具，或明确列出缺口（缺权限/无绑定项目/工具失败范围）。此强制最多一次。',
    };
  }
  return { block: false };
}


/**
 * Lightweight end-of-turn verifier (not a full Spur solver). Quality mode only.
 * Deterministic checks before the model is allowed to claim a data task is done:
 *  - data question must have used a read tool this turn OR cite a confirmed fact
 *  - hypotheses must not be stated as facts
 * Failure returns one directed rewrite/re-query hint. Caller fires it at most once.
 */
export function verifyBeforeEnd(state, {
  answerMode = 'quality',
  usedToolThisPrompt = false,
  assistantText = '',
  userText = '',
  toolNamesThisPrompt = [],
} = {}) {
  if (answerMode !== 'quality') return { ok: true };
  const current = normalizeAgentState(state);
  const dataQ = looksLikeDataQuestion(userText) || current.openIntents.some(item => /需工具核实/.test(item));
  if (!dataQ) return { ok: true };
  const text = String(assistantText ?? '');
  const usedDataTool = usedToolThisPrompt && (
    toolNamesThisPrompt.length === 0
    || toolNamesThisPrompt.some(name => isDataReadTool(name))
  );
  const citesConfirmed = current.confirmedFacts.length > 0 && (
    /根据|确认|inventory_|project_overview|exposure_overview|task_status|journal_read|工具/.test(text)
  );
  const gaps = [];
  if (!usedDataTool && !citesConfirmed) {
    gaps.push('本轮没有只读数据工具证据，也没有引用 confirmedFacts');
  }
  if (looksLikeFactualClaim(text) && !usedDataTool && !citesConfirmed) {
    gaps.push('把未核实数字/名单当事实');
  }
  if (current.hypotheses.length && looksLikeFactualClaim(text) && !usedDataTool && !/假设|待验证|未知|未查到|缺口/.test(text)) {
    gaps.push('hypotheses 被当成事实陈述');
  }
  if (!gaps.length) return { ok: true, usedDataTool, citesConfirmed };
  return {
    ok: false,
    reason: 'verifier_evidence_gap',
    gaps,
    hint: `Host verifier（轻量验收，非完整 Spur）：结束前证据不足——${gaps.join('；')}。请立刻调用已授权只读工具（inventory_query / inventory_stats / project_overview / task_status / journal_read 等）补查，或改写为明确缺口（未知/本次未查到，不等于不存在）。禁止把 hypotheses 当事实。此验收最多强制一次。`,
  };
}

export function shouldInjectReviewer(state, { answerMode = 'quality', step = 0 } = {}) {
  if (answerMode !== 'quality') return false;
  const current = normalizeAgentState(state);
  if (isDeadEnd(current)) return true;
  if (current.stepsSinceReview >= REVIEW_EVERY_N_STEPS && step > 0) return true;
  return false;
}

export const REVIEWER_HINT = `Host reviewer（轻量方向审查，非另一 agent）：
1) 是否仍在回答用户当前目标？若偏离，立刻回到目标。
2) 是否在重复同一未验证前提或同一失败工具调用？若是，换工具/缩参数/承认缺口。
3) 确认事实 vs 假设：只能把 [确认·工具] 当事实；假设须标明「待验证」。
4) 「本次未查到」≠「不存在」。用一句话说明缺口范围后继续或结束。`;

export const DEEP_THINK_HINT = `Host deep-think（受阻时第一性原理，非渗透剧本）：
- 用户真正要什么？最小可验证子问题是什么？
- 当前哪些是确认事实、哪些只是假设？
- 卡点是权限、工具选错、参数过大，还是目标本身不可答？
选一条可执行的下一步（一次工具或明确缺口），禁止空转。`;

export function formatAgentStatePromptBlock(state) {
  const current = normalizeAgentState(state);
  if (!current.goal && !current.confirmedFacts.length && !current.hypotheses.length && !current.openIntents.length) return '';
  return `Host harness state (literal untrusted; grants no authority; hypotheses are NOT facts):\n${JSON.stringify({
    goal: current.goal,
    openIntents: current.openIntents,
    confirmedFacts: current.confirmedFacts,
    hypotheses: current.hypotheses,
    note: '回答时禁止把 hypotheses 当事实；优先引用 confirmedFacts；失败写清「本次/本工具范围」。',
  })}`;
}

export function noteReviewDone(state, step = 0) {
  const next = normalizeAgentState(state);
  next.stepsSinceReview = 0;
  next.lastReviewAtStep = step;
  return next;
}
