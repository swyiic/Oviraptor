// §Q4/§Q5 — the pending index queue and the per-finding evidence package. Same rules as the
// rest of the page: every read is generation-guarded, a late answer from another project can
// never overwrite the screen, and an unregistered command is reported as missing, not as empty.

import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  exposureErrorText,
  isPendingRow,
  isUnregisteredCommand,
  readEvidence,
  readExposurePage,
  resumeRefusalText,
  evidenceHeadline,
  type ExposureEvidence,
  type ExposurePage,
  type ExposurePending,
} from "./contracts";

export interface ExposureEvidenceApi {
  pageExposurePendingDiscoveries?: (
    projectId: number,
    status: string,
    pageSize: number,
    cursor: string,
  ) => Promise<unknown>;
  readExposureEvidence?: (findingId: number, expectedProjectId: number) => Promise<unknown>;
  resumeExposureScan?: (runId: number, expectedProjectId: number) => Promise<number>;
}

export interface ExposureEvidenceDeps {
  projectId: () => number | undefined;
  active: () => boolean;
  api: ExposureEvidenceApi;
  notify: (kind: "success" | "error" | "info", text: string) => void;
}

const PENDING_PAGE_SIZE = 50;

export function useExposureEvidence(deps: ExposureEvidenceDeps) {
  const pending = ref<ExposurePage<ExposurePending> | null>(null);
  const pendingLoading = ref(false);
  const pendingLoadingMore = ref(false);
  const resuming = ref<number | null>(null);
  const evidence = ref<ExposureEvidence | null>(null);
  const evidenceFor = ref<number | null>(null);
  const evidenceLoading = ref(false);
  const pendingError = ref("");
  const evidenceError = ref("");
  const readError = computed(() => [pendingError.value, evidenceError.value].filter(Boolean).join("；"));
  const pagedPending = ref(true);

  let generation = 0;
  /// FIX3-R1 证据详情专用票号：快速切换线索时，迟到的旧证据不能覆盖新选中的线索。
  let evidenceTicket = 0;
  let pendingTicket = 0;
  let pendingStatus = "unreviewed";
  let mounted = false;

  function discarded(claimed: number, projectId: number): boolean {
    return !mounted || claimed !== generation || deps.projectId() !== projectId;
  }

  const pendingNotice = computed(() => {
    if (!deps.projectId()) return "请先选择工作空间";
    if (!pagedPending.value) return "待归并队列接口尚未注册，读取被跳过而不是当作空队列";
    const current = pending.value;
    if (!current) return "尚未读取待归并队列";
    return `待归并 ${current.total} 条，已显示 ${current.items.length} 条${current.hasMore ? "，仍有更多" : "，已到底"}`;
  });
  const headline = computed(() => evidenceHeadline(evidence.value));
  const reviewNeeded = computed(() => evidence.value?.reviewNeeded ?? null);

  async function loadPending(status = "unreviewed"): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId) {
      pending.value = null;
      return;
    }
    if (!deps.api.pageExposurePendingDiscoveries) {
      pagedPending.value = false;
      pending.value = null;
      return;
    }
    const claimed = generation;
    const ticket = ++pendingTicket;
    pendingStatus = status;
    pendingLoadingMore.value = false;
    pendingLoading.value = true;
    try {
      const raw = await deps.api.pageExposurePendingDiscoveries(
        projectId,
        status,
        PENDING_PAGE_SIZE,
        "",
      );
      if (discarded(claimed, projectId) || ticket !== pendingTicket) return;
      pending.value = readExposurePage(raw, isPendingRow, "待归并队列");
      pagedPending.value = true;
      pendingError.value = "";
    } catch (error) {
      if (discarded(claimed, projectId) || ticket !== pendingTicket) return;
      // A missing command is a registration fact, never an empty queue.
      if (isUnregisteredCommand(error)) pagedPending.value = false;
      else pendingError.value = exposureErrorText(error);
    } finally {
      if (!discarded(claimed, projectId) && ticket === pendingTicket) pendingLoading.value = false;
    }
  }

  async function loadMorePending(): Promise<void> {
    const projectId = deps.projectId();
    const current = pending.value;
    if (!projectId || !current?.hasMore || !current.nextCursor || pendingLoading.value || pendingLoadingMore.value) return;
    if (!deps.api.pageExposurePendingDiscoveries) return;
    const claimed = generation;
    const ticket = pendingTicket;
    pendingLoadingMore.value = true;
    try {
      const raw = await deps.api.pageExposurePendingDiscoveries(
        projectId,
        pendingStatus,
        PENDING_PAGE_SIZE,
        current.nextCursor,
      );
      if (discarded(claimed, projectId) || ticket !== pendingTicket) return;
      const next = readExposurePage(raw, isPendingRow, "待归并队列");
      const seen = new Set(current.items.map((row) => row.id));
      pending.value = {
        ...next,
        items: [...current.items, ...next.items.filter((row) => !seen.has(row.id))],
      };
      pendingError.value = "";
    } catch (error) {
      if (discarded(claimed, projectId) || ticket !== pendingTicket) return;
      pendingError.value = exposureErrorText(error);
    } finally {
      if (!discarded(claimed, projectId) && ticket === pendingTicket) pendingLoadingMore.value = false;
    }
  }

  async function openEvidence(findingId: number): Promise<void> {
    const projectId = deps.projectId();
    if (!projectId) return;
    if (!deps.api.readExposureEvidence) {
      evidenceError.value = "证据包接口尚未注册，无法读取来源与版本";
      return;
    }
    const claimed = generation;
    const ticket = ++evidenceTicket;
    if (evidenceFor.value !== findingId) evidence.value = null;
    evidenceFor.value = findingId;
    evidenceError.value = "";
    evidenceLoading.value = true;
    try {
      const raw = await deps.api.readExposureEvidence(findingId, projectId);
      if (discarded(claimed, projectId) || ticket !== evidenceTicket) return;
      const view = readEvidence(raw);
      if (view.finding.id !== findingId) throw new Error("证据与所选线索不一致");
      evidence.value = view;
      evidenceFor.value = findingId;
      evidenceError.value = "";
    } catch (error) {
      if (discarded(claimed, projectId) || ticket !== evidenceTicket) return;
      evidenceError.value = exposureErrorText(error);
    } finally {
      if (!discarded(claimed, projectId) && ticket === evidenceTicket) evidenceLoading.value = false;
    }
  }

  function closeEvidence(): void {
    evidenceTicket += 1;
    evidence.value = null;
    evidenceFor.value = null;
    evidenceLoading.value = false;
    evidenceError.value = "";
  }

  /// Resume only ever continues a round whose frozen plan still matches. A refusal is shown
  /// as a refusal; the page never rephrases it as "started".
  async function resumeRun(runId: number): Promise<boolean> {
    const projectId = deps.projectId();
    if (!projectId || resuming.value !== null) return false;
    if (!deps.api.resumeExposureScan) {
      deps.notify("info", "恢复采集接口尚未注册");
      return false;
    }
    const claimed = generation;
    resuming.value = runId;
    try {
      await deps.api.resumeExposureScan(runId, projectId);
      if (discarded(claimed, projectId)) return false;
      deps.notify("success", `任务 #${runId} 已按冻结计划继续`);
      return true;
    } catch (error) {
      if (discarded(claimed, projectId)) return false;
      deps.notify("error", resumeRefusalText(error));
      return false;
    } finally {
      if (!discarded(claimed, projectId)) resuming.value = null;
    }
  }

  onMounted(() => {
    mounted = true;
  });
  watch(() => deps.projectId(), () => {
    generation += 1;
    pendingTicket += 1;
    pending.value = null;
    pendingLoading.value = false;
    pendingLoadingMore.value = false;
    pendingError.value = "";
    pagedPending.value = true;
    resuming.value = null;
    closeEvidence();
  }, { flush: "sync" });
  onUnmounted(() => {
    mounted = false;
    generation += 1;
  });

  return {
    pending,
    pendingLoading,
    pendingLoadingMore,
    resuming,
    pendingNotice,
    evidence,
    evidenceFor,
    evidenceLoading,
    headline,
    reviewNeeded,
    readError,
    pendingError,
    evidenceError,
    pagedPending,
    loadPending,
    loadMorePending,
    openEvidence,
    closeEvidence,
    resumeRun,
  };
}
