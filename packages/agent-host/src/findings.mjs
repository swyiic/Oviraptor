// Agent-recorded structured findings (authorized pentest evidence). The
// record_finding tool lets the agent persist a CONFIRMED vulnerability — title,
// severity, cvss, evidence, combination/attack chain, business impact, PoC, and the
// HTTP request/response that proves it — for the results UI and the report.
// Recording is EVIDENCE CAPTURE, not an action: it grants no permission and never
// feeds any auth/scope/settlement/capability gate; sending the actual request still
// goes through authorization + approval. A finding is 'unverified' until operator review.
const clip = (s, n) => { s = String(s ?? ''); return s.length > n ? s.slice(0, n) : s; };
const SEVERITIES = ['critical', 'high', 'medium', 'low', 'info'];

export const FINDING_TOOL_SPECS = [{
  name: 'record_finding',
  description: '把你在授权测试中确认的一个漏洞记录成结构化发现(标题、严重度、证据、组合攻击链、业务影响、PoC、证明它的 HTTP 请求/响应)。这是证据记录与呈现,不授予任何权限、不扩范围;实际发请求/高危动作仍走已确认授权与审批。记录后默认待操作员复核确认。只记你已复现/可证明的,别把假设当确认。',
  inputSchema: {
    type: 'object',
    properties: {
      title: { type: 'string', description: '漏洞标题' },
      severity: { type: 'string', enum: SEVERITIES },
      cvss: { type: 'string' }, confidence: { type: 'string' }, cwe: { type: 'string' }, cve: { type: 'string' },
      description: { type: 'string' },
      evidence: { type: 'string', description: '证据 / 复现说明' },
      chainContext: { type: 'string', description: '组合攻击链:它和哪些发现串成端到端链、叠加后的最大危害' },
      businessImpact: { type: 'string' },
      pocRequest: { type: 'string' },
      pocScript: { type: 'string', description: '复现脚本正文或工作区相对路径' },
      pocOutput: { type: 'string' },
      recommendation: { type: 'string' },
      httpRequest: { type: 'object', description: '证明用的 HTTP 请求', properties: { method: { type: 'string' }, url: { type: 'string' }, headers: { type: 'object' }, body: { type: 'string' } } },
      httpResponse: { type: 'object', description: '对应响应', properties: { status: { type: 'integer' }, headers: { type: 'object' }, body: { type: 'string' } } },
      wsFrames: { type: 'array', description: 'workspace_ws 抓到的 WebSocket 帧作证据', maxItems: 100, items: { type: 'object', properties: { dir: { type: 'string' }, type: { type: 'string' }, at: { type: 'integer' }, text: { type: 'string' }, base64: { type: 'string' }, bytes: { type: 'integer' } } } },
    },
    required: ['title', 'severity'],
    additionalProperties: false,
  },
}];

function boundWsFrames(frames) {
  if (!Array.isArray(frames) || !frames.length) return undefined;
  const out = frames.slice(0, 100).map(f => {
    if (!f || typeof f !== 'object') return null;
    const frame = {};
    if (f.dir) frame.dir = clip(f.dir, 8);
    if (f.type) frame.type = clip(f.type, 8);
    if (Number.isInteger(f.at)) frame.at = f.at;
    if (Number.isInteger(f.bytes)) frame.bytes = f.bytes;
    if (f.text != null) frame.text = clip(f.text, 8000);
    if (f.base64 != null) frame.base64 = clip(f.base64, 12000);
    return Object.keys(frame).length ? frame : null;
  }).filter(Boolean);
  return out.length ? out : undefined;
}

function boundHttp(h, maxBody) {
  if (!h || typeof h !== 'object') return undefined;
  const out = {};
  if (h.method) out.method = clip(h.method, 16);
  if (h.url) out.url = clip(h.url, 2048);
  if (Number.isInteger(h.status)) out.status = h.status;
  if (h.headers && typeof h.headers === 'object') out.headers = h.headers;
  if (h.body != null) out.body = clip(h.body, maxBody);
  return Object.keys(out).length ? out : undefined;
}

/** Build a finding's record_json from the tool args and persist it (unverified). */
export function recordFindingFromTool(store, conversationId, projectId, args = {}) {
  const title = clip(args.title, 256).trim();
  if (!title) return { ok: false, reason: 'empty_title' };
  const severity = SEVERITIES.includes(args.severity) ? args.severity : 'info';
  const record = {
    title, severity,
    cvss: clip(args.cvss, 40), confidence: clip(args.confidence, 40), cwe: clip(args.cwe, 40), cve: clip(args.cve, 40),
    description: clip(args.description, 4000), evidence: clip(args.evidence, 8000),
    chain_context: clip(args.chainContext, 4000), business_impact: clip(args.businessImpact, 4000),
    poc_request: clip(args.pocRequest, 8000), poc_script: clip(args.pocScript, 16000), poc_output: clip(args.pocOutput, 8000),
    recommendation: clip(args.recommendation, 4000),
    http_request: boundHttp(args.httpRequest, 16000), http_response: boundHttp(args.httpResponse, 16000),
    ws_frames: boundWsFrames(args.wsFrames),
  };
  const saved = store.recordFinding(conversationId, projectId, { title, severity, kind: 'vulnerability', recordJson: JSON.stringify(record) });
  return { ok: true, id: saved.id, status: saved.status };
}
