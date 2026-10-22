import {node} from './dom.js';

// Own the lifetime of a manual read, including views rebuilt while it is pending.
// The action and its existing controller remain responsible for errors and data.
export function createRefreshFeedback(action, {
  changed = () => {}, failed = () => false, disabled = () => false,
  scope = () => '', minimumMs = 300, resultMs = 1400,
  now = () => Date.now(), schedule = setTimeout, cancel = clearTimeout,
} = {}) {
  const state = {phase:'idle',pending:false};
  let element, attributes = {}, label = '', scopeAtStart, resultTimer;
  function phase() { return state.phase !== 'idle' && scope() !== scopeAtStart ? 'idle' : state.phase; }
  function paint() {
    if (!element) return;
    const current = phase();
    element.className = `${attributes.class || ''} refresh-button`.trim();
    element.disabled = Boolean(attributes.disabled) || disabled() || state.pending;
    element.setAttribute('data-refresh-state',current);
    element.setAttribute('aria-busy',String(current === 'loading'));
    const result = current === 'success' ? 'Updated' : current === 'error' ? 'Refresh failed' : '';
    const icon = node('span',{class:`refresh-indicator ${current === 'loading' ? 'is-spinning' : ''}`,'aria-hidden':'true'},current === 'success' ? '✓' : current === 'error' ? '!' : current === 'loading' ? '' : '↻');
    element.replaceChildren(icon,node('span',{},current === 'loading' ? 'Refreshing…' : label),node('span',{class:'refresh-button-result',role:'status','aria-live':'polite'},result));
  }
  function publish() { paint(); changed(); }
  async function run() {
    if (state.pending || disabled()) return false;
    cancel(resultTimer);scopeAtStart = scope();
    state.pending = true;state.phase = 'loading';const started = now();publish();
    let successful = false;
    try { successful = await action() !== false && !failed(); }
    catch { successful = false; }
    const remaining = minimumMs - (now() - started);
    if (remaining > 0) await new Promise(resolve => schedule(resolve,remaining));
    state.pending = false;
    state.phase = scope() === scopeAtStart ? successful ? 'success' : 'error' : 'idle';
    publish();
    if (state.phase !== 'idle') resultTimer = schedule(() => {state.phase = 'idle';publish();},resultMs);
    return successful && scope() === scopeAtStart;
  }
  function button(text, attrs = {}) {
    label = text;attributes = attrs;
    element = node('button',{type:'button',...attrs,onClick:run});paint();return element;
  }
  function reset() { cancel(resultTimer);state.phase = 'idle';paint(); }
  function dispose() { cancel(resultTimer);element = undefined; }
  return {state,run,button,sync:paint,reset,dispose};
}
