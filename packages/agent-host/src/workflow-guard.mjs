import { createHash } from 'node:crypto';
import { scrubSecrets } from './quality-prompt.mjs';

const URL_ONLY = /^https?:\/\/\S+$/i;

/** A single URL is an intent, not a scan authorization. */
export function previewIntent(text) {
  const trimmed = String(text ?? '').trim();
  if (!URL_ONLY.test(trimmed) || /\s/.test(trimmed)) return null;
  let url;
  try { url = new URL(trimmed); } catch { return null; }
  if (url.username || url.password) return null;
  return {
    kind: 'url',
    url: `${url.origin}${url.pathname}`,
    choices: ['view_link', 'import_asset', 'prepare_authorized_scan'],
    startsScan: false,
  };
}

export function intentReply(intent, choice) {
  const target = intent?.url || '';
  if (!choice) return `这是一个链接，还不是扫描授权。目标：${target}。可选：查看链接、查询或导入资产、准备授权扫描。确认之前不会访问该地址，也不会创建扫描。`;
  return `已记下选择「${choice}」。目标仍是 ${target}。这次确认不会访问该地址，也不会启动扫描。`;
}

export function renderTaskReport(task) {
  const unknown = (task?.evidence ?? []).filter(item => item.outcomeUnknown);
  const lines = [
    '# 任务草稿报告',
    '',
    '状态：draft。这份报告不是已验证漏洞结论。',
    '',
    `- 任务：${task?.id || '缺失'}`,
    `- 项目：${task?.projectId ?? '缺失'}`,
    `- 目标：${scrubSecrets(String(task?.goal || '缺失')).slice(0, 500)}`,
    `- 任务状态：${task?.status || '未知'}`,
    '',
    '## draft',
    '- 这份报告是草稿。',
    '',
    '## verified',
    (task?.evidence ?? []).some(item => item.status === 'completed' && item.reconciliation) ? '- 有绑定原操作的核对收据。' : '- 无已核对收据。',
    '',
    '## unable_to_confirm',
    unknown.length ? unknown.map(item => `- ${item.operationId || 'legacy_unbound'} · ${item.toolName || '未知工具'} · 原文 ${item.rawRecordId || '缺失'}`).join('\n') : '- 无未知副作用。',
    '- 越权候选在缺少对象归属和合法控制组时保持无法确认。',
    '',
    '越权候选在缺少对象归属和合法控制组时保持无法确认。',
    '重生成这份草稿不会发起新的目标请求。',
  ];
  return lines.join('\n');
}

/** Capability is unknown unless the operator stored a number. Prices are never guessed. */
export function resolveModelCapability(config = {}) {
  const tokens = config.contextWindowTokens;
  const output = config.maxOutputTokens;
  return {
    provider: String(config.provider || 'unknown'),
    model: String(config.model || 'unknown'),
    contextWindowTokens: Number.isSafeInteger(tokens) && tokens > 0 ? tokens : null,
    maxOutputTokens: Number.isSafeInteger(output) && output > 0 ? output : null,
    price: null,
    usage: 'unknown',
  };
}

export function admitContext({ capability, messages, configuredInputBytes }) {
  const current = capability || resolveModelCapability();
  if (Number.isSafeInteger(current.contextWindowTokens)) {
    const reserved = current.maxOutputTokens || 1024;
    const inputTokens = Math.max(1, current.contextWindowTokens - reserved);
    return { ...contextFits(messages, inputTokens * 3), estimated: true, priceKnown: false };
  }
  if (Number.isSafeInteger(configuredInputBytes) && configuredInputBytes > 0) {
    return { ...contextFits(messages, configuredInputBytes), estimated: true, priceKnown: false };
  }
  return { ok: false, reason: 'unsupported_capability', estimated: false, priceKnown: false };
}

export const EXECUTION_PRESETS = Object.freeze({
  quick: { id: 'quick', revision: 1, rank: 1 },
  standard: { id: 'standard', revision: 1, rank: 2 },
  deep: { id: 'deep', revision: 1, rank: 3 },
});

/** Agent assistance can stay inside a preset. It cannot raise quick → standard → deep. */
export function bindExecutionPreset(currentId, nextId, actor = 'user') {
  const next = EXECUTION_PRESETS[nextId];
  if (!next) throw Object.assign(Error('host_preset_invalid'), { code: 'host_preset_invalid' });
  const current = currentId ? EXECUTION_PRESETS[currentId] : null;
  if (actor === 'agent' && current && next.rank > current.rank) {
    throw Object.assign(Error('host_preset_upgrade_denied'), { code: 'host_preset_upgrade_denied' });
  }
  return { presetRef: { id: next.id, revision: next.revision }, agentAssistance: actor === 'agent' ? 'plan-and-select' : 'off' };
}

/** Cross-owner amounts stay pending until both sides have a confirmed allocation. */
export function createBudgetAllocation({ total, host = 0, native = 0, modelCap }) {
  if (![total, host, native].every(value => Number.isSafeInteger(value) && value >= 0)) {
    throw Object.assign(Error('host_budget_invalid'), { code: 'host_budget_invalid', status: 400 });
  }
  if (host + native > total) throw Object.assign(Error('host_budget_invalid'), { code: 'host_budget_invalid', status: 400 });
  if (modelCap !== undefined && modelCap !== null && (!Number.isSafeInteger(modelCap) || modelCap < 0)) {
    throw Object.assign(Error('host_budget_invalid'), { code: 'host_budget_invalid', status: 400 });
  }
  return { total, host, native, pending: total - host - native, hostSpent: 0, nativeSpent: 0, ...(modelCap !== undefined ? { modelCap } : {}) };
}

export function spendAllocation(allocation, owner, amount) {
  if ((owner !== 'host' && owner !== 'native') || !Number.isSafeInteger(amount) || amount < 1) {
    throw Object.assign(Error('host_budget_invalid'), { code: 'host_budget_invalid' });
  }
  const cap = allocation[owner];
  const spentKey = owner === 'host' ? 'hostSpent' : 'nativeSpent';
  if (allocation[spentKey] + amount > cap) {
    throw Object.assign(Error('host_budget_hard_stop'), { code: 'host_budget_hard_stop' });
  }
  return { ...allocation, [spentKey]: allocation[spentKey] + amount };
}

function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]));
}

export function renderTaskReportHtml(task) {
  const markdown = renderTaskReport(task);
  return `<!DOCTYPE html><html><head><meta charset="utf-8"><title>任务草稿报告</title></head><body><pre>${escapeHtml(markdown)}</pre></body></html>`;
}

/** Byte ceiling is a transport guard. It is not a token window. */
export function contextFits(messages, ceiling) {
  if (!Number.isSafeInteger(ceiling) || ceiling < 1) return { ok: false, reason: 'context_insufficient' };
  const bytes = Buffer.byteLength(JSON.stringify(messages ?? []));
  return bytes <= ceiling ? { ok: true, bytes } : { ok: false, reason: 'context_insufficient', bytes };
}
