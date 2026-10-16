/** UI orchestration only. Every action goes through the Host's current grant. */
import { readHandoff } from '../../packages/app-sdk/src/handoff.mjs';

export function readScanTargetHandoff(value, projectId) {
  const handoff = readHandoff(value, projectId), payload = handoff.payload;
  const keys = Object.keys(payload).sort().join(',');
  const invalid = () => { throw Object.assign(Error('scan_handoff_targets_invalid'), { code: 'scan_handoff_targets_invalid' }); };
  if (keys !== 'kind,mode,source,targets' || payload.kind !== 'scan.targets.v1' ||
      !['quick', 'standard', 'deep'].includes(payload.mode) || !Array.isArray(payload.targets) || !payload.targets.length || payload.targets.length > 32) invalid();
  const source = payload.source;
  if (!source || typeof source !== 'object' || Array.isArray(source) ||
      Object.keys(source).sort().join(',') !== 'applicationInstanceId,assetCount,kind,selectionDigest,selectionId' ||
      source.kind !== 'asset-selection' || typeof source.applicationInstanceId !== 'string' || !/^[A-Za-z0-9._:-]{1,128}$/u.test(source.applicationInstanceId) ||
      typeof source.selectionId !== 'string' || !/^[a-f0-9]{8}-[a-f0-9]{4}-[1-8][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/u.test(source.selectionId) ||
      typeof source.selectionDigest !== 'string' || !/^[a-f0-9]{64}$/u.test(source.selectionDigest) || !Number.isSafeInteger(source.assetCount) || source.assetCount < 1) invalid();
  for (const target of payload.targets) {
    if (typeof target !== 'string' || !target || new TextEncoder().encode(target).length > 2048 || /[\s\u0000-\u001f\u007f,，;；]/u.test(target) || /^["'<>，。]|["'<>，。]$/u.test(target)) invalid();
    let url; try { url = new URL(target); } catch { invalid(); }
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password || url.hash) invalid();
  }
  return Object.freeze({ sourceAppId: handoff.sourceAppId, targets: payload.targets, scanMode: payload.mode, source });
}

export function createScanController(client, { operationId = () => globalThis.crypto.randomUUID() } = {}) {
  let closed = false, busy = false, timer;
  const listeners = new Set();
  const state = { ready: false, context: null, page: null, task: null, configuration: null, pending: false, error: null, uncertain: false, operationId: null, receipt: null, confirmation: null, handoff: null, handoffReviewed: false };
  function publish(patch) {
    if (closed) return;
    Object.assign(state, patch);
    for (const listener of listeners) listener({ ...state });
  }
  function available() {
    if (closed) throw Error('scan_page_closed');
    if (state.context && client.context && client.context !== state.context) {
      publish({ ready: false, task: null, confirmation: null, handoff: null, handoffReviewed: false, error: 'scan_page_context_changed_reopen_required' });
      throw Error('scan_page_context_changed_reopen_required');
    }
  }
  function fail(error) { publish({ error: error.code ?? error.message ?? 'scan_page_request_failed' }); }
  async function list(cursor) {
    available();
    try {
      const page = await client.callAction('scan_list', { limit: 20, ...(cursor ? { cursor } : {}) });
      publish({ page, error: null }); return page;
    } catch (error) { fail(error); throw error; }
  }
  async function read(taskId = state.task?.taskId) {
    available();
    if (!taskId) return;
    try {
      const task = await client.callAction('scan_status', { taskId });
      publish({ task, error: null, ...(state.task?.taskId !== task.taskId ? { confirmation: null } : {}) }); return task;
    } catch (error) { fail(error); throw error; }
  }
  async function configuration() {
    available();
    try {
      const result = await client.callAction('configuration_status', {});
      publish({ configuration: result }); return result;
    } catch (error) { publish({ configuration: null }); fail(error); throw error; }
  }
  async function mutate(name, args) {
    available();
    if (busy || state.uncertain) throw Error('scan_mutation_outcome_requires_review');
    busy = true;
    const id = operationId();
    publish({ pending: true, error: null, operationId: id, receipt: null });
    try {
      const receipt = await client.callAction(name, args, { operationId: id });
      const uncertain = receipt?.outcome?.disposition === 'uncertain';
      publish({ receipt, uncertain });
      const taskId = receipt?.taskId ?? receipt?.task?.taskId;
      if (name === 'configure_model') await configuration().catch(() => {});
      else {
        if (taskId && name !== 'scan_draft_cancel') await read(taskId).catch(() => {});
        if (name === 'scan_draft_cancel' && receipt?.outcome?.accepted) publish({ task: null });
        await list().catch(() => {});
      }
      return receipt;
    } catch (error) {
      // A transport error is not proof of rollback. Never repeat a mutation or
      // manufacture success. Read-only inspection remains available.
      publish({ uncertain: true }); fail(error); throw error;
    } finally { busy = false; publish({ pending: false }); }
  }
  const unsubscribe = client.subscribe(() => {
    if (closed || timer) return;
    timer = setTimeout(() => {
      timer = undefined;
      if (!closed) Promise.allSettled([list(), read()]);
    }, 200);
  });
  return Object.freeze({
    get state() { return { ...state }; },
    subscribe(listener) { listeners.add(listener); listener({ ...state }); return () => listeners.delete(listener); },
    async initialize() {
      const context = await client.ready; available();
      let handoff, handoffError;
      try { if (context.handoff) handoff = readScanTargetHandoff(context.handoff, context.projectId); } catch (error) { handoffError = error; }
      publish({ ready: true, context, handoff: handoff ?? null });
      await Promise.allSettled([list(), configuration()]);
      if (handoffError) fail(handoffError);
    },
    list, read, configuration,
    reviewHandoff() {
      available();
      if (!state.handoff || !state.ready || state.pending || state.uncertain) throw Error('scan_handoff_review_unavailable');
      publish({ handoffReviewed: true });
      // This explicit review only fills the existing draft form. It makes no
      // RPC, grants nothing and cannot create or start a task automatically.
      return { title: '已审核的资产线索', targets: [...state.handoff.targets], scanMode: state.handoff.scanMode };
    },
    create: input => mutate('scan_draft_create', input),
    configure: input => mutate('configure_model', input),
    act(name) {
      if (!['scan_start', 'scan_pause', 'scan_resume', 'scan_retry', 'scan_draft_cancel'].includes(name) || !state.task?.taskId) return Promise.reject(Error('scan_task_action_invalid'));
      return mutate(name, { taskId: state.task.taskId });
    },
    requestExecution(name) {
      available();
      if (!['scan_start', 'scan_resume', 'scan_retry'].includes(name) || !state.task?.taskId || busy || state.uncertain) throw Error('scan_task_action_invalid');
      publish({ confirmation: { name, taskId: state.task.taskId } });
    },
    cancelExecution() { publish({ confirmation: null }); },
    confirmExecution() {
      available();
      const confirmation = state.confirmation;
      if (!confirmation || confirmation.taskId !== state.task?.taskId) return Promise.reject(Error('scan_confirmation_stale'));
      // Clear the actual one-shot confirmation before dispatch. A double click
      // cannot submit another operation, and a task change invalidates it.
      publish({ confirmation: null });
      return mutate(confirmation.name, { taskId: confirmation.taskId });
    },
    destroy() {
      if (closed) return;
      closed = true; clearTimeout(timer); unsubscribe(); listeners.clear();
      // Closing a page detaches this observer. It never cancels the real owner.
      client.destroy();
    },
  });
}
