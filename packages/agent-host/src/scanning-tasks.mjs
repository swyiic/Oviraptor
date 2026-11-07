import { randomUUID } from 'node:crypto';
import { CertificateTasks } from './certificate-tasks.mjs';
import { SCANNING_APP_ID } from './legacy-bridge.mjs';
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
 // Advisory, host-handled locally (never bridged, never authorizes): it only
 // opens the operator authorization panel and records the model's proposal.
 ['request_scan_authorization','request_authorization'],
 ['scan_from_assets','from_assets'],['scan_list','list'],['scan_status','status'],['scan_preflight','preflight'],['scan_recovery','recovery'],['scan_results','results'],
 ['scan_fingerprint_search','fingerprint_search'],['scan_fingerprint_read','fingerprint_read'],
 ['scan_start','start'],['scan_pause','pause'],['scan_resume','resume'],['scan_continue_targets','continue_targets'],['scan_bind','bind'],['scan_reassociate_binding','reassociate_binding'],['scan_command','command'],['scan_command_ack','command_ack'],['scan_review','review'],['scan_verify','verify'],['scan_retry_delivery','retry_delivery'],
]);
// Canonical high-risk capability classes. Mirror of
// scan_capability_authorization.rs::HIGH_RISK_CLASS_STRINGS and
// AgentWorkbench.vue::HIGH_RISK_CLASSES — keep all three in sync. A class string
// the Rust gate does not recognize simply never matches, so the floor is closed
// either way, but the advisory tool rejects unknown classes up front.
export const SCAN_AUTH_LEGAL_CLASSES = Object.freeze(new Set([
 'code_exec', 'target_write', 'post_write', 'egress', 'internal_scope', 'persistence', 'exfil', 'waf_evasion',
]));
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
 constructor(options={}){super({origin:process.env.OVIRAPTOR_SCAN_BRIDGE_ORIGIN??null,token:process.env.OVIRAPTOR_SCAN_BRIDGE_TOKEN??null,requestTimeoutMs:60000,...options,tools:SCANNING_TOOLS,route:'/scan-task',kind:'scan',readOnlyTools:[...(options.readOnlyTools??['scan_list','scan_status','scan_preflight','scan_recovery','scan_results','scan_fingerprint_search','scan_fingerprint_read']),'scan_deliveries']});
  /** Called with `{conversationId, messageId}` for each newly stored delivery. */
  this.onDelivered = () => {};
  /** Called with the model's authorization proposal; the service turns it into a `scan.authorization.requested` event that renders the operator panel. */
  this.onAuthorizationRequested = () => {};
 }
 async call(store, projectId, name, args, operationId, context = {}) {
  // Advisory, zero-authority: opens the operator panel and records the model's
  // proposal. It is deliberately NOT bridged to the desktop and writes no
  // authorization — the attestation and the capability record are the
  // operator's, produced only by the panel's `authorize` channel.
  if (name === 'request_scan_authorization') return this.#requestAuthorization(store, projectId, args, context);
  // Just before execution, auto-apply any operator standing authorization that
  // fully covers this scan's targets, so deep tools are advertised to the run
  // without a live panel click. Best-effort: a miss or error simply leaves the
  // authorization as it is and the run falls back to the panel path.
  if (name === 'scan_start' && typeof args?.taskId === 'string') {
    try { await this.applyStandingAuthorization(store, projectId, { scanId: args.taskId, attemptNumber: args.attemptNumber }, randomUUID(), context); }
    catch { /* standing authorization is advisory sugar; never block the start */ }
  }
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
 /**
  * Advisory authorization request from the model. Validates the proposed classes
  * against the canonical set and asks the service to raise the operator panel.
  * It NEVER authorizes, NEVER signs the legal attestation, NEVER bridges to the
  * desktop and NEVER starts a scan — the deep-test tools stay undispatched until
  * the operator authorizes through the panel's operator-only channel.
  */
 #requestAuthorization(store, projectId, args, context) {
  const bad = code => hostError(code, 400, { outcomeUnknown: false, phase: 'not_started', retryable: false });
  const taskId = args?.taskId;
  if (typeof taskId !== 'string' || !taskId || taskId.length > 128) throw bad('host_scan_authorization_task_invalid');
  if (!Array.isArray(args?.proposedClasses) || args.proposedClasses.length < 1 || args.proposedClasses.length > 8
   || args.proposedClasses.some(value => typeof value !== 'string')) throw bad('host_scan_authorization_class_invalid');
  const proposedClasses = [...new Set(args.proposedClasses)].sort();
  if (proposedClasses.some(value => !SCAN_AUTH_LEGAL_CLASSES.has(value))) throw bad('host_scan_authorization_class_invalid');
  const reason = typeof args?.reason === 'string' ? args.reason.slice(0, 600) : '';
  const conversationId = context.hostConversationId ? String(context.hostConversationId) : '';
  // The panel is anchored to the Host-verified conversation, not one the model
  // named. Record the proposal and raise the panel; do not touch the authorization
  // table.
  try { this.onAuthorizationRequested({ conversationId, projectId, taskId, proposedClasses, reason }); } catch { /* the event is best-effort; the receipt stands either way */ }
  const structuredContent = { projectId, taskId, requested: true, authorizationPending: true, authorized: false,
   proposedClasses, panel: 'operator_authorization_panel',
   note: 'Operator must tick the classes and the legal-authorization checkbox and click Authorize in the panel. This tool did not authorize anything.' };
  return { content: [{ type: 'text', text: JSON.stringify(structuredContent) }], structuredContent };
 }
 /**
  * Operator-only: land a capability-class authorization for a scan attempt onto
  * the immutable desktop record, via the trusted `/scan-task` bridge. Never a
  * model tool (no entry in any model `tools/list`); it is reached only from the
  * Host's `scanAuthorizationConversationAction`, which the panel's operator click
  * drives. The desktop re-validates ownership and immutability, and
  * `set_scan_capability_authorization` itself refuses a non-empty class set
  * without the legal attestation.
  */
 async authorizeCapability(store, projectId, { scanId, attemptNumber, legalAttestation, classes, attestedBy = 'operator' }, operationId, { hostConversationId } = {}) {
  const bad = code => hostError(code, 400, { outcomeUnknown: false, phase: 'not_started', retryable: false });
  if (typeof scanId !== 'string' || !scanId || scanId.length > 128) throw bad('host_scan_authorization_task_invalid');
  if (!Array.isArray(classes) || classes.some(value => typeof value !== 'string')) throw bad('host_scan_authorization_class_invalid');
  const list = [...new Set(classes)].sort();
  if (list.some(value => !SCAN_AUTH_LEGAL_CLASSES.has(value))) throw bad('host_scan_authorization_class_invalid');
  // The same hard precondition the Rust writer enforces, asserted here too so the
  // panel can never bridge an un-attested high-risk opt-in.
  if (list.length && legalAttestation !== true) throw bad('host_scan_authorization_attestation_required');
  const attempt = attemptNumber === undefined || attemptNumber === null ? undefined
   : Number.isSafeInteger(Number(attemptNumber)) && Number(attemptNumber) > 0 ? Number(attemptNumber) : null;
  if (attempt === null) throw bad('host_scan_authorization_attempt_invalid');
  const payload = { scanId, ...(attempt !== undefined ? { attemptNumber: attempt } : {}),
   legalAttestation: Boolean(legalAttestation), classes: list, attestedBy: String(attestedBy || 'operator').slice(0, 64) };
  return super.call(store, projectId, 'scan_authorize_internal', payload, operationId, { internalAction: 'authorize_capability', hostConversationId });
 }
 /**
  * Host-triggered auto-apply of an operator standing authorization for a scan,
  * via the trusted bridge. Never a model tool and never writes authority of its
  * own: the desktop reuses the operator's prior registration only for a scan
  * whose every target the registration covers, and applies nothing otherwise.
  */
 async applyStandingAuthorization(store, projectId, { scanId, attemptNumber }, operationId, { hostConversationId } = {}) {
  if (typeof scanId !== 'string' || !scanId || scanId.length > 128) throw hostError('host_scan_authorization_task_invalid', 400, { outcomeUnknown: false, phase: 'not_started', retryable: false });
  const attempt = attemptNumber === undefined || attemptNumber === null ? undefined
   : Number.isSafeInteger(Number(attemptNumber)) && Number(attemptNumber) > 0 ? Number(attemptNumber) : null;
  if (attempt === null) throw hostError('host_scan_authorization_attempt_invalid', 400, { outcomeUnknown: false, phase: 'not_started', retryable: false });
  const payload = { scanId, ...(attempt !== undefined ? { attemptNumber: attempt } : {}) };
  return super.call(store, projectId, 'scan_apply_standing_internal', payload, operationId, { internalAction: 'apply_standing_authorization', hostConversationId });
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
   if (receipt.messageId && !receipt.replayed) this.onDelivered({ conversationId: receipt.conversationId, messageId: receipt.messageId, content: receipt.content, createdAt: receipt.createdAt, eventId: receipt.eventId, taskId: receipt.taskId, kind: receipt.kind, correlation: receipt.correlation, projectId, attemptNumber: item?.evidence?.attemptNumber });
   if (receipt.messageId || receipt.dropped) acks.push({ eventId: String(item.eventId), outcome: receipt.messageId ? 'delivered' : 'rejected' });
  }
  if (acks.length) {
   try { await super.call(store, projectId, 'scan_deliveries_ack', { acks }, randomUUID(), { internalAction: 'deliveries_ack' }); }
   catch { /* Unacknowledged events are pulled again; the inbox keeps them single. */ }
  }
  return receipts;
 }
}

/**
 * UI adapter for the in-conversation scan authorization panel, bound to a stored
 * conversation and the current access policy. Mirrors
 * `discovery-assist.mjs::discoveryConversationAction`: the operator-only
 * `authorize` action never appears in any model `tools/list` — it is reached
 * only from the panel's human click through the Host route. Hard invariants: the
 * model cannot drive this (no model tool reaches it), and it refuses a non-empty
 * class set without the operator's legal attestation.
 */
export async function scanAuthorizationConversationAction(host, conversationId, projectId, input = {}) {
  const conversation = host.store.conversation(conversationId, projectId), p = conversation.projectId;
  const app = host.store.application(SCANNING_APP_ID), identity = host.runtimeIdentity();
  const grant = host.apps.effectiveGrant(app, p, identity), policy = host.accessPolicy.view(p);
  const witness = { appRevision: app.revision, grantRevision: grant.revision, policyRevision: policy.revision, runtimeIdentity: identity };
  if (!app.enabled || host.apps.changing.has(app.id) || !host.apps.scanningTasks?.available) throw hostError('host_scan_executor_unavailable', 503);
  if (input.action === 'authorize') {
    if (Object.keys(witness).some(key => input[key] !== witness[key])) throw hostError('host_scan_authorization_stale', 409, { retryable: false });
    if (!grant.permissions.includes('investigation.execute') || grant.disclosureRuntime !== identity) throw hostError('host_tool_permission_denied', 403, { retryable: false });
    const { taskId, attemptNumber, classes, legalAttestation } = input;
    if (typeof taskId !== 'string' || !taskId || taskId.length > 128 || !Array.isArray(classes)) throw hostError('host_tool_input_invalid', 400);
    const result = await host.apps.scanningTasks.authorizeCapability(host.store, p,
      { scanId: taskId, attemptNumber, legalAttestation: legalAttestation === true, classes, attestedBy: 'operator' },
      randomUUID(), { hostConversationId: conversationId });
    // TOCTOU guard, mirroring discovery's trusted-start re-check: authority must
    // be unchanged across the write.
    const current = host.store.application(app.id), now = host.accessPolicy.view(p), checkedIdentity = host.runtimeIdentity();
    const checkedGrant = host.apps.effectiveGrant(current, p, checkedIdentity);
    if (!current.enabled || host.apps.changing.has(app.id) || current.revision !== witness.appRevision
      || checkedGrant.revision !== witness.grantRevision || now.revision !== witness.policyRevision
      || checkedIdentity !== identity || checkedGrant.disclosureRuntime !== identity
      || !checkedGrant.permissions.includes('investigation.execute')) throw hostError('host_tool_authority_changed', 403);
    host.emit({ type: 'scan.authorization.updated', conversationId, projectId: p, taskId });
    return { ...(result?.structuredContent ?? result), confirmationWitness: witness };
  }
  if (input.action !== undefined && input.action !== 'status') throw hostError('host_tool_input_invalid', 400);
  if (!grant.permissions.includes('investigation.read') || grant.disclosureRuntime !== identity) throw hostError('host_tool_permission_denied', 403, { retryable: false });
  return { confirmationWitness: witness, legalClasses: [...SCAN_AUTH_LEGAL_CLASSES] };
}
