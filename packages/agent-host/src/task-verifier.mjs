import { createHash } from 'node:crypto';
import { isAbsolute, relative, join, sep } from 'node:path';
import { hostError } from './store.mjs';
import { authorizedReadCapabilities } from './subagents.mjs';

const criterionId = { type: 'string', pattern: '^[a-zA-Z0-9_.:-]{1,80}$' };
const fileCriterion = (kind, extra = {}) => ({ type: 'object', required: ['id', 'kind', 'artifactId', ...Object.keys(extra)], properties: { id: criterionId, kind: { type: 'string', enum: [kind] }, artifactId: { type: 'string', description: 'ID of an artifact already registered by host_task_update; not its file path.' }, ...extra }, additionalProperties: false });
const toolCriterion = kind => ({ type: 'object', required: ['id', 'kind', 'toolName', 'path', 'expected'], properties: { id: criterionId, kind: { type: 'string', enum: [kind] }, toolName: { type: 'string', description: 'Exact Host MCP name of an authorized read tool, not a provider wire alias.' }, path: { type: 'array', maxItems: 8, items: { type: 'string' }, description: 'JSON field keys, for example ["total"].' }, expected: { type: ['string', 'number', 'boolean', 'null'] }, ...(kind === 'tool_read_equals' ? { arguments: { type: 'object' } } : { evidenceId: { type: 'string' } }) }, additionalProperties: false });
export const VERIFIER_TOOL_SPECS = [
  { name: 'host_task_criteria', description: 'Declare explicit finite acceptance criteria from the parent goal. Choose exactly one schema per criterion. File/content checks and named JSON field assertions prove only those observations, never test execution or overall functionality. execution_success checks an exact current-run successful workspace_run receipt, proving only that script exited zero. tool_read_equals independently re-reads an already-authorized read tool during verification.', inputSchema: { type: 'object', required: ['expectedRevision', 'criteria'], properties: { expectedRevision: { type: 'integer' }, criteria: { type: 'array', minItems: 1, maxItems: 16, items: { anyOf: [fileCriterion('file_exists'), fileCriterion('file_contains', { contains: { type: 'string', minLength: 1, maxLength: 2048 } }), toolCriterion('tool_json_equals'), toolCriterion('tool_read_equals'),{type:'object',required:['id','kind','evidenceId'],properties:{id:criterionId,kind:{enum:['execution_success']},evidenceId:{type:'string'}},additionalProperties:false}] } } }, additionalProperties: false } },
  { name: 'host_task_verify', description: 'Host verifies the declared finite assertions using authorized workspace reads, an independent read-tool call, or an exact current-run successful read response. Reports and task metadata are inadmissible. The receipt does not execute tests or certify functionality. For task completion make a separate host_task_update with only expectedRevision and complete:true.', inputSchema: { type: 'object', required: ['expectedRevision'], properties: { expectedRevision: { type: 'integer' } }, additionalProperties: false } },
];
const digest = value => createHash('sha256').update(typeof value === 'string' ? value : JSON.stringify(value)).digest('hex');
const stateDigest = task => digest({ goal: task.goal, steps: task.steps, artifacts: task.artifacts, criteria: task.criteria });
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const fail = (code, status = 409) => { throw hostError(code, status); };

export function setTaskCriteria(task, input, runId) {
  if (!object(input) || Object.keys(input).some(key => !['expectedRevision', 'criteria'].includes(key)) || input.expectedRevision !== task.revision) fail('host_task_revision_conflict');
  if (!Array.isArray(input.criteria) || input.criteria.length < 1 || input.criteria.length > 16) fail('host_task_criteria_invalid', 400);
  const criteria = input.criteria.map(item => {
    if (!object(item) || !/^[a-zA-Z0-9_.:-]{1,80}$/.test(item.id) || !['file_exists', 'file_contains', 'tool_json_equals', 'tool_read_equals','execution_success'].includes(item.kind)) fail('host_task_criteria_invalid', 400);
    if(item.kind==='execution_success'){if(Object.keys(item).some(key=>!['id','kind','evidenceId'].includes(key))||typeof item.evidenceId!=='string'||!item.evidenceId)fail('host_task_criteria_invalid',400);return structuredClone(item);}
    const fields = item.kind.startsWith('file_') ? ['id', 'kind', 'artifactId', ...(item.kind === 'file_contains' ? ['contains'] : [])] : ['id', 'kind', 'toolName', 'path', 'expected', ...(item.kind === 'tool_read_equals' ? ['arguments'] : ['evidenceId'])];
    if (Object.keys(item).some(key => !fields.includes(key))) fail('host_task_criteria_invalid', 400);
    if (item.kind.startsWith('file_')) {
      if (typeof item.artifactId !== 'string' || !task.artifacts.some(artifact => artifact.id === item.artifactId) || item.kind === 'file_contains' && (typeof item.contains !== 'string' || !item.contains || item.contains.length > 2048)) fail('host_task_criteria_invalid', 400);
    } else {
      if (typeof item.toolName !== 'string' || !item.toolName || item.toolName.length > 160 || item.toolName.startsWith('host_task_') || !Array.isArray(item.path) || item.path.length > 8 || item.path.some(key => typeof key !== 'string' || !key || key.length > 80 || ['__proto__', 'prototype', 'constructor'].includes(key)) || !Object.hasOwn(item, 'expected') || !['string', 'number', 'boolean'].includes(typeof item.expected) && item.expected !== null || typeof item.expected === 'string' && item.expected.length > 2048 || typeof item.expected === 'number' && !Number.isFinite(item.expected)) fail('host_task_criteria_invalid', 400);
      if (item.kind === 'tool_read_equals' && (item.arguments !== undefined && !object(item.arguments) || Buffer.byteLength(JSON.stringify(item.arguments ?? {})) > 8192)) fail('host_task_criteria_invalid', 400);
      if (item.evidenceId !== undefined && typeof item.evidenceId !== 'string') fail('host_task_criteria_invalid', 400);
    }
    return structuredClone(item);
  });
  if (new Set(criteria.map(item => item.id)).size !== criteria.length) fail('host_task_criteria_invalid', 400);
  return { ...task, status: task.status === 'completed' ? 'running' : task.status, criteria, criteriaSource: { provenance: 'parent-declared', runId, goal: task.goal }, acceptance: null, updatedAt: new Date().toISOString() };
}

export function acceptanceValid(task) {
  const receipt = task?.acceptance;
  return Boolean(receipt?.ok && receipt.provenance === 'host-finite-checks' && receipt.runId === task.runs.at(-1)?.id && (receipt.validAtRevision ?? receipt.taskRevision) === task.revision && receipt.stateDigest === stateDigest(task) && !task.outcomeUnknown && !['interrupted', 'cancelled'].includes(task.status) && receipt.evidenceRefs.every(ref => task.evidence.some(item => item.id === ref && item.status === 'completed' && !item.outcomeUnknown && item.provenance === 'host-tool-result')));
}
export function assertAcceptance(task, runId) {
  if (!acceptanceValid(task) || task.acceptance.runId !== runId) fail('host_task_acceptance_required');
}
function jsonField(record, path) {
  let value = record.result.structuredContent;
  if (value === undefined) { try { value = JSON.parse(record.result.content[0].text); } catch { fail('host_task_criterion_failed'); } }
  for (const key of path) { if ((!object(value) && !Array.isArray(value)) || !Object.hasOwn(value, key)) return undefined; value = value[key]; }
  return value;
}

export async function verifyTask(host, run, input) {
  let task = host.store.task(run.conversationId, run.taskId);
  if (!object(input) || Object.keys(input).some(key => key !== 'expectedRevision') || input.expectedRevision !== task.revision) fail('host_task_revision_conflict');
  if (run.verifying || !task.criteria?.length || task.outcomeUnknown || task.steps.some(step => step.status !== 'completed' || !step.verified || step.outcomeUnknown)) fail('host_task_acceptance_required');
  if ([...host.inflightEffects.values()].some(effect => effect.conversationId === run.conversationId)) fail('host_task_effect_pending');
  const originalState = stateDigest(task), checks = [], evidenceRefs = new Set(); let expectedRevision = task.revision;
  // Historical unchanged steps retain their original receipts, not authority to
  // prove a new attempt. Every declared acceptance assertion below is current.
  for (const step of task.steps) for (const ref of step.evidenceRefs) {
    const evidence = task.evidence.find(item => item.id === ref);
    if (!evidence || evidence.status !== 'completed' || evidence.provenance !== 'host-tool-result' || evidence.runId !== step.attemptRunId) fail('host_task_evidence_stale');
    evidenceRefs.add(ref);
  }
  run.verifying = true;
  const read = async (name, args) => {
    const result = await host.mcp(run.scopeToken, run.conversationId, 'tools/call', { name, arguments: args }, { runId: run.id });
    expectedRevision += 2; // one started and one finished durable evidence record
    const evidenceId = result._meta?.hostEvidenceId;
    const evidence = host.store.task(run.conversationId, run.taskId).evidence.find(item => item.id === evidenceId && item.runId === run.id && item.status === 'completed' && item.readOnly && item.toolName === name);
    if (!evidence || result.isError || result._meta?.hostEvidence === 'advisory') fail('host_task_acceptance_required');
    evidenceRefs.add(evidenceId); return { name, result, evidenceId };
  };
  try {
    const reads = new Map();
    for (const criterion of task.criteria) {
      host.validateMcpScope(run.scopeToken, run.conversationId, { runId: run.id });
      if(criterion.kind==='execution_success'){const record=run.currentToolResults?.get(criterion.evidenceId),evidence=task.evidence.find(e=>e.id===criterion.evidenceId&&e.runId===run.id&&e.toolName==='workspace_run'&&e.status==='completed'&&!e.outcomeUnknown&&e.provenance==='host-tool-result');if(!record||!evidence||record.name!=='workspace_run'||record.result.isError||jsonField(record,['success'])!==true||jsonField(record,['exitCode'])!==0)fail('host_task_acceptance_required');evidenceRefs.add(evidence.id);checks.push({criterionId:criterion.id,ok:true,scope:'current-run-isolated-script-exited-zero-only',evidenceId:evidence.id});
      }else if (criterion.kind.startsWith('file_')) {
        let record = reads.get(criterion.artifactId);
        if (!record) {
          const artifact = task.artifacts.find(item => item.id === criterion.artifactId);
          if (!artifact) fail('host_task_artifact_unverifiable');
          const root = join(host.dataDir, 'workspaces', String(task.projectId), 'agent');
          const path = isAbsolute(artifact.location) ? relative(root, artifact.location) : artifact.location;
          if (path === '..' || path.startsWith(`..${sep}`) || isAbsolute(path) || /^[a-z]+:/i.test(path)) fail('host_task_artifact_unverifiable');
          const result = await read('workspace_read', { path }); let value;
          try { value = JSON.parse(result.result.content[0].text); } catch { fail('host_task_artifact_unverifiable'); }
          if (value.truncated || typeof value.content !== 'string') fail('host_task_artifact_unverifiable');
          record = { content: value.content, evidenceId: result.evidenceId, contentSha256: digest(value.content), decodedUtf8Bytes: Buffer.byteLength(value.content) }; reads.set(criterion.artifactId, record);
        }
        checks.push({ criterionId: criterion.id, ok: criterion.kind === 'file_exists' || record.content.includes(criterion.contains), scope: criterion.kind === 'file_exists' ? 'workspace-file-readable-at-check-time-only' : 'workspace-file-literal-content-at-check-time-only', artifactId: criterion.artifactId, contentEncoding: 'decoded-utf8', contentSha256: record.contentSha256, decodedUtf8Bytes: record.decodedUtf8Bytes, evidenceId: record.evidenceId });
      } else {
        let record, evidenceId;
        if (criterion.kind === 'tool_read_equals') {
          if (!authorizedReadCapabilities(host, run).some(tool => tool.name === criterion.toolName)) fail('host_child_tool_denied', 403);
          record = await read(criterion.toolName, criterion.arguments ?? {}); evidenceId = record.evidenceId;
        } else {
          const item = [...(run.currentToolResults?.entries() ?? [])].reverse().find(([id, record]) => record.name === criterion.toolName && (!criterion.evidenceId || id === criterion.evidenceId) && !record.result.isError && record.result?._meta?.hostEvidence !== 'advisory');
          if (!item) fail('host_task_acceptance_required');
          [evidenceId, record] = item;
          if (!task.evidence.some(item => item.id === evidenceId && item.runId === run.id && item.status === 'completed' && item.readOnly && item.provenance === 'host-tool-result')) fail('host_task_acceptance_required');
          evidenceRefs.add(evidenceId);
        }
        checks.push({ criterionId: criterion.id, ok: Object.is(jsonField(record, criterion.path), criterion.expected), scope: criterion.kind === 'tool_read_equals' ? 'independent-read-tool-json-field-equality-at-check-time-only' : 'current-run-tool-json-field-equality-only', toolName: record.name, evidenceId });
      }
    }
    host.validateMcpScope(run.scopeToken, run.conversationId, { runId: run.id });
    task = host.store.task(run.conversationId, run.taskId);
    if (stateDigest(task) !== originalState || task.revision !== expectedRevision || task.outcomeUnknown) fail('host_task_revision_conflict');
    if ([...host.inflightEffects.values()].some(effect => effect.conversationId === run.conversationId)) fail('host_task_effect_pending');
    const acceptance = { provenance: 'host-finite-checks', ok: checks.every(check => check.ok), runId: run.id, taskRevision: task.revision + 1, stateDigest: originalState, checks, evidenceRefs: [...evidenceRefs], checkedAt: new Date().toISOString(), limitations: ['Only the declared finite assertions and named authorized read results were checked.', 'No test command was executed; file contents and child reports do not prove tests passed or functionality is complete.'] };
    host.store.saveTask({ ...task, acceptance, updatedAt: new Date().toISOString() });
    if (!acceptance.ok) fail('host_task_criterion_failed');
    return acceptance;
  } finally { run.verifying = false; }
}
