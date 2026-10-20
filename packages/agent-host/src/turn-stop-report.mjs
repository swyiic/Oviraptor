// A local receipt report, never another model request. Only this run's Host
// evidence is authoritative; arbitrary tool text, arguments and paths stay out.
const reasons = {
  max_turn_requests: '本轮达到模型请求次数上限，已停止继续规划。',
  paused_budget: '本轮模型请求额度已用尽，已暂停继续规划。',
  tool_loop_stalled: '相同工具调用连续失败，已停止重复调用。',
};
const labels = {
  inventory_stats: '资产统计', inventory_query: '资产查询',
  certificate_task_status: '证书任务状态', certificate_task_start: '证书补采提交',
  certificate_task_pause: '证书暂停请求', certificate_task_resume: '证书恢复请求',
  scan_list: '扫描任务列表', scan_status: '扫描任务状态', scan_preflight: '扫描启动前检查',
  scan_results: '扫描结果读取', scan_from_assets: '库存交接草稿',
  scan_start: '扫描启动请求', scan_pause: '扫描暂停请求', scan_resume: '扫描恢复请求',
};
const uuid = value => typeof value === 'string' && /^[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12}$/i.test(value) ? value : '';
const statuses = new Set(['draft','queued','pending','running','pausing','paused','completed','failed','interrupted','cancelled','stopped','recon_only','awaiting_input']);
const failureLabels = { host_budget_hard_stop: '业务调用额度已用尽', host_budget_paused: '业务调用额度暂停',
  host_tool_permission_denied: '缺少所需权限', host_tool_input_invalid: '参数无效', host_app_timeout: '业务工具超时',
  host_app_process_exited: '业务组件已退出', host_operation_unknown: '存在尚未核对的同一操作' };
const number = value => Number.isSafeInteger(value) && value >= 0 ? value : null;
const unknown = item => Boolean(item.outcomeUnknown || item.status === 'running' && item.readOnly !== true);

function receiptDetails(item, entry, projectId) {
  const fields = [], data = entry?.result?.structuredContent;
  if (!data || typeof data !== 'object' || Array.isArray(data) || data.projectId !== undefined && data.projectId !== projectId) return fields;
  const name = item.toolName;
  if (/^org\.oviraptor\.scanning__scan_/.test(name)) {
    const task = data.task;
    if (task && uuid(task.id)) {
      fields.push(`任务 ${task.id}`);
      if (statuses.has(task.status)) fields.push(`返回状态 ${task.status}`);
      if (number(task.attempt) !== null) fields.push(`尝试 ${task.attempt}`);
      if (number(task.modelRequests) !== null) fields.push(`模型请求 ${task.modelRequests}`);
    }
    if (name.endsWith('__scan_results')) {
      if (Array.isArray(data.rows)) fields.push(`本页 ${data.rows.length} 条`);
      if (number(data.nextAfterId) !== null) fields.push(`未读游标 ${data.nextAfterId}`);
    }
  } else if (/^org\.oviraptor\.assets__certificate_task_/.test(name)) {
    const tasks = data.task ? [data.task] : Array.isArray(data.tasks) ? data.tasks : [];
    for (const task of tasks.slice(0, 3)) {
      if (!uuid(task?.id)) continue;
      fields.push(`任务 ${task.id}${statuses.has(task.status) ? ` 返回状态 ${task.status}` : ''}`);
      const p = task.progress;
      if (number(p?.captured) !== null && number(p?.total) !== null) fields.push(`已采 ${p.captured}/${p.total}`);
      if (number(p?.uniqueFingerprints) !== null) fields.push(`去重指纹 ${p.uniqueFingerprints}`);
    }
    if (tasks.length > 3) fields.push('其余任务见原回执');
  } else if (name === 'org.oviraptor.assets__inventory_stats' && number(data.total) !== null) {
    fields.push(`本地资产记录 ${data.total} 条`);
  }
  return fields;
}

/** Does not modify the task, acceptance, evidence or business owner. */
export function buildTurnStopReport(task, run, stopReason, results = new Map(), modelBudget = null) {
  if (!reasons[stopReason] || !task || task.id !== run?.taskId || task.conversationId !== run.conversationId
    || !task.runs?.some(item => item.id === run.id && item.assistantMessageId === run.message?.id)) return null;
  const evidence = task.evidence.filter(item => item.runId === run.id);
  const uncertain = evidence.filter(unknown);
  const reads = evidence.filter(item => item.status === 'completed' && !unknown(item) && item.readOnly === true);
  const writes = evidence.filter(item => item.status === 'completed' && !unknown(item) && item.readOnly === false);
  const failures = evidence.filter(item => !unknown(item) && ['failed','interrupted'].includes(item.status));
  const lines = [reasons[stopReason],
    `Host 本轮回执：成功读取 ${reads.length} 次，操作返回 ${writes.length} 次，失败或拒绝 ${failures.length} 次，结果未知或尚未返回 ${uncertain.length} 次。`];
  if (number(modelBudget?.modelCap) !== null && number(modelBudget?.modelSpent) !== null) {
    lines.push(`项目 Host 模型请求额度：已准入 ${modelBudget.modelSpent} / ${modelBudget.modelCap} 次 HTTP 尝试（启用上限时累计，非美元费用或平台积分）。`);
  }
  if (!reads.length && !writes.length) lines.push('本轮没有成功业务工具回执，不能确认请求已完成。');
  if (evidence.length) {
    lines.push('最近回执（返回状态属于当时的快照，操作返回不代表后台任务已完成）：');
    for (const item of evidence.slice(-8)) {
      const short = String(item.toolName).split(/::|__/).at(-1);
      const label = labels[short] ?? (item.readOnly ? '其他业务读取' : '其他业务操作');
      const state = unknown(item) ? '结果未知，先核对再继续' : item.status === 'completed' ? '已返回' : '失败或未完成';
      const entry = results.get(item.id);
      const details = entry?.name === item.toolName && !unknown(item) && item.status === 'completed'
        ? receiptDetails(item, entry, task.projectId) : [];
      const op = unknown(item) && uuid(item.operationId) ? `；操作 ${item.operationId}` : '';
      const error = item.status !== 'completed' && failureLabels[item.code] ? `；${failureLabels[item.code]}` : '';
      lines.push(`- ${label}：${state}${details.length ? `；${details.join('；')}` : ''}${op}${error}`);
    }
    if (evidence.length > 8) lines.push(`另有 ${evidence.length - 8} 条本轮回执保留在任务记录中。`);
  }
  if (task.outcomeUnknown && !uncertain.length) lines.push('历史任务仍有结果未知的记录，本轮停止不会清除它们。');
  lines.push('原计划、任务记录和回执已保留，本轮未作整体完成验收。继续前先读取业务任务的当前状态，结果未知的操作不自动重发。');
  if (writes.length || uncertain.some(item => item.readOnly !== true)) lines.push('停止本轮规划不会暂停已启动的后台任务；如需暂停，请对相应任务执行暂停。');
  return { text: lines.join('\n\n'), outcomeUnknown: uncertain.length > 0,
    counts: { reads: reads.length, writes: writes.length, failures: failures.length, unknown: uncertain.length } };
}
