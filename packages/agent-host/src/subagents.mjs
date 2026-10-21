import { createHash, randomUUID } from 'node:crypto';
import { OpenAiCompatibleRuntime } from './provider.mjs';
import { startTaskRun, finishTaskRun, formatTaskPrompt } from './tasks.mjs';
import { hostError, boundedText } from './store.mjs';
import { scrubSecrets } from './quality-prompt.mjs';

const schema = properties => ({ type: 'object', properties, additionalProperties: false });
export const SUBAGENT_TOOL_SPECS = [
  { name: 'host_task_delegate', description: 'Start a bounded read-only research/review child under the current parent scope. Returns a durable handle immediately; use host_task_await to collect its advisory report. At most two concurrent children, eight per run. No writes, new consent, recursive delegation, or parent completion. Reports never prove task completion or test execution.', inputSchema: { ...schema({ goal: { type: 'string', maxLength: 4000 }, kind: { enum: ['research', 'review'] }, tools: { type: 'array', minItems: 1, maxItems: 16, items: { type: 'string' } } }), required: ['goal', 'kind', 'tools'] } },
  { name: 'host_task_await', description: 'Collect advisory reports for named children of this task. Waits at most ten seconds; pending children remain running. Historical reports retain their original run identity and never become parent evidence.', inputSchema: { ...schema({ childTaskIds: { type: 'array', minItems: 1, maxItems: 8, items: { type: 'string' } }, timeoutMs: { type: 'integer', minimum: 0, maximum: 10000 } }), required: ['childTaskIds'] } },
  { name: 'host_task_cancel', description: 'Cancel one current-run child and revoke its scope before cleanup. Does not cancel the parent or authorize a replacement child. Metadata only.', inputSchema: { ...schema({ childTaskId: { type: 'string' } }), required: ['childTaskId'] } },
];
const metadataNames = new Set(['host_task_read', 'host_task_update']);
export const advisoryResult = value => ({ content: [{ type: 'text', text: JSON.stringify(value) }], _meta: { hostEvidence: 'advisory' } });
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const fail = code => { throw hostError(code, 403); };
const nameOf = tool => tool.name.replace('::', '__');
// Read classification comes from the existing granted permission contract, not
// a model-supplied tool name or a guess based on get/list/read in its spelling.
const readPermission = permission => permission === 'read' || typeof permission === 'string' && permission.endsWith('.read');

export function authorizedReadCapabilities(host, parent) {
  const project = host.store.conversation(parent.conversationId).projectId;
  const apps = host.apps.capabilities(project, { runtimeIdentity: host.runtimeIdentity() });
  const skills = parent.skillScope && host.skillAccess ? host.skillAccess.toolSpecs(parent.conversationId, parent.skillScope) : [];
  return [
    ...(parent.workspaceAllow?.has('workspace.read') ? ['workspace_list', 'workspace_read'].map(name => ({ name, source: 'workspace' })) : []),
    ...skills.map(tool => ({ name: tool.name, source: 'skill' })),
    ...apps.filter(tool => readPermission(tool.permission)).map(tool => ({ name: nameOf(tool), source: 'application', permission: tool.permission, grantRevision: tool.grantRevision, appId: tool.appId })),
  ];
}
function authority(host, parent) {
  const project = host.store.conversation(parent.conversationId).projectId;
  const tools = host.apps.capabilities(project, { runtimeIdentity: host.runtimeIdentity() }).map(tool => { const app = host.apps.store?.application?.(tool.appId); return [nameOf(tool), tool.permission, tool.grantRevision, tool.appId, app?.revision, app?.digest, app?.version]; }).sort((a, b) => a[0].localeCompare(b[0]));
  return createHash('sha256').update(JSON.stringify({ tools, runtime: host.runtimeIdentity(), workspace: [...(parent.workspaceAllow ?? [])].sort() })).digest('hex');
}
export function childSummary(task) {
  return { id: task.id, runId: task.runs.at(-1)?.id, parentRunId: task.parentRunId, kind: task.kind, goal: task.goal, status: task.status, resultRef: `child-report:${task.id}:${task.runs.at(-1)?.id}`, provenance: 'advisory-child-report', ...(task.report ? { report: task.report } : {}) };
}

/** Children use dedicated model sessions but the parent's immutable authority
 * snapshot. This is Host capability isolation, not a new OS sandbox for Apps. */
export class ReadOnlySubagents {
  constructor(host) { this.host = host; this.runs = new Map(); }
  parentActive(run) {
    const host = this.host, parent = host.runs.get(run.conversationId);
    if (!parent || parent.id !== run.parentRunId || parent.taskId !== run.parentTaskId || parent.cancelled || ['cancelling', 'finishing'].includes(parent.state) || host.mcpScopes.get(run.parentToken) !== run.parentScope || run.cancelled || this.runs.get(run.id) !== run || authority(host, parent) !== run.authority) fail('host_child_scope_unavailable');
    if (run.skillScope && host.skillAccess) host.skillAccess.validateQuick(run.conversationId, run.skillScope);
    return parent;
  }
  allows(run, name) { this.parentActive(run); return metadataNames.has(name) || run.scopeTools.includes(name); }
  revalidate() {
    for (const run of this.runs.values()) {
      try { this.parentActive(run); }
      catch { this.stop(run, 'host_child_scope_unavailable').catch(() => {}); }
    }
  }
  #publish(run) {
    const host = this.host, child = host.store.task(run.conversationId, run.taskId), parent = host.store.task(run.conversationId, run.parentTaskId);
    const children = (parent.children ?? []).filter(item => item.id !== child.id);
    children.push(childSummary(child));
    host.store.saveTask({ ...parent, children, acceptance: null, status: parent.status === 'completed' ? 'awaiting_input' : parent.status, updatedAt: new Date().toISOString() });
    host.emit({ type: 'task.updated', conversationId: run.conversationId, taskId: parent.id, runId: run.parentRunId, task: host.task(run.conversationId, { taskId: parent.id }) });
    host.emit({ type: 'task.child.updated', conversationId: run.conversationId, parentTaskId: parent.id, parentRunId: run.parentRunId, taskId: child.id, runId: run.id, task: child });
  }
  async delegate(parent, input, toolCallId) {
    const host = this.host;
    host.validateMcpScope(parent.scopeToken, parent.conversationId, { runId: parent.id });
    if (parent.parentRunId || host.runtimeId !== 'xai-api' || !(host.runtime instanceof OpenAiCompatibleRuntime)) fail('host_child_runtime_unsupported');
    if (!object(input) || Object.keys(input).some(key => !['goal', 'kind', 'tools'].includes(key)) || !['research', 'review'].includes(input.kind) || !Array.isArray(input.tools) || input.tools.length < 1 || input.tools.length > 16 || input.tools.some(name => typeof name !== 'string')) fail('host_child_input_invalid');
    if (toolCallId && parent.childCalls?.has(toolCallId)) return advisoryResult(childSummary(host.store.task(parent.conversationId, parent.childCalls.get(toolCallId))));
    const goal = boundedText(input.goal, 4000), names = [...new Set(input.tools)];
    const existing = host.store.tasks(parent.conversationId).filter(task => task.parentTaskId === parent.taskId);
    if ([...this.runs.values()].filter(run => run.parentRunId === parent.id).length >= 2 || existing.filter(task => task.parentRunId === parent.id).length >= 8 || existing.length >= 64) fail('host_child_limit');
    const available = authorizedReadCapabilities(host, parent);
    const capabilities = names.map(name => available.find(tool => tool.name === name) ?? fail('host_child_tool_denied'));
    const task = host.store.createTask(parent.conversationId, goal, { parentTaskId: parent.taskId, parentRunId: parent.id, kind: input.kind, scopeTools: names, report: null });
    const run = { id: randomUUID(), conversationId: parent.conversationId, projectId: task.projectId, taskId: task.id, parentTaskId: parent.taskId, parentRunId: parent.id, parentToken: parent.scopeToken, parentScope: host.mcpScopes.get(parent.scopeToken), authority: authority(host, parent), kind: input.kind, capabilities, scopeTools: Object.freeze(names), skillScope: parent.skillScope, workspaceAllow: new Set(parent.workspaceAllow?.has('workspace.read') ? ['workspace.read'] : []), state: 'running', text: '', userMessageId: null, message: { id: null }, cancelled: false };
    run.scopeToken = randomUUID(); this.runs.set(run.id, run);
    host.mcpScopes.set(run.scopeToken, { conversationId: run.conversationId, runtimePid: host.runtime.status.pid, runId: run.id, parentRunId: parent.id });
    host.store.saveTask(startTaskRun(task, run)); this.#publish(run);
    if (toolCallId) { parent.childCalls ??= new Map(); parent.childCalls.set(toolCallId, task.id); }
    const parentSession = host.runtime.sessions.get(parent.sessionId);
    run.runtime = new OpenAiCompatibleRuntime({ config: { ...(parentSession?.config ?? host.runtime.config) }, fetch: host.providerFetch, maxSteps: 4, answerMode: 'speed', timeoutMs: 30000, onEvent: event => { if (event.type === 'text_delta' && !run.cancelled) run.text = (run.text + event.text).slice(0, 8000); } });
    run.runtime.bindHost({ mcp: host.mcp.bind(host), takeSteer: () => [], permissionOffer: () => ({ offers: [] }), requestAuthorization: () => false, waitForAuthorization: () => false, providerStore: host.providerStore, ensureOAuthAccess: host.ensureOAuthAccess.bind(host), admitModelRequest: host.admitModelRequest.bind(host), recordSummary: host.recordSummary.bind(host) });
    run.completion = this.#execute(run, goal).catch(error => {
      host.mcpScopes.delete(run.scopeToken); this.runs.delete(run.id);
      // Storage/cleanup failures never escape as a falsely successful report.
      host.emit({ type: 'runtime.child.error', conversationId: run.conversationId, parentTaskId: run.parentTaskId, runId: run.id, code: error.code ?? 'host_child_failed' });
    });
    return advisoryResult(childSummary(host.store.task(run.conversationId, run.taskId)));
  }
  async #execute(run, goal) {
    const host = this.host; let status = 'interrupted', errorCode = '', result;
    const timer = setTimeout(() => { this.stop(run, 'host_child_timeout').catch(() => {}); }, 30000); timer.unref?.();
    try {
      const session = await run.runtime.newSession({ mcpServers: [{ env: [{ name: 'OVIRAPTOR_MCP_TOKEN', value: run.scopeToken }, { name: 'OVIRAPTOR_CONVERSATION', value: run.conversationId }, { name: 'OVIRAPTOR_RUN_ID', value: run.id }] }] });
      run.sessionId = session.sessionId; this.parentActive(run);
      result = await run.runtime.prompt({ sessionId: run.sessionId, turnId: run.id, userText: goal, taskContext: { id: run.taskId, goal }, answerMode: 'speed', prompt: `Read-only ${run.kind}. Report observations with limits and tool references. Never request new authority, delegate or write.\n${formatTaskPrompt(host.store.task(run.conversationId, run.taskId))}\n${goal}` });
      this.parentActive(run); status = result.stopReason === 'end_turn' ? 'reported' : 'interrupted';
      if (result.replaceText) run.text = result.assistantText;
    } catch (error) { errorCode = run.cancelReason ?? error.code ?? 'host_child_failed'; }
    finally {
      clearTimeout(timer); host.mcpScopes.delete(run.scopeToken);
      await run.runtime.close();
      if (run.cancelled) status = run.cancelReason === 'host_child_cancelled' ? 'cancelled' : 'interrupted';
      let saved = finishTaskRun(host.store.task(run.conversationId, run.taskId), run, status === 'reported' ? 'completed' : 'cancelled', { outcomeUnknown: false, failureCode: errorCode, stopReason: result?.stopReason ?? '', cancelRequested: run.cancelled });
      const report = { provenance: 'advisory-child-report', authority: 'none', parentTaskId: run.parentTaskId, childTaskId: run.taskId, runId: run.id, status, text: scrubSecrets(run.text).slice(0, 8000), evidenceRefs: saved.evidence.filter(item => item.status === 'completed').map(item => item.id) };
      host.store.saveTask({ ...saved, status, report }); this.#publish(run); this.runs.delete(run.id);
    }
  }
  #owned(parent, childTaskId) {
    const task = this.host.store.task(parent.conversationId, childTaskId);
    if (task.parentTaskId !== parent.taskId) fail('host_child_scope_unavailable');
    return task;
  }
  async await(parent, input) {
    if (!object(input) || Object.keys(input).some(key => !['childTaskIds', 'timeoutMs'].includes(key)) || !Array.isArray(input.childTaskIds) || !input.childTaskIds.length || input.childTaskIds.length > 8 || input.childTaskIds.some(id => typeof id !== 'string') || input.timeoutMs !== undefined && (!Number.isSafeInteger(input.timeoutMs) || input.timeoutMs < 0 || input.timeoutMs > 10000)) fail('host_child_input_invalid');
    const tasks = input.childTaskIds.map(id => this.#owned(parent, id));
    const pending = tasks.map(task => this.runs.get(task.runs.at(-1)?.id)?.completion).filter(Boolean);
    let timer;
    try { if (pending.length && input.timeoutMs !== 0) await Promise.race([Promise.allSettled(pending), new Promise(resolve => { timer = setTimeout(resolve, input.timeoutMs ?? 1000); })]); }
    finally { clearTimeout(timer); }
    this.host.validateMcpScope(parent.scopeToken, parent.conversationId, { runId: parent.id });
    const children = input.childTaskIds.map(id => childSummary(this.#owned(parent, id)));
    return advisoryResult({ children, pending: children.some(child => child.status === 'running') });
  }
  async cancel(parent, input) {
    if (!object(input) || Object.keys(input).some(key => key !== 'childTaskId') || typeof input.childTaskId !== 'string') fail('host_child_input_invalid');
    const task = this.#owned(parent, input.childTaskId), run = this.runs.get(task.runs.at(-1)?.id);
    if (run && run.parentRunId !== parent.id) fail('host_child_scope_unavailable');
    if (run) { await this.stop(run, 'host_child_cancelled'); await run.completion; }
    return advisoryResult(childSummary(this.#owned(parent, input.childTaskId)));
  }
  async stop(run, reason) {
    run.cancelled = true; run.cancelReason ??= reason; this.host.mcpScopes.delete(run.scopeToken);
    await run.runtime?.cancel(run.sessionId); await run.runtime?.close();
    // #execute owns durable finalization; avoid waiting on its own timeout path.
  }
  async cancelTree(parentRunId) {
    const children = [...this.runs.values()].filter(run => run.parentRunId === parentRunId);
    for (const run of children) { run.cancelled = true; run.cancelReason ??= 'host_parent_ended'; this.host.mcpScopes.delete(run.scopeToken); }
    await Promise.allSettled(children.map(run => this.stop(run, run.cancelReason)));
    await Promise.allSettled(children.map(run => run.completion));
  }
}
