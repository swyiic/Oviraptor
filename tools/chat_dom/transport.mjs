import { reactive } from 'vue';

export const metrics = reactive({ reads: 0, active: 0, peak: 0, hold: false, failNext: false,
  pending: [], calls: [], errors: [] });
const listeners = new Set();
const pending = [];
const states = new Map();
const views = new Map();
const scan = (id) => ({ id, projectId: 1, taskName: `夹具任务 ${id}`, updatedAt: id,
  status: 'completed', latestAttemptNumber: 1 });
const makeItem = (id, sequence, attempt) => ({ id: `${id}-${attempt}-${sequence}`, sequence,
  timestamp: new Date(1700000000000 + sequence * 1000).toISOString(), eventType: 'mailbox_message',
  fromRole: sequence % 2 ? 'source_analyst' : 'evidence_reviewer', toRole: 'coordinator',
  messageKind: 'fixture_message', status: 'persisted', deliveryState: 'delivered', ackState: 'acknowledged',
  summary: `模拟消息 ${id} / attempt ${attempt} / #${sequence}（仅验证 DOM，不是执行证明）` });
for (const id of ['A', 'B']) states.set(id, { scanId: id, status: 'completed', attemptNumber: 1,
  latestSequence: 1, timeline: [makeItem(id, 1, 1)], directiveDrafts: [],
  stopDiagnostic: { code: 'completed_with_gaps', obligations: [], nextAction: '模拟任务，不执行扫描' } });

export async function listen(name, listener) {
  if (name !== 'nest://collaboration-event') throw new Error('fixture_unsupported_event');
  listeners.add(listener);
  return () => listeners.delete(listener);
}
function notify(state) {
  for (const listener of listeners) listener({ payload: { scanId: state.scanId,
    attemptNumber: state.attemptNumber, sequence: state.latestSequence } });
}
export function burst(id, count = 200) {
  const state = states.get(id);
  for (let i = 0; i < count; i++) {
    state.timeline.push(makeItem(id, ++state.latestSequence, state.attemptNumber));
    notify(state);
  }
}
export function newAttempt(id) {
  const state = states.get(id);
  state.attemptNumber++;
  state.latestSequence = 1;
  state.timeline = [makeItem(id, 1, state.attemptNumber)];
  notify(state);
}
export function malformed() {
  for (const payload of [null, {}, { scanId: 'A', sequence: -1, attemptNumber: 1 }]) {
    for (const listener of listeners) listener({ payload });
  }
}
export function release(last = false) {
  const index = last ? pending.length - 1 : 0;
  const item = pending.splice(index, 1)[0];
  if (item) {
    metrics.pending.splice(index, 1);
    item.resolve();
  }
}
const emptyView = (scanId, attemptNumber) => ({ scanId, attemptNumber, revision: 0,
  selectedThread: '', allReadSequence: 0, threadReadSequences: {} });
let selection = { projectId: 1, revision: 0, selectedScanId: null,
  selectedScan: null, selectionUnavailable: false };
const unavailable = async () => { throw new Error('fixture_action_unavailable_no_execution'); };
export const sentinelApi = {
  listSentinelScans: async () => ['A', 'B'].map(scan),
  getAgentDialogTask: async (id) => scan(id),
  getAgentDialogSelection: async () => structuredClone(selection),
  saveAgentDialogSelection: async (input) => {
    if (selection.revision !== input.expectedRevision) throw new Error('dialog_selection_revision_conflict');
    selection = { projectId: 1, revision: selection.revision + 1,
      selectedScanId: input.scanId, selectedScan: scan(input.scanId), selectionUnavailable: false };
    return structuredClone(selection);
  },
  getAgentDialogView: async (id, attempt) => structuredClone(views.get(`${id}:${attempt}`) || emptyView(id, attempt)),
  saveAgentDialogView: async (input) => {
    const key = `${input.scanId}:${input.attemptNumber}`;
    const view = structuredClone(views.get(key) || emptyView(input.scanId, input.attemptNumber));
    if (view.revision !== input.expectedRevision) throw new Error('dialog_view_revision_conflict');
    view.revision++;
    view.selectedThread = input.selectedThread;
    if (input.markReadThrough !== undefined) {
      if (input.selectedThread) view.threadReadSequences[input.selectedThread] = input.markReadThrough;
      else view.allReadSequence = input.markReadThrough;
    }
    views.set(key, view);
    return structuredClone(view);
  },
  getNativeScanTimelinePage: async (id, attemptNumber, beforeSequence) => {
    const state = states.get(id);
    if (state?.attemptNumber !== attemptNumber) throw new Error('fixture_attempt_changed');
    const earlier = state.timeline.filter((item) => item.sequence < beforeSequence);
    const timeline = structuredClone(earlier.slice(-100));
    return { scanId: id, attemptNumber, beforeSequence, timeline,
      timelineBeforeSequence: timeline[0]?.sequence ?? 0, hasEarlierTimeline: earlier.length > timeline.length };
  },
  getNativeScanStatus: async (id, after, attempt) => {
    const number = ++metrics.reads;
    metrics.active++;
    metrics.peak = Math.max(metrics.peak, metrics.active);
    const snapshot = structuredClone(states.get(id));
    snapshot.isIncremental = after !== undefined && attempt === snapshot.attemptNumber;
    if (snapshot.isIncremental) {
      const pending = snapshot.timeline.filter((item) => item.sequence > after);
      snapshot.timeline = pending.slice(0, 100);
      snapshot.timelineHasMore = pending.length > 100;
      if (snapshot.timelineHasMore) snapshot.latestSequence = snapshot.timeline.at(-1).sequence;
    } else {
      snapshot.timeline = snapshot.timeline.slice(-100);
      snapshot.timelineHasMore = false;
    }
    snapshot.timelineBeforeSequence = snapshot.timeline[0]?.sequence ?? 0;
    snapshot.hasEarlierTimeline = snapshot.timelineBeforeSequence > 1;
    const fail = metrics.failNext;
    metrics.failNext = false;
    metrics.calls.push(`#${number} ${id} cursor=${after ?? 'full'} snapshot=${snapshot.attemptNumber}/${snapshot.latestSequence}`);
    if (metrics.calls.length > 100) metrics.calls.shift();
    try {
      if (metrics.hold) await new Promise((resolve) => {
        pending.push({ resolve });
        metrics.pending.push(`#${number} ${id}`);
      });
      if (fail) throw new Error('fixture_database_read_failed');
      return snapshot;
    } finally { metrics.active--; }
  },
  draftScanDirective: unavailable, confirmScanDirective: unavailable, cancelScanDirective: unavailable,
  reconcileScanDirectiveReceipt: unavailable, reconcileHistoricalScanDirectiveReceipt: unavailable,
  previewAgentGapFollowup: unavailable,
};
