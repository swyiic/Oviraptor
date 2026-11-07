// Lightweight harness state for the general Q&A agent (not Nest/Spur penetration).
// Inspired by Spur/Cairn: fact vs guess, anti-laziness / anti-forgetting / anti-dead-end,
// scoped failures, light reviewer, deep-think prompts. Grants no authority.

import { createHash } from 'node:crypto';
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
  let end = Math.max(0, max - 3);
  while (end > 0 && (bytes[end] & 0xc0) === 0x80) end--;
  return bytes.subarray(0, end).toString('utf8') + '…';
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

function argumentFingerprint(args) {
  let encoded = '';
  try { encoded = JSON.stringify(args ?? {}); } catch { encoded = ''; }
  // 16 hex digits. A full sha256 is 64 hex and scrubSecrets rewrites that to the same placeholder.
  return createHash('sha256').update(encoded).digest('hex').slice(0, 16);
}

export function toolSignature(name, args = {}, ok = true) {
  return `${ok ? 'ok' : 'fail'}:${String(name)}#${argumentFingerprint(args)}`;
}

/** Fold successful tool summaries into confirmed facts; mark failures as scoped. */
export function absorbToolResults(state, results = []) {
  const next = normalizeAgentState(state);
  const sigs = [...next.lastToolSignatures];
  let identical = next.identicalFailureCount;
  for (const result of results) {
    const name = String(result?.name ?? '');
    // Host task metadata and child reports are advisory, including when another
    // caller bypasses the provider's evidence filter.
    if (result?.advisory === true || name.startsWith('host_task_')) continue;
    const ok = result?.ok === true && result?.isError !== true;
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
  toolResultsThisPrompt,
} = {}) {
  if (answerMode !== 'quality') return { ok: true };
  const current = normalizeAgentState(state);
  const dataQ = looksLikeDataQuestion(userText) || current.openIntents.some(item => /需工具核实/.test(item));
  if (!dataQ) return { ok: true };
  const text = String(assistantText ?? '');
  const hasResultEvidence = Array.isArray(toolResultsThisPrompt);
  const usedDataTool = hasResultEvidence ? toolResultsThisPrompt.some(result => result?.ok === true && result?.isError !== true && isDataReadTool(result.name)) : usedToolThisPrompt && (
    toolNamesThisPrompt.length === 0
    || toolNamesThisPrompt.some(name => isDataReadTool(name))
  );
  // Prior memory is advisory and cannot establish freshness for a new data query.
  const citesConfirmed = !hasResultEvidence && current.confirmedFacts.length > 0 && (
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

/** A data answer that still states numbers without a tool result is stored as a gap, not a fact. */
export function qualifyUnverifiedEnd(verdict, assistantText) {
  if (verdict?.ok !== false || !looksLikeFactualClaim(assistantText)) return { unqualified: false, note: '' };
  return {
    unqualified: true,
    note: '上面的数字没有本轮只读工具证据，不能当成事实。没查到要写成「本次未查到」，这不等于不存在。',
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

const PHASE_LABELS = Object.freeze({
  model: '正在问模型',
  tool: '正在执行工具',
  hold: '宿主还不让结束',
  compress: '正在收起更早的上下文',
  wait: '在等你授权',
  done: '这一轮结束',
});

/** UI snapshot of the harness. Strips the stored goal prefix. Grants nothing. */
export function publicInnerState(state, { phase = 'model', hold = '', compressed = false } = {}) {
  const current = normalizeAgentState(state);
  const goal = current.goal.replace(/^(?:核实并回答数据问题：|回答用户：)/, '');
  return {
    goal,
    phase,
    phaseLabel: PHASE_LABELS[phase] || PHASE_LABELS.model,
    facts: current.confirmedFacts.slice(-3),
    hypotheses: current.hypotheses.slice(-2),
    compressed: Boolean(compressed),
    hold: clip(hold, 180),
  };
}

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

// ---------------------------------------------------------------------------
// Pi-inspired finishTurn completion policy (Task 1) and tool-set announcement
// (Task 2). These are LOOP MECHANICS only. They grant no authority, never
// weaken a permission/scope/budget gate, and never retry a denied tool. The
// provider loop still owns cancellation, max_turn_requests and stall stops;
// these helpers only decide, at a clean end-of-turn, whether to inject one
// bounded "keep going / produce the deliverable" nudge.
// ---------------------------------------------------------------------------

const shortToolName = name => String(name ?? '').split(/::|__/).at(-1) || '';

// A finished deliverable for an autonomous analysis/pentest turn.
const DELIVERABLE_TOOLS = new Set(['record_finding']);
// Writing a report file (not merely writing a helper script) is a deliverable.
const REPORT_WRITE_TOOLS = new Set(['workspace_write', 'workspace_edit']);
// Tools that count as having done real analysis / execution work this turn.
const ANALYSIS_TOOLS = new Set([
  'workspace_run', 'workspace_read', 'workspace_grep', 'workspace_glob', 'workspace_write', 'workspace_edit',
  'scan_results', 'scan_fingerprint_read', 'scan_fingerprint_search',
  'inventory_query', 'inventory_search', 'inventory_stats',
  'exposure_overview', 'project_overview', 'journal_read',
]);

export const FORCED_CONTINUE_BUDGET = 3;

function reportLikePath(args) {
  const path = String(args?.path ?? args?.file ?? args?.filePath ?? '').toLowerCase();
  if (!path) return false;
  return /\.(md|markdown)$/.test(path) || /report|报告|finding|发现|writeup|summary/.test(path);
}

const okBusinessResult = result => result?.ok === true && result?.isError !== true && result?.advisory !== true;

/** A record_finding or a written report file this turn is a finished deliverable. */
export function producedDeliverable(toolResultsThisPrompt = []) {
  if (!Array.isArray(toolResultsThisPrompt)) return false;
  return toolResultsThisPrompt.some(result => okBusinessResult(result) && (
    DELIVERABLE_TOOLS.has(shortToolName(result.name))
    || (REPORT_WRITE_TOOLS.has(shortToolName(result.name)) && reportLikePath(result.arguments))
  ));
}

/** Did the turn actually run an analysis/execution/write tool (vs pure chat)? */
export function didAnalysisWork(toolResultsThisPrompt = []) {
  if (!Array.isArray(toolResultsThisPrompt)) return false;
  return toolResultsThisPrompt.some(result => okBusinessResult(result) && ANALYSIS_TOOLS.has(shortToolName(result.name)));
}

/**
 * Does the user's goal expect a produced deliverable (a finding, report, written
 * code, a fix, an exploit/PoC, or an audit/pentest conclusion)? A plain review,
 * lookup or Q&A does NOT — and must never be force-continued. This is the
 * "in-scope autonomous analysis/pentest task" gate; it keeps the force-continue
 * narrowly on the user's actual complaint (partial work on a build/analyze task).
 */
export function deliverableExpected(goalText = '') {
  const text = String(goalText ?? '');
  if (!text.trim()) return false;
  return /(审计|渗透|漏洞|利用链|提权|getshell|反序列化|注入|解密|破解|逆向|记录发现|写出?报告|输出报告|生成报告|复现|实现一个|实现个|写(?:一个|个)?(?:脚本|工具|代码|程序|报告|exploit|poc)|修复(?:漏洞|这个|该|问题|bug)|exploit|pentest|vulnerab|writeup|record[_\s-]?finding|report\b|\bpoc\b|implement\b|write (?:a |an )?(?:script|tool|report|exploit|poc|patch)|build (?:a |an )?(?:script|tool|exploit)|fix (?:the )?(?:bug|vuln|vulnerability|issue))/i.test(text);
}

/** The assistant's final message is a question / decision request directed at the user. */
export function looksLikeQuestionToUser(assistantText = '') {
  const text = String(assistantText ?? '').trim();
  if (!text) return false;
  const lastLine = text.split('\n').map(line => line.trim()).filter(Boolean).at(-1) ?? '';
  if (/[?？]\s*$/.test(lastLine)) return true;
  return /(请确认|请问|需要你|要不要我|是否(?:要|需要)我|要我(?:继续|先|不要|是否)|你(?:希望|想要|要不要|打算)|告诉我|请提供|请告知|等你(?:确认|决定|批准|回复)|由你决定|需要你的(?:确认|批准|决定)|\b(?:should i|shall i|do you want|would you like|let me know|please confirm|please advise|which option|how would you like)\b)/i.test(text);
}

/**
 * A SOFT confirmation-punt: the model defers an IN-SCOPE next step back to the user
 * ("要不要我继续 / 需你确认后再做 / 可以继续吗 / shall I continue"), rather than asking for
 * a specific missing fact or a choice between named options. These are exactly the stops the
 * user does NOT want — within confirmed scope the agent should just keep going. A genuine
 * request (请提供 X / which option / a real new authorization) does NOT match this and still stops.
 */
export function looksLikeSoftConfirmation(assistantText = '') {
  const text = String(assistantText ?? '').trim();
  if (!text) return false;
  // If the confirmation is about a specific OUTWARD / high-risk action (scan, send a request,
  // production, delete, attack, deploy, exploit, upload, push…), it is a GENUINE authorization
  // question and must stop — never auto-continued as an in-scope analysis punt.
  if (/(扫描|\bscan\b|发送|\bsend\b|发起|发请求|发包|攻击|\battack\b|删除|\bdelete\b|生产环境|\bprod(?:uction)?\b|外发|上传|\bupload\b|部署|\bdeploy\b|越权|利用漏洞|\bexploit\b|推送|\bpush\b|提交(?:请求|扫描|任务))/i.test(text)) return false;
  return /(需(?:要)?(?:你|您)?(?:的)?确认(?:后|无误后)?(?:再|才)?(?:做|继续|进行|执行|操作|动手|推进)?|要不要我(?:继续|往下|接着|做下去|推进)|要我(?:继续|接着|往下|做下去|推进)|是否(?:要|需要)?(?:我)?(?:继续|往下|接着|推进)|下一步(?:是否)?(?:需|要)(?:你|您)?(?:确认|同意|批准)|确认(?:后|无误后)?(?:再|才)(?:继续|执行|进行|做|推进)|可以(?:继续|推进)?(?:的话|了吗|吗|\?|？)|等(?:你|您)(?:确认|批准|同意|指示|回复)(?:后)?|待(?:你|您)确认|\b(?:shall i (?:continue|proceed|go ahead)|should i (?:proceed|continue|go ahead)|do you want me to (?:continue|proceed)|want me to keep going|let me know if (?:you'?d like|i should)|awaiting your (?:confirmation|go-?ahead)|need your (?:confirmation|approval) to (?:proceed|continue)|confirm before i (?:proceed|continue))\b)/i.test(text);
}

/**
 * finishTurn policy: at a clean end-of-turn (model emitted no tool calls), decide
 * whether to force exactly one more model request. Returns
 *   { continue: true, reason, hint }  — inject `hint` as context and loop once more
 *   { continue: false, reason }        — let the turn end
 *
 * Forces a continue only when ALL hold:
 *  - quality answer mode (autonomous runs),
 *  - the forced-continue budget is not yet exhausted,
 *  - this is not a read-only/lookup turn,
 *  - the turn actually did analysis/execution work,
 *  - no deliverable (record_finding / report file) was produced,
 *  - the model did NOT end by asking the user a question, and
 *  - the model did NOT hit an authorization/scope boundary.
 * Cancellation always wins (never forces when cancelled). The budget makes this
 * bounded and never unconditional.
 */
export function finishTurnDecision({
  answerMode = 'quality',
  goalText = '',
  usedToolThisPrompt = false,
  toolResultsThisPrompt = [],
  assistantText = '',
  readOnlyTurn = false,
  scopeBoundary = false,
  forcedContinuesUsed = 0,
  budget = FORCED_CONTINUE_BUDGET,
  cancelled = false,
} = {}) {
  if (cancelled) return { continue: false, reason: 'cancelled' };
  if (answerMode !== 'quality') return { continue: false, reason: 'not_quality_mode' };
  if (!(Number(forcedContinuesUsed) < Number(budget))) return { continue: false, reason: 'budget_exhausted' };
  if (readOnlyTurn) return { continue: false, reason: 'read_only_turn' };
  if (!deliverableExpected(goalText)) return { continue: false, reason: 'no_deliverable_expected' };
  if (!usedToolThisPrompt) return { continue: false, reason: 'no_work_simple_qa' };
  // A finished deliverable means done, regardless of what else ran this turn.
  if (producedDeliverable(toolResultsThisPrompt)) return { continue: false, reason: 'deliverable_produced' };
  if (!didAnalysisWork(toolResultsThisPrompt)) return { continue: false, reason: 'no_analysis_work' };
  // A real authorization/scope boundary stops first. Then: a SOFT confirmation-punt on
  // in-scope work is overridden (keep going — the user should not have to confirm every
  // step); only a genuine question (specific missing info / a choice) legitimately stops.
  if (scopeBoundary) return { continue: false, reason: 'scope_boundary' };
  if (looksLikeSoftConfirmation(assistantText)) return {
    continue: true,
    reason: 'soft_confirmation_override',
    hint: 'Host autonomy: 你刚用"要不要我继续 / 需你确认后再做 / 可以继续吗"把 scope 内的下一步推回给了用户——不要这样。在已确认授权与范围内，你不需要用户逐步确认，直接把当前这件事做完（继续分析/执行、产出 record_finding、或写成报告文件）。只有在确实需要用户新授权一个范围外或外发的高危动作、或必须让用户在几个具体选项里选择、或提供一个你拿不到的具体信息时，才停下并用一句话写清那个具体缺口。此自动续跑有次数上限。',
  };
  if (looksLikeQuestionToUser(assistantText)) return { continue: false, reason: 'asked_user_question' };
  return {
    continue: true,
    reason: 'incomplete_no_deliverable',
    hint: 'Host autonomy: 本轮已经做了分析/执行工作，但还没有产出最终交付物（没有 record_finding，也没有写出报告文件）。不要在这里停下、把半成品交回给用户。请继续完成当前任务：产出发现（record_finding）、把结果写成报告文件（workspace_write/workspace_edit 到 .md），或推进到下一个可执行步骤。只有确实遇到授权/范围边界，或必须由用户决定的问题时，才用一句话写清该具体缺口再停止。此自动续跑有次数上限。',
  };
}

// Tools whose arrival is worth flagging explicitly: the model has been seen to
// falsely conclude "I have no execution tools" right after a grant unlocks them.
const EXECUTION_TOOL_HINTS = new Set(['workspace_run', 'workspace_write', 'workspace_edit', 'record_finding']);

/**
 * Diff the currently-granted tool set against what the model was last told, and
 * build a short announcement for the newly-available tools. Returns
 *   { added: [...hostNames], executionAdded: [...shortNames], message }  or  null.
 * `previousNames === null` means no baseline yet (first step) → returns null.
 * `exclude` removes tools already announced by another path this step (by short name).
 */
export function toolSetAnnouncement(previousNames, currentNames = [], { exclude = [] } = {}) {
  if (!Array.isArray(previousNames)) return null;
  const prev = new Set(previousNames);
  const excluded = new Set((Array.isArray(exclude) ? exclude : []).map(shortToolName));
  const added = (Array.isArray(currentNames) ? currentNames : []).filter(name => !prev.has(name) && !excluded.has(shortToolName(name)));
  if (!added.length) return null;
  const executionAdded = added.map(shortToolName).filter(name => EXECUTION_TOOL_HINTS.has(name));
  const message = `Host: 以下工具现在可用（本次会话中新增）：${JSON.stringify(added.map(shortToolName))}。`
    + (executionAdded.length ? `其中 ${JSON.stringify(executionAdded)} 是执行/读写能力——你现在确实具备执行手段，不要再说自己没有可用的执行工具。` : '')
    + '只使用当前 tools/list 的 schema；实际执行仍由 Host 和业务 owner 复核，不要重发已发送或结果未知的控制。';
  return { added, executionAdded, message };
}
