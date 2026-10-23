// §E5 — review, remediation and retest for one finding. The queue's revision is read before a
// decision is sent, so a conclusion written against a moved queue is refused instead of
// overwriting somebody else's work; a retest is always a new round, and closing needs a basis
// the retest actually produced. Observations are never rewritten here.

import { computed, ref, watch, onUnmounted } from "vue";
import { exposureErrorText, isUnregisteredCommand, remediationBlockedReason, type ExposureEvidence } from "./contracts";
import type { ExposureRemediation, ExposureRetestOutcome, ExposureTaxonomyRow } from "./api";

export interface ExposureReviewApi {
  readExposureEvidence?: (findingId: number, expectedProjectId: number) => Promise<unknown>;
  reviewExposureFindingScoped?: (
    projectId: number,
    findingId: number,
    expectedRevision: number,
    status: string,
    actor: string,
    reason: string,
    references: number[],
    expectedReviewEvent: number,
  ) => Promise<number>;
  listExposureReviewTrail?: (projectId: number, findingId: number) => Promise<unknown>;
  saveExposureRemediation?: (
    projectId: number,
    findingId: number,
    observationId: number | null,
    runId: number,
    actor: string,
    note: string,
    status: string,
  ) => Promise<number>;
  pageExposureRemediations?: (projectId: number, status: string, pageSize: number, cursor: string) => Promise<unknown>;
  pageExposureFindingRemediations?: (projectId: number, findingId: number, pageSize: number, cursor: string) => Promise<unknown>;
  startExposureRetest?: (projectId: number, remediationId: number) => Promise<number>;
  readExposureRetestOutcome?: (projectId: number, remediationId: number) => Promise<unknown>;
  markExposureRemediation?: (
    projectId: number,
    remediationId: number,
    status: string,
    actor: string,
    reason: string,
  ) => Promise<unknown>;
  exposureReviewTaxonomy?: (projectId: number) => Promise<unknown>;
}

export interface ExposureReviewDeps {
  projectId: () => number | undefined;
  findingId: () => number | undefined;
  api: ExposureReviewApi;
  notify: (kind: "success" | "error" | "info", text: string) => void;
}

interface ReviewTrailRow {
  fromStatus: string;
  toStatus: string;
  note: string;
  actor: string;
  expectedRevision: number;
  references: number[];
  createdAt: string;
  eventId?: number;
}

const OUTCOME_LABELS: Record<string, string> = {
  no_retest_round: "还没有复测轮次",
  retest_content_changed: "复测取回不同字节",
  retest_content_identical: "复测字节与原结论一致",
  retest_not_seen: "复测轮没再看到这条 URL",
  retest_unreachable: "复测没能取到该资源",
  retest_round_incomplete: "复测轮本身没收口",
  retest_without_baseline: "原观察已不可读",
};

export function useExposureReview(deps: ExposureReviewDeps) {
  const evidence = ref<ExposureEvidence | null>(null);
  const trail = ref<ReviewTrailRow[]>([]);
  const remediations = ref<ExposureRemediation[]>([]);
  const taxonomy = ref<ExposureTaxonomyRow[]>([]);
  const outcome = ref<ExposureRetestOutcome | null>(null);
  const actor = ref("");
  const reason = ref("");
  const remediationNote = ref("");
  const revision = ref(0);
  const busy = ref(false);
  const loaded = ref(false);
  const error = ref("");
  const conflict = ref(false);
  const available = ref(true);

  let generation = 0;
  let identity = 0;
  let disposed = false;
  let bound = "";
  const key = () => JSON.stringify([deps.projectId(), deps.findingId()]);
  const actionable = () => !disposed && loaded.value && available.value && bound === key();
  const currentAction = (ticket: number) => !disposed && ticket === identity;
  function reset(): void {
    generation += 1; identity += 1; bound = "";
    evidence.value = null; trail.value = []; remediations.value = []; taxonomy.value = [];
    outcome.value = null; revision.value = 0; busy.value = false; loaded.value = false;
    error.value = ""; conflict.value = false; available.value = true;
    reason.value = ""; remediationNote.value = "";
  }
  watch(() => [deps.projectId(), deps.findingId()], (next, previous) => {
    reset();
    if (next[0] !== previous[0]) actor.value = "";
  }, { flush: "sync" });
  onUnmounted(() => { disposed = true; reset(); });

  /// FIX3-R3 迟到的响应必须同时属于当前这一代读取和当前项目：切换项目后旧项目的结果一律丢弃。
  function discarded(claimed: number, projectId: number): boolean {
    return disposed || claimed !== generation || deps.projectId() !== projectId;
  }

  const topRevision = computed(() =>
    (evidence.value?.versions ?? []).reduce((top, row) => (row.revision > top ? row.revision : top), 0),
  );
  /// FIX3-R2 读到的最新复核事件；同版本另一名复核员先提交后，后端据此拒绝本次写入。
  const latestReviewEvent = computed(() => trail.value.reduce((top, row) => Math.max(top, Number(row.eventId ?? 0)), 0));
  const latestObservationId = computed(() => {
    const rows = evidence.value?.observations ?? [];
    return rows.length ? rows[rows.length - 1].id : null;
  });
  const openRemediation = computed(
    () => remediations.value.find((row) => row.status !== "closed" && row.status !== "rejected") ?? null,
  );
  const canClose = computed(() => outcome.value?.canClose === true);
  /// G1-3 an auto-excluded finding is not offered a remediation (the backend refuses it too).
  const remediationBlocked = computed(() =>
    remediationBlockedReason(evidence.value?.disposition ?? null, evidence.value?.finding?.status),
  );
  const outcomeLabel = computed(() =>
    outcome.value ? OUTCOME_LABELS[outcome.value.basis] ?? outcome.value.basis : "尚未复测",
  );
  const notice = computed(() => {
    if (!deps.projectId() || !deps.findingId()) return "请先在线索列表里选中一条线索";
    if (!available.value) return "复核/整改接口尚未注册，页面显示的不是“没有记录”";
    if (error.value && !loaded.value) return `读取失败：${error.value}`;
    if (!loaded.value) return "尚未读取该线索的证据与历史";
    return `队列版本 r${revision.value}；复核会带上这个版本，队列前进就拒绝写入`;
  });

  /// P1-1 整改列表按项目分页；这里翻页收集属于当前线索的记录（最多 25 页），绝不回退到别的线索。
  async function ownRemediations(projectId: number, findingId: number): Promise<ExposureRemediation[]> {
    if (!deps.api.pageExposureRemediations && !deps.api.pageExposureFindingRemediations) return [];
    const own: ExposureRemediation[] = [];
    let cursor = "";
    for (let page = 0; page < 25; page += 1) {
      const read = (await (deps.api.pageExposureFindingRemediations
        ? deps.api.pageExposureFindingRemediations(projectId, findingId, 50, cursor)
        : deps.api.pageExposureRemediations!(projectId, "", 50, cursor))) as {
        items?: ExposureRemediation[];
        hasMore?: boolean;
        nextCursor?: string;
      } | null;
      own.push(...(read?.items ?? []).filter((row) => row.findingId === findingId));
      if (!read?.hasMore) return own;
      if (!read.nextCursor || read.nextCursor === cursor) throw new Error("整改历史分页未前进，不能判定历史已读取完整");
      cursor = read.nextCursor;
    }
    throw new Error("整改历史超过本次读取窗口，不能判定为没有整改记录");
  }

  async function load(): Promise<boolean> {
    const projectId = deps.projectId();
    const findingId = deps.findingId();
    if (!projectId || !findingId) return false;
    if (!deps.api.readExposureEvidence || !deps.api.reviewExposureFindingScoped) {
      available.value = false;
      error.value = "";
      return false;
    }
    const claimed = ++generation;
    bound = "";
    loaded.value = false;
    busy.value = true;
    try {
      const read = await deps.api.readExposureEvidence(findingId, projectId);
      if (discarded(claimed, projectId) || deps.findingId() !== findingId) return false;
      const view = read as ExposureEvidence;
      if (!view || view.finding?.id !== findingId || !Array.isArray(view.versions)) {
        throw new Error("证据返回结构无效");
      }
      evidence.value = view;
      revision.value = topRevision.value;
      const [rows, own, classes] = await Promise.all([
        deps.api.listExposureReviewTrail?.(projectId, findingId) ?? [],
        ownRemediations(projectId, findingId),
        deps.api.exposureReviewTaxonomy?.(projectId) ?? [],
      ]);
      if (discarded(claimed, projectId) || deps.findingId() !== findingId) return false;
      if (!Array.isArray(rows) || !Array.isArray(classes)) throw new Error("复核历史或分类返回结构无效");
      trail.value = rows as ReviewTrailRow[];
      taxonomy.value = classes;
      remediations.value = own;
      // P1-1 只读这条线索自己的整改：别的线索的复测结论不能显示在这里，也不能被这里的按钮操作。
      const target = openRemediation.value ?? own[0];
      if (target && deps.api.readExposureRetestOutcome) {
        const read2 = await deps.api.readExposureRetestOutcome(projectId, target.id);
        if (!discarded(claimed, projectId) && deps.findingId() === findingId) {
          outcome.value = (read2 ?? null) as ExposureRetestOutcome | null;
        }
      } else {
        outcome.value = null;
      }
      if (discarded(claimed, projectId) || deps.findingId() !== findingId) return false;
      bound = key();
      loaded.value = true;
      available.value = true;
      conflict.value = false;
      error.value = "";
      return true;
    } catch (reason) {
      if (discarded(claimed, projectId) || deps.findingId() !== findingId) return false;
      if (isUnregisteredCommand(reason)) available.value = false;
      error.value = exposureErrorText(reason);
      return false;
    } finally {
      if (claimed === generation) busy.value = false;
    }
  }

  async function review(status: string): Promise<boolean> {
    const projectId = deps.projectId();
    const findingId = deps.findingId();
    if (!projectId || !findingId || busy.value || !actionable()) return false;
    const ticket = identity;
    if (!actor.value.trim()) {
      error.value = "复核必须写明操作者";
      return false;
    }
    if (!reason.value.trim()) {
      error.value = "复核必须写明依据，不能只点一个按钮";
      return false;
    }
    busy.value = true;
    try {
      await deps.api.reviewExposureFindingScoped!(
        projectId,
        findingId,
        revision.value,
        status,
        actor.value.trim(),
        reason.value.trim(),
        latestObservationId.value ? [latestObservationId.value] : [],
        latestReviewEvent.value,
      );
      if (!currentAction(ticket)) return false;
      conflict.value = false;
      error.value = "";
      deps.notify("success", `已按 r${revision.value} 记录复核结论：${status}`);
      reason.value = "";
      await load();
      return true;
    } catch (reason) {
      if (!currentAction(ticket)) return false;
      const text = exposureErrorText(reason);
      error.value = text;
      if (/队列已前进|revision|版本/.test(text)) {
        conflict.value = true;
        deps.notify("error", "队列已经变化，本次复核没有写入；请重新读取后再判断");
      } else {
        deps.notify("error", `复核失败：${text}`);
      }
      return false;
    } finally {
      if (currentAction(ticket)) busy.value = false;
    }
  }

  async function recordRemediation(): Promise<boolean> {
    if (remediationBlocked.value) {
      deps.notify("info", remediationBlocked.value);
      return false;
    }
    const projectId = deps.projectId();
    const findingId = deps.findingId();
    if (!projectId || !findingId || !deps.api.saveExposureRemediation || busy.value || !actionable()) return false;
    const ticket = identity;
    if (!actor.value.trim() || !remediationNote.value.trim()) {
      error.value = "整改必须写明操作者与说明";
      return false;
    }
    busy.value = true;
    try {
      const id = await deps.api.saveExposureRemediation(
        projectId,
        findingId,
        latestObservationId.value,
        evidence.value?.observations?.find(row => row.id === latestObservationId.value)?.runId ?? 0,
        actor.value.trim(),
        remediationNote.value.trim(),
        "fixing",
      );
      if (!currentAction(ticket)) return false;
      deps.notify("success", `整改已绑定线索 ${findingId} 与观察 ${latestObservationId.value ?? "无"}`);
      if (id) await load();
      return true;
    } catch (reason) {
      if (!currentAction(ticket)) return false;
      const text = exposureErrorText(reason);
      error.value = text;
      deps.notify("error", `整改登记失败：${text}`);
      return false;
    } finally {
      if (currentAction(ticket)) busy.value = false;
    }
  }

  async function retest(): Promise<boolean> {
    const projectId = deps.projectId();
    const target = openRemediation.value;
    if (!projectId || !target || target.findingId !== deps.findingId() || !deps.api.startExposureRetest || busy.value || !actionable()) return false;
    const ticket = identity;
    if (!available.value) {
      error.value = "复测接口尚未注册";
      return false;
    }
    busy.value = true;
    try {
      const runId = await deps.api.startExposureRetest(projectId, target.id);
      if (!currentAction(ticket)) return false;
      deps.notify("info", `复测另起第 ${runId} 轮，旧观察按原样保留`);
      await load();
      return true;
    } catch (reason) {
      if (!currentAction(ticket)) return false;
      const text = exposureErrorText(reason);
      error.value = text;
      deps.notify("error", `复测启动被拒绝：${text}`);
      return false;
    } finally {
      if (currentAction(ticket)) busy.value = false;
    }
  }

  async function mark(status: string): Promise<boolean> {
    const projectId = deps.projectId();
    const target = openRemediation.value;
    if (!projectId || !target || target.findingId !== deps.findingId() || !deps.api.markExposureRemediation || busy.value || !actionable()) return false;
    const ticket = identity;
    if (!actor.value.trim()) {
      error.value = "变更整改状态必须写明操作者";
      return false;
    }
    busy.value = true;
    try {
      const result = await deps.api.markExposureRemediation(
        projectId,
        target.id,
        status,
        actor.value.trim(),
        reason.value.trim() || remediationNote.value.trim(),
      );
      if (!currentAction(ticket)) return false;
      outcome.value = result as ExposureRetestOutcome;
      deps.notify("success", status === "closed" ? "整改已按复测依据关闭" : "整改已驳回，理由进入轨迹");
      await load();
      return true;
    } catch (reason) {
      if (!currentAction(ticket)) return false;
      const text = exposureErrorText(reason);
      error.value = text;
      deps.notify("error", `整改状态未改变：${text}`);
      return false;
    } finally {
      if (currentAction(ticket)) busy.value = false;
    }
  }

  return {
    evidence,
    trail,
    remediations,
    taxonomy,
    outcome,
    actor,
    reason,
    remediationNote,
    revision,
    busy,
    loaded,
    error,
    conflict,
    available,
    notice,
    canClose,
    remediationBlocked,
    outcomeLabel,
    openRemediation,
    latestObservationId,
    load,
    review,
    recordRemediation,
    retest,
    mark,
  };
}
