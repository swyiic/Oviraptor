// Host half of the Host <-> Nest end-to-end check, driven by the desktop test
// `scanning::host_loop_e2e`. It talks to the real desktop bridge with the real
// Host store; each phase runs in a fresh process to cover restarts.
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { HostStore } from '../src/store.mjs';
import { ScanningTasks, scanTaskConversation } from '../src/scanning-tasks.mjs';

const { HOST_DIR, PHASE, OVIRAPTOR_SCAN_BRIDGE_ORIGIN: origin, OVIRAPTOR_SCAN_BRIDGE_TOKEN: token } = process.env;
const store = new HostStore(join(HOST_DIR, 'host.sqlite'));
const tasks = new ScanningTasks({ origin, token });
const project = store.projects().find(item => item.externalId === '1') ?? store.createProject('desktop-1', { externalId: '1' });
const other = store.projects().find(item => item.externalId === '2') ?? store.createProject('desktop-2', { externalId: '2' });
const rejected = async call => { try { await call(); return false; } catch { return true; } };
const count = id => store.messages(id, project.id).length;
const out = {};

if (PHASE === 'bind') {
  const conversation = store.createConversation(project.id);
  const elsewhere = store.createConversation(other.id);
  out.conversationId = conversation.id;
  await tasks.call(store, project.id, 'scan_bind', { taskId: 'scan-1' }, randomUUID(), { hostConversationId: conversation.id });
  out.forgedRejected = await rejected(() => tasks.call(store, project.id, 'scan_bind', { taskId: 'scan-1', conversationId: `host:${elsewhere.id}` }, randomUUID()));
  out.crossProjectRejected = await rejected(() => tasks.call(store, other.id, 'scan_bind', { taskId: 'scan-1' }, randomUUID(), { hostConversationId: elsewhere.id }));
  await tasks.sync(store, project.id);
  await tasks.sync(store, other.id);
  out.messages = count(conversation.id);
  out.otherMessages = store.messages(elsewhere.id, other.id).length;
} else {
  const conversationId = scanTaskConversation(store, project.id, 'scan-1');
  out.conversationId = conversationId;
  const before = count(conversationId);
  await tasks.sync(store, project.id);
  await tasks.sync(store, project.id);
  out.before = before;
  out.messages = count(conversationId);
  out.lastText = store.messages(conversationId, project.id).at(-1)?.content ?? '';
}
store.close();
process.stdout.write(JSON.stringify(out));
