import { randomUUID } from 'node:crypto';

export const FACT_TYPES = Object.freeze([
  'request_captured',
  'crypto_function',
  'plaintext_field',
  'identity',
  'endpoint',
  'internal_address',
  'file_identified',
  'file_excerpt',
]);

const FACT_LABELS = {
  request_captured: '抓到的请求',
  crypto_function: '加密函数',
  plaintext_field: '明文参数',
  identity: '身份',
  endpoint: '新接口',
  internal_address: '响应里的内部地址',
  file_identified: '识别出的文件',
  file_excerpt: '文件片段',
};

export function ensureChain(db) {
  db.exec(`CREATE TABLE IF NOT EXISTS conversation_hops(
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    action TEXT NOT NULL,
    fact_type TEXT NOT NULL,
    fact_value TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT '',
    request_json TEXT NOT NULL DEFAULT '{}',
    opens_next INTEGER NOT NULL DEFAULT 1,
    permission TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
  )`);
  db.exec('CREATE INDEX IF NOT EXISTS conversation_hops_order ON conversation_hops(conversation_id, seq, id)');
}

function clip(value, max = 500) {
  const text = String(value ?? '').replace(/\s+/g, ' ').trim();
  return text.length > max ? text.slice(0, max) : text;
}

export function addHop(db, conversationId, hop) {
  ensureChain(db);
  if (!FACT_TYPES.includes(hop.factType)) throw Object.assign(Error('chain_fact_invalid'), { code: 'chain_fact_invalid' });
  const value = clip(hop.factValue);
  if (!value) throw Object.assign(Error('chain_fact_invalid'), { code: 'chain_fact_invalid' });
  const parent = hop.parentId || latestHop(db, conversationId)?.id || '';
  const row = {
    id: randomUUID(),
    conversationId,
    parentId: parent,
    action: clip(hop.action || 'note', 80),
    factType: hop.factType,
    factValue: value,
    source: clip(hop.source, 800),
    request: hop.request && typeof hop.request === 'object' ? hop.request : {},
    opensNext: hop.opensNext !== false,
    permission: clip(hop.permission, 80),
    createdAt: new Date().toISOString(),
  };
  const seq = db.prepare('SELECT COALESCE(MAX(seq),0)+1 AS n FROM conversation_hops WHERE conversation_id=?').get(conversationId).n;
  db.prepare(`INSERT INTO conversation_hops(id,conversation_id,seq,parent_id,action,fact_type,fact_value,source,request_json,opens_next,permission,created_at)
    VALUES(?,?,?,?,?,?,?,?,?,?,?,?)`).run(row.id, conversationId, seq, row.parentId, row.action, row.factType, row.factValue, row.source,
    JSON.stringify(row.request), row.opensNext ? 1 : 0, row.permission, row.createdAt);
  return row;
}

function mapRow(row) {
  let request = {};
  try { request = JSON.parse(row.request_json || '{}'); } catch { request = {}; }
  return {
    id: row.id,
    parentId: row.parent_id,
    action: row.action,
    factType: row.fact_type,
    factLabel: FACT_LABELS[row.fact_type] || row.fact_type,
    factValue: row.fact_value,
    source: row.source,
    request,
    opensNext: row.opens_next === 1,
    permission: row.permission,
    createdAt: row.created_at,
  };
}

export function listHops(db, conversationId, limit = 40) {
  ensureChain(db);
  return db.prepare('SELECT * FROM conversation_hops WHERE conversation_id=? ORDER BY seq, id LIMIT ?').all(conversationId, limit).map(mapRow);
}

function latestHop(db, conversationId) {
  ensureChain(db);
  const row = db.prepare('SELECT * FROM conversation_hops WHERE conversation_id=? ORDER BY seq DESC, id DESC LIMIT 1').get(conversationId);
  return row ? mapRow(row) : null;
}

export function formatChain(hops) {
  if (!hops?.length) return '';
  const lines = ['Host: 当前分析链。下一跳只能使用最后一条 opensNext 为 true 的事实，不能另起一条无关测试。文件分析也是链：先识别文件，再引用片段。'];
  for (const hop of hops.slice(-12)) {
    const request = hop.request?.url ? ` 请求 ${hop.request.method || 'GET'} ${hop.request.url} ${hop.request.status || ''}` : '';
    lines.push(`- ${hop.factLabel}：${hop.factValue}（动作 ${hop.action}，出处 ${hop.source || '未记'}，${hop.opensNext ? '可进入下一跳' : '本跳停止'}）${request}`);
  }
  return lines.join('\n');
}

function requestOf(result) {
  return { method: result.method || 'GET', url: result.url || '', status: result.status || 0, path: result.path || result.absolutePath || '' };
}

export function hopsFromTool(name, args = {}, result = {}) {
  if (!result || result.isError) return [];
  if (name === 'workspace_chain_add') {
    return [{ action: 'judge', factType: result.factType, factValue: result.factValue, source: result.source, request: result.request, opensNext: result.opensNext !== false, permission: 'user' }];
  }
  if (name === 'workspace_fetch' || name === 'workspace_http' || name === 'workspace_probe_command') {
    const url = result.url || args.url;
    if (!url) return [];
    return [{ action: name.replace('workspace_', ''), factType: 'request_captured', factValue: url, source: result.path || result.absolutePath || '', request: requestOf(result), opensNext: true }];
  }
  if (name === 'workspace_probe_sql' && Array.isArray(result.shots)) {
    return result.shots.slice(0, 8).map(shot => ({
      action: 'probe_sql', factType: 'request_captured', factValue: `${result.param}=${shot.value}`, source: shot.path || '',
      request: { method: 'GET', url: result.url, status: shot.status, path: shot.path || '' }, opensNext: true,
    }));
  }
  if (name === 'workspace_browser') {
    const hops = [{ action: 'browser', factType: 'request_captured', factValue: result.url || args.url || '', source: 'browser', request: { method: 'GET', url: result.url || '', status: 0 }, opensNext: true }];
    for (const script of (result.scripts || []).slice(0, 8)) {
      hops.push({ action: 'browser', factType: 'file_identified', factValue: script.path, source: script.url || '', request: { url: script.url || '' }, opensNext: true });
    }
    return hops.filter(hop => hop.factValue);
  }
  if (name === 'workspace_source_search' || name === 'workspace_grep') {
    return (result.matches || []).slice(0, 5).map(match => ({
      action: name === 'workspace_grep' ? 'grep' : 'source_search',
      factType: 'file_excerpt',
      factValue: match.text || '',
      source: `${match.path || match.file || ''}${match.offset != null ? ` @${match.offset}` : match.line ? `:${match.line}` : ''}`,
      opensNext: true,
    })).filter(hop => hop.factValue);
  }
  if (name === 'workspace_read_path') {
    return [{ action: 'read_file', factType: 'file_excerpt', factValue: String(result.content || '').slice(0, 500), source: `${result.path || args.path || ''} @${result.offset || 0}`, opensNext: true }];
  }
  if (name === 'workspace_source_list') {
    return [{ action: 'source_list', factType: 'file_identified', factValue: result.path || args.path || '', source: `${(result.entries || []).length} entries`, opensNext: true }];
  }
  return [];
}

export function fileHops(files) {
  return (files || []).map(file => ({
    action: 'file',
    factType: 'file_identified',
    factValue: file.path || file.source,
    source: file.source || file.path || '',
    request: { bytes: file.bytes || 0 },
    opensNext: true,
  })).filter(hop => hop.factValue);
}
