import { createPageClient } from '../../packages/app-sdk/src/page-client.mjs';
import { createScanController } from './native-controller.mjs';

const client = createPageClient({ appId: 'org.oviraptor.scanning' });
const controller = createScanController(client);
const el = id => document.getElementById(id);
const showFailure = error => { el('error').hidden = false; el('error').textContent = error.code ?? error.message ?? '操作失败'; };
const invoke = action => Promise.resolve().then(action).catch(showFailure);

controller.subscribe(state => {
  el('project').textContent = state.context ? `${state.context.projectName || '项目'} · ${state.context.projectId}` : '正在等待可信项目上下文…';
  el('error').hidden = !state.error; el('error').textContent = state.error || '';
  el('uncertain').hidden = !state.uncertain;
  el('execution-confirmation').hidden = !state.confirmation;
  const labels = { scan_start: '启动', scan_resume: '续跑', scan_retry: '重新执行' };
  el('execution-confirmation-description').textContent = state.confirmation ? `${labels[state.confirmation.name]}任务 ${state.confirmation.taskId}` : '';
  el('operation').textContent = state.operationId ? `最近操作编号：${state.operationId}` : '';
  for (const button of document.querySelectorAll('[data-mutation]')) button.disabled = !state.ready || state.pending || state.uncertain || (button.dataset.action && !state.task);
  el('task').textContent = state.task ? JSON.stringify(state.task, null, 2) : '请选择或创建任务';
  el('receipt').hidden = !state.receipt; el('receipt').textContent = state.receipt ? JSON.stringify(state.receipt, null, 2) : '';
  el('configuration').textContent = state.configuration ? (state.configuration.configured ? '已配置；启动仍需通过原引擎准入' : '尚未配置可解析的模型') : '配置状态未知';
  el('tasks').replaceChildren();
  if (state.page) {
    el('list-summary').textContent = `当前项目服务任务共 ${state.page.total} 条，本页 ${state.page.returned} 条；读取时间 ${state.page.asOf}`;
    for (const task of state.page.items) {
      const item = document.createElement('li'), button = document.createElement('button');
      button.type = 'button'; button.textContent = `${task.title || task.taskId} · ${task.status} · attempt ${task.attemptNumber}`;
      button.addEventListener('click', () => invoke(() => controller.read(task.taskId)));
      item.append(button); el('tasks').append(item);
    }
  }
  el('next').disabled = !state.page?.hasMore;
});
const inputValue = (container, name) => container.querySelector(`[name="${name}"]`).value;
const inputsValid = container => [...container.querySelectorAll('input,textarea,select')].every(input => input.checkValidity());
el('create-draft').addEventListener('click', () => {
  const fields = el('draft-form');
  if (!inputsValid(fields)) { showFailure(Error('请填写任务名和目标 URL')); return; }
  const targets = inputValue(fields, 'targets').split(/\r?\n/u).map(value => value.trim()).filter(Boolean);
  invoke(() => controller.create({ title: inputValue(fields, 'title'), targets, scanMode: inputValue(fields, 'scanMode') }));
});
el('save-configuration').addEventListener('click', () => {
  const fields = el('configuration-form');
  if (!inputsValid(fields)) { showFailure(Error('请填写完整且有效的模型配置')); return; }
  const configuration = Object.fromEntries(['llm', 'deployment', 'apiBase', 'apiKey'].map(name => [name, inputValue(fields, name)]));
  // Do not keep credentials in page/controller state or receipt history.
  fields.querySelector('[name="apiKey"]').value = '';
  invoke(() => controller.configure(configuration));
});
for (const button of document.querySelectorAll('[data-action]')) button.addEventListener('click', () => {
  const name = button.dataset.action;
  if (['scan_start', 'scan_resume', 'scan_retry'].includes(name)) {
    invoke(() => controller.requestExecution(name)); return;
  }
  invoke(() => controller.act(name));
});
el('confirm-execution').addEventListener('click', () => invoke(() => controller.confirmExecution()));
el('cancel-execution').addEventListener('click', () => controller.cancelExecution());
el('refresh').addEventListener('click', () => invoke(() => controller.list()));
el('next').addEventListener('click', () => invoke(() => controller.list(controller.state.page?.nextCursor)));
el('read-task').addEventListener('click', () => invoke(() => controller.read()));
window.addEventListener('pagehide', () => controller.destroy(), { once: true });
invoke(() => controller.initialize());
