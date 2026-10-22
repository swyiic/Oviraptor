import { randomUUID } from 'node:crypto';
import { CertificateTasks } from './certificate-tasks.mjs';
import { hostError } from './store.mjs';

export const DISCOVERY_ASSIST_TOOLS = new Map([
  ['discovery_assist_start', 'start'], ['discovery_assist_status', 'status'],
  ['discovery_assist_pause', 'pause'], ['discovery_review_read', 'review'],
  ['discovery_scope_propose', 'proposal'],
]);
const reads = new Set(['discovery_assist_status', 'discovery_review_read', 'discovery_scope_propose']);
export class DiscoveryAssist extends CertificateTasks {
  constructor(options = {}) {
    super({ ...options, tools: DISCOVERY_ASSIST_TOOLS, route: '/discovery-assist', kind: 'discovery',
      maxInputBytes: 256 * 1024, maxResponseBytes: 1024 * 1024, requestTimeoutMs: 120000, readOnlyTools: reads });
  }
  async readFull(store, projectId) {
    return (await super.call(store, projectId, 'discovery_assist_status', {}, randomUUID())).structuredContent;
  }
  async call(store, projectId, name, args, operationId) {
    const result = await super.call(store, projectId, name, args, operationId), body = result.structuredContent;
    const proposal = body.proposal;
    const verifiedGroups = (proposal?.groups ?? []).filter(g => g.verification?.status === 'bound_local_evidence'
      && g.verification.sources?.some(source => typeof source.host === 'string' && source.host && typeof source.kind === 'string' && source.kind));
    const scopeCount = verifiedGroups.reduce((n, g) => n + (g.scopes?.length ?? 0), 0);
    const verificationConflicts = Number.isSafeInteger(proposal?.verificationConflicts) && proposal.verificationConflicts >= 0 ? proposal.verificationConflicts : null;
    const humanReview = { available: Boolean(proposal), source: 'current_domain_verification', scopeFamilyCount: verifiedGroups.length, scopeCount,
      verificationConflicts, totalGroups: verificationConflicts === null ? null : verifiedGroups.length + verificationConflicts,
      required: verifiedGroups.length > 0 ? true : verificationConflicts === null ? null : verificationConflicts > 0,
      rawModelClassificationsAreNotHumanReview: true, automaticEvidenceRequiresHumanReview: false };
    // Complete families stay in the authenticated human panel. A model receives
    // bounded summaries and cannot manufacture a confirmation flag/tool.
    const view = { ...body, humanReview, ...(proposal ? { proposal: { token: proposal.token, totalEvidenceGroups: proposal.totalEvidenceGroups,
      familyCount: verifiedGroups.length, scopeCount, verificationConflicts,
      verificationPolicyVersion: proposal.verificationPolicyVersion, networkVerified: false,
      heldForVerification: (proposal.heldForVerification ?? 0) + (proposal.groups?.length ?? 0) - verifiedGroups.length,
      removedUnverified: proposal.removedUnverified ?? 0, modelAdviceIsNotOwnershipVerification: true,
      preview: verifiedGroups.slice(0, 12).map(g => ({ root: g.root, shared: g.shared, observedCount: g.observedCount,
        verification: { status: g.verification.status, sources: g.verification.sources.slice(0, 3) }, aiReasons: g.reasons?.slice(0, 2) })),
      userConfirmationRequired: verifiedGroups.length > 0, fullProposalInConversationPanel: true } } : {}) };
    if (view.review) view.review = { ...view.review, items: (view.review.items ?? []).slice(0, 4) };
    if (view.items) view.items = view.items.map(item => ({ ...item, evidence: { domains: item.evidence?.domains?.slice(0, 12),
      domainEvidence: item.evidence?.domainEvidence?.slice(0, 12),
      subjectOrganizations: item.evidence?.subjectOrganizations, icpOrganizations: item.evidence?.icpOrganizations,
      evidenceTruncated: item.evidence?.evidenceTruncated, exampleAssetIds: item.evidence?.exampleAssetIds } }));
    const text = JSON.stringify(view);
    if (Buffer.byteLength(text) > 48 * 1024) throw hostError('host_discovery_result_too_large', 409, { retryable: false });
    return { ...result, structuredContent: view, content: [{ type: 'text', text }] };
  }
  async confirm(store, projectId, args, operationId) {
    // Private to the session-authenticated human handler, absent from tools/list.
    const bridge = new CertificateTasks({ origin: this.origin, token: this.token,
      tools: new Map([['human_scope_confirm', 'confirm']]), route: this.route, kind: this.kind,
      maxInputBytes: 256 * 1024, maxResponseBytes: 1024 * 1024, requestTimeoutMs: 120000 });
    return (await bridge.call(store, projectId, 'human_scope_confirm', args, operationId)).structuredContent;
  }
  async trustedStart(store, projectId, args, operationId) {
    // Session-authenticated human options deliberately do not enter the model
    // tool schema. In particular a model cannot manufacture paid consent.
    const bridge = new CertificateTasks({ origin: this.origin, token: this.token,
      tools: new Map([['human_discovery_start', 'start']]), route: this.route, kind: this.kind,
      maxInputBytes: 256 * 1024, maxResponseBytes: 1024 * 1024, requestTimeoutMs: 120000 });
    return (await bridge.call(store, projectId, 'human_discovery_start', args, operationId)).structuredContent;
  }
}

/** UI adapter, always bound to a stored conversation and current access policy. */
export async function discoveryConversationAction(host, conversationId, projectId, input = {}) {
  const conversation = host.store.conversation(conversationId, projectId), p = conversation.projectId;
  const app = host.store.application('org.oviraptor.assets'), identity = host.runtimeIdentity();
  const grant = host.apps.effectiveGrant(app, p, identity), policy = host.accessPolicy.view(p);
  const witness = { appRevision: app.revision, grantRevision: grant.revision, policyRevision: policy.revision, runtimeIdentity: identity };
  if (!app.enabled || host.apps.changing.has(app.id) || !host.apps.discoveryAssist.available) throw hostError('host_discovery_executor_unavailable', 503);
  if (input.action === 'start' || input.action === 'pause') {
    if (Object.keys(witness).some(key => input[key] !== witness[key])) throw hostError('host_discovery_confirmation_stale', 409);
    if (grant.disclosureRuntime !== identity) throw hostError('host_tool_permission_denied', 403, { retryable: false });
    const args = input.action === 'start' ? { organization: input.organization, includeIsolated: input.includeIsolated,
      ...(input.collectFirst !== undefined ? { collectFirst: input.collectFirst } : {}),
      ...(input.collectionTargets !== undefined ? { collectionTargets: input.collectionTargets } : {}),
      ...(input.allowHunter !== undefined ? { allowHunter: input.allowHunter } : {}) } : {};
    if(input.action==='start'&&(input.collectFirst!==undefined||input.collectionTargets!==undefined||input.allowHunter!==undefined)){
      if(typeof input.organization!=='string'||input.organization.trim().length<2||input.organization.length>240
        ||input.includeIsolated!==undefined&&typeof input.includeIsolated!=='boolean'
        ||input.collectFirst!==undefined&&typeof input.collectFirst!=='boolean'
        ||input.allowHunter!==undefined&&typeof input.allowHunter!=='boolean'
        ||input.collectionTargets!==undefined&&(!Array.isArray(input.collectionTargets)||input.collectionTargets.length>100||input.collectionTargets.some(v=>typeof v!=='string'||!v.trim()||v.length>240||/[\u0000-\u001f\u007f]/u.test(v))))throw hostError('host_tool_input_invalid',400);
      if(!grant.permissions.includes('inventory.write')||!grant.permissions.includes('certificate.collect'))throw hostError('host_tool_permission_denied',403,{retryable:false});
      const before=await host.apps.discoveryAssist.readFull(host.store,p);
      const checkedApp=host.store.application(app.id),checkedPolicy=host.accessPolicy.view(p),checkedIdentity=host.runtimeIdentity(),checkedGrant=host.apps.effectiveGrant(checkedApp,p,checkedIdentity);
      if(!checkedApp.enabled||host.apps.changing.has(app.id)||checkedApp.revision!==witness.appRevision||checkedGrant.revision!==witness.grantRevision||checkedPolicy.revision!==witness.policyRevision||checkedIdentity!==identity||checkedGrant.disclosureRuntime!==identity)throw hostError('host_tool_authority_changed',403);
      if(!checkedGrant.permissions.includes('inventory.write')||!checkedGrant.permissions.includes('certificate.collect'))throw hostError('host_tool_permission_denied',403,{retryable:false});
      const same=before.organization===input.organization&&before.pipeline;
      if((input.collectFirst===true||input.allowHunter===true||same&&before.pipeline.allowHunter&&input.allowHunter!==false||same&&(before.pipeline.collectionPending??Boolean(before.pipeline.collectionRunId)))&&!checkedGrant.permissions.includes('inventory.discovery.run'))throw hostError('host_tool_permission_denied',403,{retryable:false});
      const result=await host.apps.discoveryAssist.trustedStart(host.store,p,args,randomUUID());
      const now=host.accessPolicy.view(p),current=host.store.application(app.id),g=host.apps.effectiveGrant(current,p,host.runtimeIdentity());
      if(current.revision!==witness.appRevision||g.revision!==witness.grantRevision||now.revision!==witness.policyRevision||host.runtimeIdentity()!==identity)throw hostError('host_tool_authority_changed',403);
      return result;
    }
    const result = await host.apps.call(app.id, 'discovery_assist_' + input.action, p, args, grant.revision, randomUUID(), undefined, identity);
    return result.structuredContent;
  }
  if (input.action === 'confirm') {
    if (input.userConfirmed !== true || Object.keys(witness).some(key => input[key] !== witness[key])) throw hostError('host_discovery_confirmation_stale', 409, { retryable: false });
    if (!grant.permissions.includes('certificate.collect') || grant.disclosureRuntime !== identity) throw hostError('host_tool_permission_denied', 403, { retryable: false });
    const { runId, proposalToken, expectedRoots, hosts } = input;
    if (typeof runId !== 'string' || typeof proposalToken !== 'string' || !Array.isArray(expectedRoots) || !Array.isArray(hosts)
      || !hosts.length || hosts.length > 500 || hosts.some(h => typeof h !== 'string' || h.length > 253)) throw hostError('host_tool_input_invalid', 400);
    const result = await host.apps.confirmDiscoveryScope(p, { runId, proposalToken, expectedRoots, hosts, userConfirmed: true }, witness);
    host.emit({ type: 'discovery.assist.updated', conversationId, projectId: p });
    return result;
  }
  if (input.action !== undefined && input.action !== 'status') throw hostError('host_tool_input_invalid', 400);
  if (!grant.permissions.includes('inventory.read') || grant.disclosureRuntime !== identity) throw hostError('host_tool_permission_denied', 403, { retryable: false });
  const view = await host.apps.discoveryAssist.readFull(host.store, p);
  const now = host.accessPolicy.view(p), current = host.store.application(app.id), g = host.apps.effectiveGrant(current, p, host.runtimeIdentity());
  if (current.revision !== witness.appRevision || g.revision !== witness.grantRevision || now.revision !== witness.policyRevision || host.runtimeIdentity() !== identity) throw hostError('host_tool_authority_changed', 403);
  let enrichment = null;
  if (host.apps.discoveryEnrichment.available) {
    enrichment = (await host.apps.call(app.id, 'discovery_enrichment_status', p, {}, grant.revision, randomUUID(), undefined, identity)).structuredContent;
  }
  return { ...view, enrichment, confirmationWitness: witness };
}
