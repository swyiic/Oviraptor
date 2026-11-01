<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from "vue";
import type { NativeSourceFindingsPage } from "../types";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const props = defineProps<{ scanId: string; attemptNumber: number }>();
const { tr } = useI18n();
const page = ref<NativeSourceFindingsPage>();
const loading = ref(false);
const error = ref("");
const exporting = ref(false);
const exportPath = ref("");
let serial = 0;
async function load(older = false) {
  if (older && (loading.value || page.value?.nextOffset == null)) return;
  const request = ++serial;
  const scan = props.scanId;
  const attempt = props.attemptNumber;
  const prior = older ? page.value : undefined;
  const offset = prior?.nextOffset ?? 0;
  // Even refreshes clear old evidence; a failed re-audit cannot leave an old
  // confirmed badge on screen. Older-page errors also revoke the local view.
  if (!older) { page.value = undefined; exportPath.value = ""; }
  loading.value = true;
  error.value = "";
  try {
    const next = await sentinelApi.getNativeSourceFindings(scan, attempt, offset);
    if (request !== serial) return;
    const counts = next.counts;
    const coverage = next.coverage;
    const nonempty = (value: unknown): value is string => typeof value === "string" && value.trim().length > 0;
    const digest = (value: unknown): value is string => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
    const reviewed = coverage?.status === "reviewed";
    const record = coverage?.sourceCoverageDecision;
    const verdict = record?.decision;
    const reviewedCoverageValid = record && verdict && record.schemaVersion === 1
      && record.scanId === scan && record.attemptNumber === attempt && record.rootRunId === next.rootRunId
      && nonempty(record.id) && nonempty(record.assignmentId) && nonempty(record.messageId)
      && nonempty(record.reviewerRunId) && record.reviewerRunId !== next.rootRunId
      && digest(record.modelRequestHash) && digest(record.modelResponseHash)
      && record.materialDigest === coverage?.materialDigest
      && verdict.schemaVersion === 1 && verdict.subject === "source_coverage"
      && verdict.materialDigest === record.materialDigest
      && typeof verdict.coverageSufficient === "boolean" && verdict.coverageSufficient === coverage?.coverageSufficient
      && nonempty(verdict.rationale)
      && Array.isArray(verdict.reasonCodes) && verdict.reasonCodes.length > 0 && verdict.reasonCodes.every(nonempty)
      && Array.isArray(verdict.evidenceRefs) && verdict.evidenceRefs.every(nonempty)
      && Array.isArray(verdict.outstandingGaps) && verdict.outstandingGaps.every(nonempty)
      && JSON.stringify(verdict.outstandingGaps) === JSON.stringify(coverage?.outstandingGaps)
      && (verdict.coverageSufficient ? verdict.outstandingGaps.length === 0 && verdict.evidenceRefs.length > 0
        : verdict.outstandingGaps.length > 0);
    const candidateStateValid = next.independentCandidateReviewCompleted === true
      ? (next.candidateReviewStatus == null || next.candidateReviewStatus === "completed")
        && (next.materialSubject == null || next.materialSubject === "source_candidates")
        && (!reviewed || next.candidateReviewStatus === "completed" && next.materialSubject === "source_candidates")
      : next.independentCandidateReviewCompleted === false && reviewed
        && next.candidateReviewStatus === "not_applicable_no_candidates" && next.materialSubject === "source_coverage"
        && next.materialDigest === coverage?.materialDigest && counts?.decisions === 0;
    if (next.schemaVersion !== 1 || next.scanId !== scan || next.attemptNumber !== attempt
      || next.offset !== offset || !Array.isArray(next.findings)
      || !["audited", "unverified", "not_available"].includes(next.status)
      || next.independentReviewCompleted !== reviewed
      || (coverage != null && (next.status !== "audited" || coverage.schemaVersion !== 1
        || !["prepared_not_reviewed", "reviewed"].includes(coverage.status) || coverage.executionEligible !== false
        || coverage.independentReviewCompleted !== reviewed
        || coverage.scanId !== scan || coverage.attemptNumber !== attempt || coverage.rootRunId !== next.rootRunId
        || typeof coverage.materialDigest !== "string" || !/^[a-f0-9]{64}$/.test(coverage.materialDigest)
        || !["auto", "full", "diff"].includes(coverage.requestedScope)
        || !["full", "diff", "unavailable"].includes(coverage.effectiveScope)
        || [coverage.selectedFileCount, coverage.changedPathsWithoutContentCount]
          .some(value => !Number.isSafeInteger(value) || value < 0)
        || !Array.isArray(coverage.outstandingGaps) || !coverage.outstandingGaps.every(nonempty)
        || (reviewed ? !reviewedCoverageValid
          : coverage.coverageSufficient != null || coverage.sourceCoverageDecision != null
            || !coverage.outstandingGaps.includes("source_coverage_review"))))
      || (next.status === "audited" && (!counts || !candidateStateValid
        || !nonempty(next.rootRunId) || !nonempty(next.materialDigest)
        || [counts.decisions, counts.confirmed, counts.rejected, counts.insufficient]
          .some(value => !Number.isSafeInteger(value) || value < 0)
        || counts.decisions !== counts.confirmed + counts.rejected + counts.insufficient
        || offset + next.findings.length > counts.confirmed
        || (next.nextOffset === null ? offset + next.findings.length !== counts.confirmed
          : next.nextOffset !== offset + next.findings.length || next.nextOffset >= counts.confirmed)))
      || (next.status !== "audited" && (next.findings.length !== 0 || next.counts !== null
        || next.independentCandidateReviewCompleted || next.nextOffset !== null))
      || next.findings.some(item => item.scanId !== scan || item.attemptNumber !== attempt
        || item.rootRunId !== next.rootRunId || item.materialDigest !== next.materialDigest
        || item.reviewState !== "confirmed" || !nonempty(item.id) || item.sourceDecisionId !== item.id
        || !nonempty(item.reviewerRunId) || item.reviewerRunId === item.rootRunId
        || !Array.isArray(item.evidenceRefs) || item.evidenceRefs.length === 0 || !item.evidenceRefs.every(nonempty))
      || new Set(next.findings.map(item => item.id)).size !== next.findings.length
      || (next.nextOffset !== null && (!Number.isSafeInteger(next.nextOffset) || next.nextOffset <= offset))) {
      throw new Error("source_review_scope_mismatch");
    }
    if (prior && next.status === "audited") {
      if (next.rootRunId !== prior.rootRunId || next.materialDigest !== prior.materialDigest
        || next.independentCandidateReviewCompleted !== prior.independentCandidateReviewCompleted
        || next.candidateReviewStatus !== prior.candidateReviewStatus || next.materialSubject !== prior.materialSubject
        || JSON.stringify(next.coverage ?? null) !== JSON.stringify(prior.coverage ?? null)
        || JSON.stringify(next.counts) !== JSON.stringify(prior.counts)
        || next.findings.some(item => prior.findings.some(previous => previous.id === item.id))) {
        throw new Error("source_review_page_changed");
      }
      page.value = { ...next, findings: [...prior.findings, ...next.findings] };
    } else page.value = next;
  } catch {
    if (request === serial) {
      page.value = undefined;
      error.value = tr("此轮源码审查结果暂时不可核验，未展示任何确认发现。", "This attempt's source review results can't be verified right now; no confirmed findings are shown.");
    }
  } finally {
    if (request === serial) loading.value = false;
  }
}
async function exportReview(format?: "json" | "sarif" | "bundle") {
  if (exporting.value || loading.value || page.value?.status !== "audited") return;
  const request = serial;
  exporting.value = true;
  exportPath.value = "";
  try {
    const path = format
      ? await sentinelApi.exportNativeSourceFindings(props.scanId, props.attemptNumber, format)
      : await sentinelApi.exportNativeSourceFindings(props.scanId, props.attemptNumber);
    if (request !== serial) return;
    if (typeof path !== "string" || !path.trim()) throw new Error("invalid_export_path");
    exportPath.value = path;
  } catch {
    if (request === serial) {
      ++serial;
      page.value = undefined;
      error.value = tr("导出时重新核验或写入失败，已撤销页面确认展示；请重新核验后重试。", "Re-verification or writing failed during export, so the confirmed results on this page were withdrawn. Re-verify and try again.");
    }
  } finally { exporting.value = false; }
}
watch(() => [props.scanId, props.attemptNumber], () => { void load(); }, { immediate: true });
onBeforeUnmount(() => { ++serial; });
</script>

<template>
  <section class="task-attempt-log source-review-evidence" :aria-label="tr('源码独立审查结果', 'Independent source review results')" :aria-busy="loading">
    <header><strong>{{ tr(`第 ${attemptNumber} 次执行 · 源码独立审查`, `Attempt ${attemptNumber} · Independent source review`) }}</strong><button type="button" class="button ghost compact" :disabled="loading || exporting" @click="load()">{{ tr('重新核验', 'Re-verify') }}</button></header>
    <p>{{ tr('候选审查与总体覆盖分别记录；候选已审查不代表扫描充分，也不代表 CI 门禁通过。', "Candidate review and overall coverage are recorded separately; reviewed candidates don't mean the scan was sufficient or that the CI gate passed.") }}</p>
    <p v-if="loading" role="status">{{ tr('正在核验此轮持久化裁决与证据…', "Verifying this attempt's stored verdicts and evidence…") }}</p>
    <p v-if="exportPath" role="status">{{ tr('已导出本轮审查快照：', "Exported this attempt's review snapshot: ") }}<code>{{ exportPath }}</code>{{ tr('。仅在导出时核验，不授予后续执行权限，不代表总体覆盖或 CI 通过。', '. Verified only at export time; it grants no further execution permission and does not mean overall coverage or a CI pass.') }}</p>
    <p v-if="error" role="alert">{{ error }}</p>
    <template v-else-if="page">
      <p v-if="page.status === 'not_available'">{{ tr('此轮尚无原生源码审查记录；这不表示没有漏洞。', 'No native source review record for this attempt yet; this does not mean there are no vulnerabilities.') }}</p>
      <p v-else-if="page.status === 'unverified'" role="alert">{{ tr('此轮尚无可核验的完整正式裁决：可能尚未交付、属于旧版合同或证据不完整。未展示确认发现，请到执行记录核对。', 'This attempt has no complete, verifiable formal verdict yet: it may not have been delivered, may use an older contract, or may have incomplete evidence. No confirmed findings are shown; check the run record.') }}</p>
      <template v-else-if="page.counts">
        <p class="source-review-counts">{{ tr(`已核验 ${page.counts.decisions} 条裁决 · 已确认 ${page.counts.confirmed} · 已排除 ${page.counts.rejected} · 证据不足 ${page.counts.insufficient}`, `${page.counts.decisions} verdicts verified · ${page.counts.confirmed} confirmed · ${page.counts.rejected} rejected · ${page.counts.insufficient} with insufficient evidence`) }}</p>
        <p v-if="page.independentReviewCompleted">{{ tr('总体覆盖独立审查：已完成。', 'Independent overall coverage review: complete. ') }}{{ page.coverage?.coverageSufficient ? tr('仅在本次冻结范围内裁决为覆盖充分，仍不代表 CI 通过。', 'Coverage was judged sufficient only within this frozen scope; this still does not mean CI passed.') : tr('覆盖仍不充分，缺口保留；审查完成不代表 CI 通过。', 'Coverage is still insufficient and gaps remain; a completed review does not mean CI passed.') }}</p>
        <p v-else>{{ tr('总体覆盖独立审查：尚未完成。', 'Independent overall coverage review: not complete yet.') }}</p>
        <p v-if="page.candidateReviewStatus === 'not_applicable_no_candidates'">{{ tr('本轮没有待审候选，未创建候选 Reviewer；不能将其视为独立候选审查已完成。', 'No candidates to review in this attempt, so no candidate Reviewer was created; this does not count as a completed independent candidate review.') }}</p>
        <details v-if="page.coverage">
          <summary>{{ page.coverage.status === 'reviewed' ? tr('覆盖裁决与交付证据已核验', 'Coverage verdict and delivery evidence verified') : tr('覆盖材料已准备，不代表覆盖审查已完成', 'Coverage material prepared; this does not mean the coverage review is complete') }} · {{ tr(`${page.coverage.outstandingGaps.length} 项未闭合缺口`, `${page.coverage.outstandingGaps.length} open gaps`) }}</summary>
          <p>{{ tr(`请求范围 ${page.coverage.requestedScope} · 实际范围 ${page.coverage.effectiveScope} · 所选文件 ${page.coverage.selectedFileCount} 个；所选文件不代表逐文件审查通过。`, `Requested scope ${page.coverage.requestedScope} · Effective scope ${page.coverage.effectiveScope} · ${page.coverage.selectedFileCount} files selected; selected files do not mean each file passed review.`) }}</p>
          <p v-if="page.coverage.changedPathsWithoutContentCount">{{ tr(`${page.coverage.changedPathsWithoutContentCount} 个变更路径没有选中内容；不能直接认定为已删除或已检查。`, `${page.coverage.changedPathsWithoutContentCount} changed paths have no selected content; they can't be assumed deleted or checked.`) }}</p>
          <ul><li v-for="(gap, index) in page.coverage.outstandingGaps.slice(0, 20)" :key="index"><code>{{ gap }}</code></li></ul>
          <p v-if="page.coverage.outstandingGaps.length > 20">{{ tr('此处仅展示前 20 项；完整缺口保留在审查导出中。', 'Only the first 20 are shown here; the full gap list is kept in the review export.') }}</p>
          <small>{{ tr(`覆盖材料摘要 ${page.coverage.materialDigest}；此材料不授予执行权限。`, `Coverage material digest ${page.coverage.materialDigest}; this material grants no execution permission.`) }}</small>
          <template v-if="page.coverage.sourceCoverageDecision">
            <p>{{ page.coverage.sourceCoverageDecision.decision.rationale }}</p>
            <small>{{ tr(`覆盖裁决 ${page.coverage.sourceCoverageDecision.id} · Reviewer ${page.coverage.sourceCoverageDecision.reviewerRunId}`, `Coverage verdict ${page.coverage.sourceCoverageDecision.id} · Reviewer ${page.coverage.sourceCoverageDecision.reviewerRunId}`) }}</small>
            <ul><li v-for="reference in page.coverage.sourceCoverageDecision.decision.evidenceRefs" :key="reference"><code>{{ reference }}</code></li></ul>
          </template>
        </details>
        <p>{{ tr('CI 任务的导出同时保留本轮冻结版本、分析器、发布策略和门禁结果；历史导出不会恢复执行权限。', "Exports from CI tasks also keep this attempt's frozen version, analyzers, release policy, and gate results; historical exports do not restore execution permission.") }}</p>
        <button type="button" class="button ghost compact" :disabled="loading || exporting" @click="exportReview()">{{ exporting ? tr('正在重新核验并导出…', 'Re-verifying and exporting…') : tr('导出本轮审查 JSON（全部确认发现）', 'Export review JSON (all confirmed findings)') }}</button>
        <button type="button" class="button ghost compact" :disabled="loading || exporting" @click="exportReview('sarif')">{{ tr('导出本轮审查 SARIF（全部确认发现）', 'Export review SARIF (all confirmed findings)') }}</button>
        <button type="button" class="button ghost compact" :disabled="loading || exporting" @click="exportReview('bundle')">{{ tr('导出完整审查包（JSON + SARIF）', 'Export full review bundle (JSON + SARIF)') }}</button>
        <p>{{ tr('完整审查包是单个 JSON 容器，两种报告来自同一次核验并整体发布；可通过历史 JSON 导入入口回读。需要交给 SARIF 工具时使用上方单独 SARIF 导出。', 'The full review bundle is a single JSON container: both reports come from the same verification and are published together, and it can be read back through the historical JSON import. For SARIF tools, use the separate SARIF export above.') }}</p>
        <p v-if="!page.counts.confirmed">{{ tr('本轮没有确认发现；不等于系统安全或覆盖完整。', 'No confirmed findings in this attempt; this does not mean the system is secure or coverage is complete.') }}</p>
        <ol class="task-detail-records">
          <li v-for="item in page.findings" :key="item.id">
            <strong>{{ item.title || tr('未命名源码发现', 'Untitled source finding') }} · {{ item.severity }} · {{ tr('独立审查已确认', 'Confirmed by independent review') }}</strong>
            <span>{{ item.path || tr('未记录路径', 'No path recorded') }}<template v-if="item.line">:{{ item.line }}</template><template v-if="item.cwe"> · {{ item.cwe }}</template></span>
            <p>{{ item.rationale }}</p>
            <details><summary>{{ tr('审查来源与证据引用', 'Review source and evidence references') }}</summary>
              <small>{{ tr(`裁决 ${item.sourceDecisionId} · Reviewer ${item.reviewerRunId}`, `Verdict ${item.sourceDecisionId} · Reviewer ${item.reviewerRunId}`) }}</small>
              <ul><li v-for="reference in item.evidenceRefs" :key="reference"><code>{{ reference }}</code></li></ul>
            </details>
          </li>
        </ol>
        <button v-if="page.nextOffset !== null" type="button" class="button ghost compact" :disabled="loading || exporting" @click="load(true)">{{ tr('加载更多确认发现', 'Load more confirmed findings') }}</button>
      </template>
    </template>
  </section>
</template>

<style scoped>
.source-review-evidence { min-width: 0; }
.source-review-evidence p, .source-review-evidence code, .source-review-evidence small,
.source-review-evidence strong, .source-review-evidence span { overflow-wrap: anywhere; white-space: pre-wrap; }
.source-review-evidence header { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 8px; }
.source-review-evidence details { margin-top: 8px; }
.source-review-evidence summary { cursor: pointer; }
</style>
