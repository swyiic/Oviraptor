import type { ObjectDirective, VNode } from 'vue';

export type RefreshFeedbackOptions = { busy?: boolean; error?: unknown; scope?: string | number };
type Click = (...args: unknown[]) => unknown;
type State = {
  el: HTMLButtonElement; epoch: number; scope?: string | number; options: RefreshFeedbackOptions; alive: boolean;
  running: boolean; pending: boolean; settled: boolean; failed: boolean; confirmed: boolean;
  desiredDisabled: boolean; started: number; originalLabel: string | null;
  label: string; title: string | null; resultClass: string; finishTimer?: ReturnType<typeof setTimeout>;
  clearTimer?: ReturnType<typeof setTimeout>;
};
const states = new WeakMap<HTMLButtonElement, State>();
const original = Symbol('original-refresh-handler');
const VISIBLE_MS = 360;
const MESSAGE_MS = 1400;

function announce(state: State, text: string) {
  const doc = state.el.ownerDocument;
  if (!doc?.body) return;
  let live = doc.querySelector('[data-refresh-announcement]');
  if (!live) {
    live = doc.createElement('span');
    live.setAttribute('data-refresh-announcement', '');
    live.setAttribute('role', 'status');
    live.setAttribute('aria-live', 'polite');
    live.className = 'refresh-announcement';
    doc.body.appendChild(live);
  }
  live.textContent = `${state.label}：${text}`;
}
function reset(state: State) {
  state.el.classList.remove('refresh-feedback--done', 'refresh-feedback--error', 'refresh-feedback--ack');
  state.el.removeAttribute('data-refresh-message'); state.resultClass = '';
  if (state.originalLabel === null) state.el.removeAttribute('aria-label');
  else state.el.setAttribute('aria-label', state.originalLabel);
  if (state.title === null) state.el.removeAttribute('title');
  else state.el.setAttribute('title', state.title);
}
function complete(state: State) {
  if (!state.alive || !state.running || state.pending || state.options.busy) return;
  clearTimeout(state.finishTimer);
  const remaining = VISIBLE_MS - (Date.now() - state.started);
  if (remaining > 0) {
    state.finishTimer = setTimeout(() => complete(state), remaining);
    return;
  }
  state.running = false;
  state.el.disabled = state.desiredDisabled;
  state.el.removeAttribute('aria-busy');
  state.el.classList.remove('refresh-feedback--pending');
  const failed = state.failed || Boolean(state.options.error), confirmed = state.confirmed;
  const english = !/[\u3400-\u9fff]/.test(state.label);
  const text = failed ? (english ? 'Refresh failed' : '刷新失败')
    : confirmed ? (english ? 'Updated' : '已刷新') : (english ? 'Requested' : '已发起');
  state.resultClass = failed ? 'refresh-feedback--error' : confirmed ? 'refresh-feedback--done' : 'refresh-feedback--ack';
  state.el.classList.add(state.resultClass);
  state.el.setAttribute('data-refresh-message', state.el.textContent?.trim() ? `${failed ? '!' : confirmed ? '✓' : '↻'} ${text}` : failed ? '!' : confirmed ? '✓' : '↻');
  state.el.setAttribute('aria-label', `${state.label}，${text}`);
  state.el.setAttribute('title', `${state.label} · ${text}`);
  announce(state, text);
  state.clearTimer = setTimeout(() => { if (state.alive && !state.running) reset(state); }, MESSAGE_MS);
}
function begin(state: State) {
  clearTimeout(state.finishTimer); clearTimeout(state.clearTimer); reset(state);
  state.running = true; state.pending = false; state.settled = false; state.failed = false; state.confirmed = false;
  state.started = Date.now();
  state.el.disabled = true;
  state.el.classList.add('refresh-feedback--pending');
  state.el.setAttribute('aria-busy', 'true');
  const text = /[\u3400-\u9fff]/.test(state.label) ? '正在刷新' : 'Refreshing';
  state.el.setAttribute('aria-label', `${state.label}，${text}`);
  state.el.setAttribute('title', text);
}
function wire(state: State, vnode: VNode) {
  const props = vnode.props;
  if (!props?.onClick) return;
  state.desiredDisabled = props.disabled === '' || Boolean(props.disabled);
  const supplied = props.onClick as Click & { [original]?: Click };
  const handler = supplied[original] || supplied;
  const wrapped = function (this: unknown, ...args: unknown[]) {
    if (state.running || state.desiredDisabled) return;
    begin(state);
    const epoch = ++state.epoch;
    let result: unknown;
    try {
      result = Array.isArray(handler) ? handler.map(fn => fn.apply(this, args)) : handler.apply(this, args);
    } catch (error) {
      state.failed = true; state.settled = true; complete(state); throw error;
    }
    const promise = Array.isArray(result)
      ? result.some(value => value && typeof value.then === 'function') ? Promise.all(result) : undefined
      : result && typeof (result as PromiseLike<unknown>).then === 'function' ? result : undefined;
    if (promise) {
      state.pending = true; state.confirmed = true;
      return Promise.resolve(promise).then(value => {
        if (!state.alive || epoch !== state.epoch) return value;
        state.pending = false; state.settled = true;
        state.failed = value === false || Array.isArray(value) && value.includes(false);
        complete(state); return value;
      }, error => {
        if (!state.alive || epoch !== state.epoch) throw error;
        state.pending = false; state.settled = true; state.failed = true;
        complete(state); throw error;
      });
    }
    // emit-only controls bind their owner's busy flag. If no lifecycle is
    // supplied, acknowledge the click without pretending that the read finished.
    queueMicrotask(() => { if (state.alive && epoch === state.epoch) { state.settled = true; complete(state); } });
    return result;
  } as Click & { [original]?: Click };
  wrapped[original] = handler;
  // Vue caches this props object before directive hooks. Update the object
  // in place so the actual DOM listener and disabled prop receive the wrapper.
  props.onClick = wrapped;
  props.disabled = state.running || state.desiredDisabled;
}

export const refreshFeedback: ObjectDirective<HTMLButtonElement, RefreshFeedbackOptions | undefined> = {
  created(el, binding, vnode) {
    // Non-DOM SFC harnesses exercise their original handlers separately.
    if (!el.classList || !el.setAttribute) return;
    const state: State = {
      el, epoch: 0, scope: binding.value?.scope, options: binding.value || {}, alive: true, running: false,
      pending: false, settled: false, failed: false, confirmed: false, desiredDisabled: false, started: 0,
      originalLabel: el.getAttribute('aria-label'),
      label: String(vnode.props?.['aria-label'] || vnode.props?.title || el.textContent?.trim() || '刷新'),
      title: el.getAttribute('title'), resultClass: '',
    };
    states.set(el, state); el.classList.add('refresh-feedback');
    wire(state, vnode);
  },
  mounted(el) {
    const state = states.get(el); if (!state) return;
    el.classList.add('refresh-feedback');
    state.originalLabel = el.getAttribute('aria-label');
    state.title = el.getAttribute('title');
    state.label = state.originalLabel || state.title || el.textContent?.trim() || '刷新';
    if (!el.querySelector('svg')) el.classList.add('refresh-feedback--text');
  },
  beforeUpdate(el, binding, vnode) {
    const state = states.get(el); if (!state) return;
    if (state.scope !== binding.value?.scope) {
      state.epoch++; state.scope = binding.value?.scope;
      clearTimeout(state.finishTimer); clearTimeout(state.clearTimer);
      state.running = false; state.pending = false; state.settled = false;
      el.classList.remove('refresh-feedback--pending'); el.removeAttribute('aria-busy'); reset(state);
    }
    state.options = binding.value || {};
    if (state.running && state.options.busy) state.confirmed = true;
    wire(state, vnode);
  },
  updated(el) {
    const state = states.get(el);
    if (state) { el.disabled = state.running || state.desiredDisabled; el.classList.add('refresh-feedback'); if (state.running) el.classList.add('refresh-feedback--pending'); if (state.resultClass) el.classList.add(state.resultClass); }
    if (state?.running && state.settled) complete(state);
  },
  beforeUnmount(el) {
    const state = states.get(el); if (!state) return;
    state.alive = false; clearTimeout(state.finishTimer); clearTimeout(state.clearTimer);
    states.delete(el);
  },
};
