import { randomUUID } from 'node:crypto';
import { CertificateTasks } from './certificate-tasks.mjs';
import { hostError } from './store.mjs';
import { scrubbedModelText } from './quality-prompt.mjs';
// The App SDK's bounded schema subset has no conditional fields. Validate the
// section contract before calling the owner as well as at its Rust boundary.
export function validateScanningResultsArguments(args) {
 const section=args.section??'targets',has=key=>Object.hasOwn(args,key);
 if(args.kind!==undefined&&(typeof args.kind!=='string'||!/^[a-z0-9_]{1,80}$/.test(args.kind))
  ||section==='evidence'&&(!Number.isSafeInteger(args.recordId)||args.recordId<1||has('kind')||has('afterId')||has('limit'))
  ||section==='findings'&&has('recordId')
  ||section==='targets'&&(has('kind')||has('recordId')))throw hostError('host_tool_input_invalid',400);
 return args;
}
export const SCANNING_TOOLS = new Map([
 ['scan_from_assets','from_assets'],['scan_list','list'],['scan_status','status'],['scan_preflight','preflight'],['scan_recovery','recovery'],['scan_results','results'],
 ['scan_start','start'],['scan_pause','pause'],['scan_resume','resume'],['scan_continue_targets','continue_targets'],['scan_bind','bind'],['scan_reassociate_binding','reassociate_binding'],['scan_command','command'],['scan_command_ack','command_ack'],['scan_review','review'],['scan_verify','verify'],['scan_retry_delivery','retry_delivery'],
]);
function ensureScanInbox(store) {
 store.db.exec(`CREATE TABLE IF NOT EXISTS scan_result_inbox(
  event_id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  created_at TEXT NOT NULL
 );
 CREATE TABLE IF NOT EXISTS scan_task_conversations(
  project_id INTEGER NOT NULL,
  task_id TEXT NOT NULL,
  conversation_id TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  bound INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY(project_id, task_id)
 )`);
 const columns = new Set(store.db.prepare('PRAGMA table_info(scan_task_conversations)').all().map(row => row.name));
 if (!columns.has('bound')) store.db.exec('ALTER TABLE scan_task_conversations ADD COLUMN bound INTEGER NOT NULL DEFAULT 1');
 // Correlation ids name the execution unit an event came from; rows stored
 // before they existed read back as empty strings.
 const inbox = new Set(store.db.prepare('PRAGMA table_info(scan_result_inbox)').all().map(row => row.name));
 for (const column of ['kind', ...INBOX_CORRELATION.map(([, name]) => name)]) {
  if (!inbox.has(column)) store.db.exec(`ALTER TABLE scan_result_inbox ADD COLUMN ${column} TEXT NOT NULL DEFAULT ''`);
 }
}

const INBOX_CORRELATION = [['assignmentId', 'assignment_id'], ['workerId', 'worker_id'], ['targetKey', 'target_key'], ['runId', 'run_id']];

/** Correlation ids of a delivery, bounded and scrubbed like the text they accompany. */
function deliveryCorrelation(delivery) {
 const out = {};
 for (const [field] of INBOX_CORRELATION) {
  const value = typeof delivery?.[field] === 'string' ? delivery[field].slice(0, 512) : '';
  out[field] = field === 'targetKey' && value ? scrubbedModelText(value) : value;
 }
 return out;
}

/**
 * Stored deliveries of one conversation, oldest first, with their correlation
 * ids; `assignmentId` narrows them to one execution unit.
 */
export function scanDeliveryEvents(store, conversationId, { assignmentId } = {}) {
 ensureScanInbox(store);
 const rows = store.db.prepare(`SELECT event_id, task_id, message_id, kind, assignment_id, worker_id, target_key, run_id, created_at
  FROM scan_result_inbox WHERE conversation_id=? AND (?='' OR assignment_id=?) ORDER BY created_at, rowid`)
  .all(String(conversationId), String(assignmentId ?? ''), String(assignmentId ?? ''));
 return rows.map(row => ({
  eventId: row.event_id, taskId: row.task_id, messageId: row.message_id, kind: row.kind,
  assignmentId: row.assignment_id, workerId: row.worker_id, targetKey: row.target_key, runId: row.run_id, createdAt: row.created_at,
 }));
}

/** The recorded conversation for a task, whether or not it still exists, and whether the desktop confirmed the binding. */
export function scanTaskMapping(store, projectId, taskId) {
 ensureScanInbox(store);
 const row = store.db.prepare('SELECT conversation_id AS conversationId, bound FROM scan_task_conversations WHERE project_id=? AND task_id=?').get(Number(projectId), String(taskId));
 return row ? { conversationId: row.conversationId, bound: Boolean(row.bound) } : undefined;
}

/** The Host conversation a Nest task reports to, while that conversation exists. */
export function scanTaskConversation(store, projectId, taskId) {
 const row = scanTaskMapping(store, projectId, taskId);
 if (!row) return undefined;
 try { return store.conversation(row.conversationId, projectId).id; } catch { return undefined; }
}

export function rememberScanTaskConversation(store, projectId, taskId, conversationId, bound = true) {
 ensureScanInbox(store);
 store.db.prepare(`INSERT INTO scan_task_conversations(project_id,task_id,conversation_id,updated_at,bound) VALUES(?,?,?,?,?)
  ON CONFLICT(project_id,task_id) DO UPDATE SET conversation_id=excluded.conversation_id, updated_at=excluded.updated_at, bound=excluded.bound`)
  .run(Number(projectId), String(taskId), String(conversationId), new Date().toISOString(), bound ? 1 : 0);
}

/** Projects that have at least one task reporting to a Host conversation. */
export function scanDeliveryProjects(store) {
 ensureScanInbox(store);
 return store.db.prepare('SELECT DISTINCT project_id AS projectId FROM scan_task_conversations').all().map(row => row.projectId);
}

/**
 * Persist one bridge delivery into the Host conversation. The same event id is
 * not shown twice. A conversation that no longer exists in this Host project is
 * refused, so a delivery can never land in another project's conversation.
 */
export function inboxScanDelivery(store, delivery, projectId) {
 const eventId = String(delivery?.eventId ?? '');
 const conversationId = String(delivery?.conversationId ?? '');
 const taskId = String(delivery?.taskId ?? '');
 if (!eventId || !conversationId || !taskId) return { replayed: false, dropped: true };
 ensureScanInbox(store);
 const existing = store.db.prepare('SELECT message_id FROM scan_result_inbox WHERE event_id=?').get(eventId);
 if (existing) return { replayed: true, messageId: existing.message_id, dropped: false };
 try { store.conversation(conversationId, projectId); } catch { return { replayed: false, dropped: true, rejected: true }; }
 const text = scrubbedModelText(String(delivery.text ?? ''));
 if (!text) return { replayed: false, dropped: true };
 const correlation = deliveryCorrelation(delivery);
 const kind = typeof delivery.kind === 'string' ? delivery.kind.slice(0, 80) : '';
 store.db.exec('BEGIN IMMEDIATE');
 try {
  const again = store.db.prepare('SELECT message_id FROM scan_result_inbox WHERE event_id=?').get(eventId);
  if (again) { store.db.exec('COMMIT'); return { replayed: true, messageId: again.message_id, dropped: false }; }
  const message = store.addMessage(conversationId, 'assistant', text);
  // Marked so turn bookkeeping (the reply of a run) never mistakes it for one.
  store.finishMessage(message.id, text, 'completed', { stopReason: 'scan_delivery' });
  store.db.prepare('INSERT INTO scan_result_inbox(event_id,conversation_id,task_id,message_id,created_at,kind,assignment_id,worker_id,target_key,run_id) VALUES(?,?,?,?,?,?,?,?,?,?)')
   .run(eventId, conversationId, taskId, message.id, message.createdAt, kind, correlation.assignmentId, correlation.workerId, correlation.targetKey, correlation.runId);
  store.db.exec('COMMIT');
  return { replayed: false, dropped: false, messageId: message.id, conversationId, content: text, createdAt: message.createdAt, eventId, taskId, kind, correlation };
 } catch (error) {
  store.db.exec('ROLLBACK');
  throw error;
 }
}

export class ScanningTasks extends CertificateTasks {
 constructor(options={}){super({origin:process.env.OVIRAPTOR_SCAN_BRIDGE_ORIGIN??null,token:process.env.OVIRAPTOR_SCAN_BRIDGE_TOKEN??null,requestTimeoutMs:60000,...options,tools:SCANNING_TOOLS,route:'/scan-task',kind:'scan',readOnlyTools:[...(options.readOnlyTools??['scan_list','scan_status','scan_preflight','scan_recovery','scan_results']),'scan_deliveries']});
  /** Called with `{conversationId, messageId}` for each newly stored delivery. */
  this.onDelivered = () => {};
 }
 async call(store, projectId, name, args, operationId, context = {}) {
  if (name === 'scan_reassociate_binding') args = this.#reassociation(store, projectId, args, context);
  const result = await super.call(store, projectId, name, args, operationId, context);
  if (context.hostConversationId && ['scan_bind','scan_reassociate_binding'].includes(name) && typeof args?.taskId === 'string')
   rememberScanTaskConversation(store, projectId, args.taskId, context.hostConversationId);
  return result;
 }
 /**
  * A binding moves to the calling conversation only away from a conversation
  * this Host knows is gone. The desktop accepts the move from a Host
  * conversation only with that conversation named as gone.
  */
 #reassociation(store, projectId, args, context) {
  const stop = code => hostError(code, 409, { outcomeUnknown: false, phase: 'not_started', retryable: false });
  if (!context.hostConversationId || typeof args?.taskId !== 'string') throw stop('host_scan_binding_destination_unverified');
  const mapping = scanTaskMapping(store, projectId, args.taskId);
  if (!mapping) throw stop('host_scan_binding_source_unknown');
  if (mapping.conversationId === context.hostConversationId) throw stop('host_scan_binding_already_here');
  let alive = true;
  try { store.conversation(mapping.conversationId); } catch { alive = false; }
  if (alive) throw stop('host_scan_binding_source_active');
  return { ...args, hostSourceGone: mapping.conversationId };
 }
 /** Binds a Host conversation the Host itself chose for a task (no model involved). */
 async bindConversation(store, projectId, taskId, conversationId) {
  const result = await super.call(store, projectId, 'scan_bind', { taskId }, randomUUID(), { hostConversationId: conversationId });
  rememberScanTaskConversation(store, projectId, taskId, conversationId);
  return result;
 }
 /**
  * Pulls pending outbox events for one project, stores them in their Host
  * conversations and acknowledges what was stored or refused. Events that
  * fail to store stay pending on the desktop and are pulled again later.
  */
 async sync(store, projectId) {
  const result = await super.call(store, projectId, 'scan_deliveries', { limit: 32 }, randomUUID(), { internalAction: 'deliveries' });
  const batch = Array.isArray(result.structuredContent?.deliveries) ? result.structuredContent.deliveries : [];
  return batch.length ? this.#store(store, projectId, batch) : [];
 }
 async #store(store, projectId, batch) {
  const receipts = [], acks = [];
  for (const item of batch) {
   if (typeof item?.eventId !== 'string' || !item.eventId) continue;
   let receipt;
   try { receipt = inboxScanDelivery(store, item, projectId); } catch { receipts.push({ dropped: true, failed: true }); continue; }
   receipts.push(receipt);
   if (receipt.messageId && !receipt.replayed) this.onDelivered({ conversationId: receipt.conversationId, messageId: receipt.messageId, content: receipt.content, createdAt: receipt.createdAt, eventId: receipt.eventId, taskId: receipt.taskId, kind: receipt.kind, correlation: receipt.correlation });
   if (receipt.messageId || receipt.dropped) acks.push({ eventId: String(item.eventId), outcome: receipt.messageId ? 'delivered' : 'rejected' });
  }
  if (acks.length) {
   try { await super.call(store, projectId, 'scan_deliveries_ack', { acks }, randomUUID(), { internalAction: 'deliveries_ack' }); }
   catch { /* Unacknowledged events are pulled again; the inbox keeps them single. */ }
  }
  return receipts;
 }
}
