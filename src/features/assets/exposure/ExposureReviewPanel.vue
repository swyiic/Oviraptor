<script setup lang="ts">
// §E5 — review, remediation and retest for one selected finding. Nothing here rewrites an
// observation: a decision is an event carrying the revision it read, and a retest is a new
// round that has to pass the authorization gate again.
import { onMounted, ref, watch } from "vue";
import { Check, X } from "@lucide/vue";
import { exposureApi } from "./api";
import { useExposureReview } from "./useExposureReview";
import { DISPOSITION_LABELS, INFO_CATEGORY_LABELS, buildReclassifyRequest } from "./contracts";
import { submitAssetsFormOnEnter } from "../formInteraction";
import { useI18n } from "../uiPorts";

const { tr } = useI18n();
const props = defineProps<{ projectId?: number; findingId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();

const review = useExposureReview({
  projectId: () => props.projectId,
  findingId: () => props.findingId,
  api: exposureApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  trail, remediations, taxonomy, outcome, actor, reason, remediationNote, revision, busy,
  loaded, error, conflict, available, notice, canClose, outcomeLabel, openRemediation,
  latestObservationId, load, review: decide, recordRemediation, retest, mark, remediationBlocked,
  evidence,
} = review;

const reclassifyDisposition = ref("risk_candidate");
const reclassifyCategory = ref("credential");
const reclassifyBusy = ref(false);
const reclassifyNotice = ref("");
let selectionEpoch = 0;
watch(() => [props.projectId, props.findingId], () => {
  selectionEpoch += 1; reclassifyBusy.value = false; reclassifyNotice.value = "";
  reclassifyDisposition.value = "risk_candidate"; reclassifyCategory.value = "credential";
}, { flush: "sync" });

function freshOperationId(): string {
  const raw = globalThis.crypto?.randomUUID?.() ?? "";
  const id = `ui${raw.replace(/-/g, "")}`;
  return id.length >= 8 ? id : "ui-reclass-fallback";
}

async function submitReclassify(): Promise<void> {
  const projectId = props.projectId;
  const findingId = props.findingId;
  const current = evidence.value?.disposition;
  if (!projectId || !findingId || !current || reclassifyBusy.value || busy.value || !loaded.value || evidence.value?.finding.id !== findingId) return;
  const ticket = selectionEpoch;
  const built = buildReclassifyRequest({
    findingId,
    expectedClassificationId: current.classificationId ?? 0,
    disposition: reclassifyDisposition.value,
    infoCategory: reclassifyCategory.value,
    actor: actor.value,
    reason: reason.value,
  }, freshOperationId());
  if (!built.ok) {
    reclassifyNotice.value = built.error;
    return;
  }
  reclassifyBusy.value = true;
  try {
    await exposureApi.reclassifyExposureFinding(projectId, built.payload);
    if (ticket !== selectionEpoch) return;
    reclassifyNotice.value = tr("已改判（写入审计，不联系任何人）", "Reclassified (written to the audit log; no one is contacted)");
    await load();
  } catch (error) {
    if (ticket !== selectionEpoch) return;
    reclassifyNotice.value = tr(`改判未写入：${error instanceof Error ? error.message : String(error)}`, `Reclassification not written: ${error instanceof Error ? error.message : String(error)}`);
  } finally {
    if (ticket === selectionEpoch) reclassifyBusy.value = false;
  }
}

onMounted(() => {
  void load();
});
watch(() => [props.projectId, props.findingId], () => void load());
</script>

<template>
  <section class="panel exposure-review">
    <header>
      <div>
        <strong>{{ tr("人工复核与整改复测", "Manual review and remediation re-probe") }}</strong>
        <small>{{ notice }}</small>
      </div>
      <button class="button ghost" :aria-busy="busy" :disabled="!findingId || busy" @click="load">{{busy?tr('正在读取…','Loading…'):tr('重新读取','Reload')}}</button>
    </header>

    <p v-if="!findingId" class="exposure-empty">{{ tr("请先在上方选择要复核的线索（只列出当前已读到的一页）。", "Select a lead to review above first (only the page read so far is listed).") }}</p>
    <p v-else-if="!available" class="exposure-error">
      <strong>{{ tr("复核接口尚未注册", "Review API is not registered yet") }}</strong>
      <span>{{ tr("页面不会把“读不到”显示成“没有复核记录”；等主控注册 review_exposure_finding_scoped 等命令后才能写入结论。", 'This page never shows "unreadable" as "no review records"; decisions can be written once the main app registers review_exposure_finding_scoped and related commands.') }}</span>
    </p>
    <p v-else-if="error && !loaded" class="exposure-error"><strong>{{ tr("读取失败", "Read failed") }}</strong><span>{{ error }}</span></p>

    <template v-if="findingId">
      <div class="exposure-review-taxonomy">
        <span v-for="row in taxonomy" :key="row.code" :title="row.meaning">
          {{ row.label }} <strong>{{ row.count }}</strong>
        </span>
        <small>{{ tr("每个数字都来自真实行；“证据不足”不等于“没有问题”。", 'Every number comes from real rows; "insufficient evidence" does not mean "no problem".') }}</small>
      </div>

      <div class="exposure-review-form" :class="{ pending: !loaded }">
        <label><span>{{ tr("操作者", "Operator") }}</span><input v-model="actor" :placeholder="tr('姓名或工号，写进复核轨迹', 'Name or employee ID, recorded in the review trail')" /></label>
        <label class="wide"><span>{{ tr("依据（为什么会这么定）", "Basis (why this decision)") }}</span>
          <textarea v-model="reason" rows="3" :placeholder="tr('例：两份响应的字节一致，且路径可公开访问', 'e.g. Both responses are byte-identical and the path is publicly reachable')"></textarea>
        </label>
        <p v-if="conflict" class="exposure-error"><strong>{{ tr("队列已前进，本次结论没有写入", "The queue has moved on; this decision was not written") }}</strong><span>{{ error }}</span></p>
        <div class="exposure-review-actions">
          <span>{{ tr(`按 r${revision} 提交，引用观察 ${latestObservationId ?? "无"}`, `Submitting against r${revision}, citing observation ${latestObservationId ?? "none"}`) }}</span>
          <button :disabled="busy || !loaded || !available" @click="decide('reviewing')">{{ tr("标记复核中", "Mark as in review") }}</button>
          <button class="confirm" :disabled="busy || !loaded || !available" @click="decide('confirmed')"><Check :size="12" />{{ tr("确认暴露", "Confirm exposure") }}</button>
          <button :disabled="busy || !loaded || !available" @click="decide('dismissed')"><X :size="12" />{{ tr("排除", "Dismiss") }}</button>
        </div>
      </div>

      <div class="exposure-review-remediation">
        <label class="wide"><span>{{ tr("整改说明（绑定这条线索与其观察）", "Remediation note (bound to this lead and its observation)") }}</span>
          <textarea v-model="remediationNote" rows="2" :placeholder="tr('例：已从镜像删除该文件并轮换凭据', 'e.g. Removed the file from the mirror and rotated the credentials')"></textarea>
        </label>
        <div class="exposure-review-actions">
          <button v-if="!remediationBlocked" :disabled="busy || !loaded || !available || !remediationNote.trim() || !actor.trim()" @click="recordRemediation">{{ tr("登记整改", "Record remediation") }}</button>
          <span v-else class="exposure-gap">{{ remediationBlocked }}</span>
          <div v-if="remediationBlocked" role="form" class="exposure-reclassify" @keydown="submitAssetsFormOnEnter($event, submitReclassify, reclassifyBusy || !loaded)">
            <strong>{{ tr("人工改判", "Manual reclassification") }}</strong>
            <label>{{ tr("处置", "Disposition") }}
              <select v-model="reclassifyDisposition">
                <option v-for="(label, key) in DISPOSITION_LABELS" :key="key" :value="key">{{ label }}</option>
              </select>
            </label>
            <label>{{ tr("信息类别", "Information category") }}
              <select v-model="reclassifyCategory">
                <option v-for="(label, key) in INFO_CATEGORY_LABELS" :key="key" :value="key">{{ label }}</option>
              </select>
            </label>
            <button type="button" :disabled="reclassifyBusy || !loaded" @click="submitReclassify">{{ tr("写入改判", "Save reclassification") }}</button>
            <small v-if="reclassifyNotice">{{ reclassifyNotice }}</small>
          </div>
          <button :disabled="busy || !openRemediation" @click="retest">{{ tr("另起一轮复测", "Start a new re-probe round") }}</button>
          <button class="confirm" :disabled="busy || !loaded || !openRemediation || !canClose || !actor.trim()" @click="mark('closed')">{{ tr("按依据关闭", "Close with basis") }}</button>
          <button :disabled="busy || !openRemediation" @click="mark('rejected')">{{ tr("驳回", "Reject") }}</button>
        </div>
        <p v-if="openRemediation" class="exposure-review-state">
          {{ tr(
            `整改 #${openRemediation.id} · ${openRemediation.status} · 操作者 ${openRemediation.actor} · 来源轮次 ${openRemediation.runId} · 复测轮 ${openRemediation.retestRunIds.join("/") || "无"}`,
            `Remediation #${openRemediation.id} · ${openRemediation.status} · operator ${openRemediation.actor} · source round ${openRemediation.runId} · re-probe rounds ${openRemediation.retestRunIds.join("/") || "none"}`,
          ) }}
        </p>
        <p v-if="!openRemediation" class="exposure-empty">{{ tr("这条线索还没有整改记录", "This lead has no remediation records yet") }}</p>
        <p class="exposure-review-outcome">
          {{ tr(`复测依据：${outcomeLabel}`, `Re-probe basis: ${outcomeLabel}`) }}
          <template v-if="outcome?.note"> · {{ outcome.note }}</template>
          <template v-if="outcome?.previousHash">{{ tr(` · 原 ${outcome.previousHash.slice(0, 8)} → 新 ${(outcome.currentHash || "无").slice(0, 8)}`, ` · old ${outcome.previousHash.slice(0, 8)} → new ${(outcome.currentHash || "none").slice(0, 8)}`) }}</template>
        </p>
        <p v-if="outcome && !canClose" class="exposure-gap">{{ tr("关闭必须拿到复测依据（例如内容确实变了）；“本轮没取到”只说明没取到。", 'Closing requires re-probe evidence (e.g. the content really changed); "not fetched this round" only means it was not fetched.') }}</p>
      </div>

      <div class="exposure-review-trail">
        <strong>{{ tr("复核轨迹（追加，不覆盖）", "Review trail (append-only, never overwritten)") }}</strong>
        <div v-for="(row, index) in trail" :key="index">
          <span>{{ tr(`${row.createdAt} · ${row.actor} · ${row.fromStatus || "初始"} → ${row.toStatus}（按 r${row.expectedRevision}）`, `${row.createdAt} · ${row.actor} · ${row.fromStatus || "initial"} → ${row.toStatus} (against r${row.expectedRevision})`) }}</span>
          <small>{{ row.note }}<template v-if="row.references?.length">{{ tr(` · 引用 ${row.references.join(",")}`, ` · references ${row.references.join(",")}`) }}</template></small>
        </div>
        <p v-if="!trail.length">{{ tr("还没有人工复核事件", "No manual review events yet") }}</p>
      </div>

      <div v-if="remediations.length" class="exposure-review-list">
        <strong>{{ tr("本项目整改", "Remediations in this project") }}</strong>
        <div v-for="row in remediations" :key="row.id">
          <span>#{{ row.id }} · {{ row.findingTitle }} · {{ row.status }}</span>
          <small>{{ tr(`观察 ${row.observationId ?? "无"} · 轮次 ${row.runId} · ${row.actor} · ${row.createdAt}`, `Observation ${row.observationId ?? "none"} · round ${row.runId} · ${row.actor} · ${row.createdAt}`) }}{{ row.closedAt ? tr(` · 关闭于 ${row.closedAt}`, ` · closed at ${row.closedAt}`) : "" }}</small>
        </div>
      </div>
    </template>
  </section>
</template>
