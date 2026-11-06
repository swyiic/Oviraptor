import type { Ref } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";
import type { GapFollowupDraftInput, GapFollowupPreview, GapFollowupSubmission, SentinelScan, WorkbenchScanInput } from "../types";

export type WorkbenchMode = "web" | "code" | "greybox" | "cicd" | "skills";
type CreationForm = Omit<WorkbenchScanInput, "scanType" | "urls" | "authSessionScopeId"> & {
  urls: string;
  closure: "breadth" | "proof";
  orchestrationMode: "single" | "multi";
  homogeneousWorkers?: boolean;
  homogeneousExecution?: boolean;
  approvedToolIds?: string[];
  // web only: the task-creation gate for autonomous OSS source-audit fetch.
  allowSourceFetch?: boolean;
  // web only: operator opt-in for high-risk capability classes, default empty
  // (all off). Any non-empty selection requires `legalAttestation`. The authored
  // set is bound to the draft, immutable per scan+attempt; per-action approval
  // still sits on top at run time.
  highRiskClasses?: string[];
  legalAttestation?: boolean;
};

export function canonicalApprovedToolIds(ids: readonly string[]): string[] {
  const { tr } = useI18n();
  if (ids.length > 16) throw new Error(tr("工具批准 ID 最多 16 个", "At most 16 approval IDs"));
  const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
  if (ids.some(id => !uuid.test(id))) throw new Error(tr("请输入小写完整 UUID 批准 ID", "Use canonical lowercase UUID approval IDs"));
  if (new Set(ids).size !== ids.length) throw new Error(tr("批准 ID 重复，请主动移除重复项", "Remove duplicate approval IDs"));
  return [...ids].sort();
}

export function useWorkbenchTaskCreation(context: {
  mode: Ref<WorkbenchMode>;
  form: CreationForm;
  busy: Ref<boolean>;
  followup: () => GapFollowupPreview | undefined;
  hasHandoff: () => boolean;
  saveClosureHandoff: () => Promise<void>;
  restoreFollowupSubmission: () => Promise<boolean>;
  pendingFollowupInput: Ref<GapFollowupDraftInput | undefined>;
  followupSubmission: Ref<GapFollowupSubmission | undefined>;
  authSessionScopeId: Ref<string>;
  resetIdentitySelection: () => void;
  generation: () => number;
  isDisposed: () => boolean;
  tr: (zh: string, en: string) => string;
  notify: (type: "success" | "error" | "info", text: string) => void;
  reload: () => void;
  prepareScan: (scan: SentinelScan) => void;
  openScan: (scan: SentinelScan) => void;
}) {
  const { form, busy, tr, notify, reload, prepareScan, openScan,
    pendingFollowupInput, followupSubmission, authSessionScopeId, resetIdentitySelection } = context;

  async function start(saveDraftOnly = false) {
    if (busy.value || context.isDisposed()) return;
    let approvedToolIds: string[];
    try {
      approvedToolIds = canonicalApprovedToolIds(form.approvedToolIds || []);
      if (form.homogeneousExecution && (!form.homogeneousWorkers || !approvedToolIds.length || form.authSessionIds.length < 2))
        throw new Error(tr("受控执行需要同质多智能体、至少一个批准工具和两个测试登录身份。", "Controlled execution requires homogeneous Multi, an approved tool and two test identities."));
      if (form.homogeneousExecution) saveDraftOnly = true;
      if (form.homogeneousWorkers && (context.mode.value !== "web" || (approvedToolIds.length && !form.homogeneousExecution) || context.followup() || context.hasHandoff() || form.orchestrationMode !== "multi"))
        throw new Error(tr("同质 Worker 需要新的多智能体任务，使用内置只读能力。请先清空工具选择。", "Homogeneous workers require a new Multi task with built-in read-only capabilities. Clear tool selection first."));
      if (approvedToolIds.length && (context.mode.value !== "web" || form.orchestrationMode !== "multi" || context.followup() || context.hasHandoff()))
        throw new Error(tr("工具选择仅支持新的多智能体 Web 任务，请主动清空选择后再切换任务类型。", "Tool selection requires a new Multi Web task. Clear the selection explicitly before changing task type."));
    } catch (error) { notify("error", String(error)); return; }
    if (context.hasHandoff()) { await context.saveClosureHandoff(); return; }
    if (context.followup()) {
      const generation = context.generation();
      busy.value = true;
      const recovered = await context.restoreFollowupSubmission();
      busy.value = false;
      if (!recovered || context.isDisposed() || generation !== context.generation()) return;
      const saved = followupSubmission.value;
      if (saved?.scan) {
        // Opening a recovered task is not creation or confirmation of execution.
        reload();
        if (saved.scan.status === "draft") prepareScan(saved.scan);
        else openScan(saved.scan);
        return;
      }
      if (saved?.createdScanId) {
        notify("info", tr(`原提交已创建过任务 ${saved.createdScanId}，该任务现已删除；不会重建。请先结束原提交。`, `The original task ${saved.createdScanId} was deleted and will not be recreated. Finish the original submission first.`));
        return;
      }
    }
    const followup = context.followup();
    const pending = pendingFollowupInput.value;
    if (pending && (!followup || pending.sourceScanId !== followup.sourceScanId
      || pending.assessmentMessageId !== followup.assessmentMessageId
      || pending.sourceHash !== followup.sourceHash)) {
      notify("error", tr("另一补充任务的提交尚未确认，请先核对任务中心或回到原申请重试。", "Another follow-up submission is unconfirmed. Check the task center or return to the original request before retrying."));
      return;
    }
    if (followup && (context.mode.value !== "web" || form.projectId !== followup.projectId
      || form.urls !== followup.targetUrl)) {
      notify("error", tr("补充任务必须保持原项目和原目标；请重新从聊天准备。", "Follow-up must keep the original project and target; prepare again from chat."));
      return;
    }
    if (followup) saveDraftOnly = true;
    if (!form.projectId) {
      notify("info", tr("请先选择归属工作空间", "Select a workspace"));
      return;
    }
    // Source model calls cannot currently reconcile a monetary limit. Reject
    // before persistence; the backend retains its independent fail-closed gate.
    if (context.mode.value !== "web" && form.maxBudgetUsd !== undefined) {
      notify("error", tr(
        "源码多智能体尚不能执行 USD 费用上限。若必须限制账单，请暂勿启动；若接受仅由 token 和时间限额约束，请主动清空 USD 上限再创建任务。",
        "Source agents cannot enforce a USD cap yet. Do not start if a billing ceiling is required; otherwise clear the USD cap explicitly to use token and time limits."
      ));
      return;
    }
    busy.value = true;
    const generation = context.generation();
    const currentView = () => !context.isDisposed() && generation === context.generation();
    try {
      const urls = form.urls
        .split(/\r?\n|,|，|;|；|\s+/)
        .map((value) => value.trim().replace(/\/+$/, ""))
        .filter(Boolean);
      if (context.mode.value === "web") {
        if (!urls.length) {
          throw new Error(tr("至少添加一个 http:// 或 https:// URL", "Add at least one http:// or https:// URL"));
        }
        // Mandatory switch: any high-risk capability requires the legal-authorization
        // attestation before the task is even created. The backend enforces the same
        // rule as a fail-closed floor; this surfaces it to the operator up front.
        if ((form.highRiskClasses?.length || 0) > 0 && !form.legalAttestation) {
          throw new Error(tr(
            "已选择高危能力：必须先勾选「已获得对这些目标的合法书面授权」才能创建任务。",
            "High-risk capabilities are selected: confirm written legal authorization for these targets before creating the task.",
          ));
        }
        const submitted = { projectId: form.projectId, taskName: form.taskName, urls: form.urls, scopeId: authSessionScopeId.value, approvedToolIds: [...(form.approvedToolIds || [])] };
        if (followup && !pendingFollowupInput.value) {
          pendingFollowupInput.value = {
            requestId: globalThis.crypto.randomUUID(), sourceScanId: followup.sourceScanId,
            assessmentMessageId: followup.assessmentMessageId, sourceHash: followup.sourceHash,
            taskName: form.taskName, scanMode: form.scanMode, maxBudgetUsd: form.maxBudgetUsd,
            authSessionIds: [...form.authSessionIds], authSessionScopeId: authSessionScopeId.value,
            skillIds: [...form.skillIds], instruction: form.instruction, closure: form.closure,
          };
        }
        const createOrdinaryDraft = form.homogeneousWorkers
          ? (...args: Parameters<typeof sentinelApi.createSentinelUrlScan>) => sentinelApi.createSentinelHomogeneousUrlScan(...args, Boolean(form.homogeneousExecution), approvedToolIds)
          : approvedToolIds.length
          ? (...args: Parameters<typeof sentinelApi.createSentinelUrlScan>) => sentinelApi.createSentinelUrlScanWithToolSelection(...args, approvedToolIds)
          : sentinelApi.createSentinelUrlScan;
        const draft = followup && pendingFollowupInput.value
          ? await sentinelApi.createAgentGapFollowup({ ...pendingFollowupInput.value,
            authSessionIds: [...pendingFollowupInput.value.authSessionIds], skillIds: [...pendingFollowupInput.value.skillIds] })
          : await createOrdinaryDraft(
            form.projectId, form.taskName, urls, form.scanMode, form.maxBudgetUsd,
            form.authSessionId || undefined, [...form.authSessionIds], authSessionScopeId.value,
            [...form.skillIds], form.instruction, form.closure, form.orchestrationMode,
          );
        if (followup && pendingFollowupInput.value && currentView())
          followupSubmission.value = { input: pendingFollowupInput.value, scan: draft, createdScanId: draft.id };
        pendingFollowupInput.value = undefined;
        // Bind the source-fetch consent to the draft at creation time — the hard
        // gate: without this, the backend refuses every fetch for this scan. A
        // failure here must not block the scan; it only means fetch stays denied.
        if (form.allowSourceFetch && draft.id) {
          try {
            await sentinelApi.authorizeScanSourceFetch(draft.id);
          } catch (error) {
            notify("error", tr(
              `任务已创建，但「允许拉源」登记失败：${String(error)}。源码审计复现暂不可用，可在任务中重试。`,
              `Task created, but the source-fetch consent failed to register: ${String(error)}. Source-audit reproduction is unavailable; retry from the task.`
            ));
          }
        }
        // Bind the high-risk capability opt-in to the draft at creation time. Like
        // source-fetch this is fail-closed: a failure here only leaves every
        // high-risk class denied, it never blocks the scan. The authorization is
        // immutable per scan+attempt, so it is written exactly once, here.
        if ((form.highRiskClasses?.length || 0) > 0 && draft.id) {
          try {
            await sentinelApi.setScanCapabilityAuthorization(
              draft.id, Boolean(form.legalAttestation), [...(form.highRiskClasses || [])],
            );
          } catch (error) {
            notify("error", tr(
              `任务已创建，但高危能力授权登记失败：${String(error)}。这些高危能力保持关闭；该授权一经设定不可更改，请核对后重新新建任务。`,
              `Task created, but the high-risk capability authorization failed to register: ${String(error)}. Those capabilities stay off; the authorization is immutable once set, so create a new task after checking.`
            ));
          }
        }
        // The persisted draft consumes these task-local identities even if start later fails.
        reload();
        if (form.projectId === submitted.projectId && authSessionScopeId.value === submitted.scopeId) {
          resetIdentitySelection();
          if (JSON.stringify(form.approvedToolIds || []) === JSON.stringify(submitted.approvedToolIds)) form.approvedToolIds = [];
          if (form.taskName === submitted.taskName) form.taskName = "";
          if (form.urls === submitted.urls) form.urls = "";
        }
        if (!currentView()) return;
        if (saveDraftOnly) {
          notify("success", tr("Web 草稿已保存，尚未启动。可在任务预览中登记授权控制组，再确认启动。", "Web draft saved without starting. Register authorization controls in task preview, then confirm start."));
          prepareScan(draft);
          return;
        }
        let scan: SentinelScan;
        try {
          scan = await sentinelApi.confirmSentinelScan(draft.id);
        } catch (error) {
          reload();
          if (currentView()) {
            notify("error", tr(`任务 ${draft.id} 已保存，但启动未获确认：${String(error)}。请在任务中心核对状态，不要重复创建。`, `Task ${draft.id} was saved, but start was not confirmed: ${String(error)}. Check task status before retrying.`));
            prepareScan(draft);
          }
          return;
        }
        reload();
        if (!currentView()) return;
        notify("success", tr(
          `Nest Web 扫描已启动，共 ${urls.length} 个 URL。${draft.currentCheckpoint || ""}`,
          `Nest Web scan started with ${urls.length} URL(s). ${draft.currentCheckpoint || ""}`
        ));
        openScan(scan);
        return;
      }
      const submitted = {
        projectId: form.projectId, taskName: form.taskName, instruction: form.instruction,
        authValue: form.authValue, scopeId: authSessionScopeId.value,
      };
      const scan = await sentinelApi.startWorkbenchScan({
        projectId: form.projectId, taskName: form.taskName,
        scanType: context.mode.value as Exclude<WorkbenchMode, "skills" | "web">,
        urls, sourcePath: form.sourcePath, skillIds: form.skillIds,
        instruction: form.instruction, scanMode: form.scanMode,
        scopeMode: form.scopeMode, diffBase: form.diffBase,
        maxBudgetUsd: form.maxBudgetUsd, environment: form.environment,
        authProfileName: form.authProfileName, authType: form.authType,
        authHeaderName: form.authHeaderName, authValue: form.authValue,
        authSessionId: form.authSessionId, authSessionIds: form.authSessionIds,
        authSessionScopeId: authSessionScopeId.value, ciProvider: form.ciProvider,
        repositoryUrl: form.repositoryUrl, branch: form.branch,
        commitSha: form.commitSha, buildId: form.buildId,
        maxCritical: form.maxCritical, maxHigh: form.maxHigh,
        blockRelease: form.blockRelease,
      });
      reload();
      // A stale response must not navigate or erase newer form edits.
      if (!currentView()) return;
      notify("success", tr("Nest 任务已启动，可在任务总览查看进度", "Nest task started; track it in Tasks"));
      if (form.taskName === submitted.taskName) form.taskName = "";
      if (form.instruction === submitted.instruction) form.instruction = "";
      if (form.authValue === submitted.authValue) form.authValue = "";
      if (form.projectId === submitted.projectId && authSessionScopeId.value === submitted.scopeId)
        resetIdentitySelection();
      openScan(scan);
    } catch (error) {
      // Transport failure after a committed follow-up retains its idempotency key.
      if (String(error).startsWith("followup_rejected:")) {
        pendingFollowupInput.value = undefined;
        if (currentView()) followupSubmission.value = undefined;
      }
      if (currentView()) notify("error", String(error));
    } finally {
      busy.value = false;
    }
  }
  return { start };
}
