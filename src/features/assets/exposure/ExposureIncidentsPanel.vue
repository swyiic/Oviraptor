<script setup lang="ts">
// G4 — the incident ledger panel: deduplicated incidents, their locations, unit links and
// action trail. Handling is recorded only; nothing on this panel contacts anyone.
import { computed, ref, watch } from "vue";
import { exposureIncidentApi } from "./api";
import { useExposureIncidents } from "./useExposureIncidents";
import {
  BASIS_LABELS,
  EVIDENCE_GRADE_LABELS,
  HANDLING_ACTIONS,
  HANDLING_LABELS,
  handlingNotice,
  INCIDENT_STATUS_LABELS,
  LINK_LEVEL_LABELS,
  type ActionForm,
} from "./incidents";
import { useI18n } from "../uiPorts";

const props = defineProps<{ projectId?: number }>();
const emit = defineEmits<{ notify: [kind: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();

const incidents = useExposureIncidents({
  projectId: () => props.projectId,
  api: exposureIncidentApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  items, hasMore, statusFilter, reviewNeededOnly, loading, available, detail, acting, notice,
  locations, locationsLoading, selectedLocations, mergeTarget, detailLoading, proposing, close,
  load, loadMore, open, act, proposeHistory, loadMoreLocations, toggleLocation, readMergeTarget, merge, split,
} = incidents;

const actor = ref("");
const reason = ref("");
const reviewStatus = ref("reviewing");
const handling = ref("notification_recorded");
const channel = ref("");
const externalReference = ref("");
const retestRunId = ref<number | undefined>(undefined);
const entityId = ref<number | undefined>(undefined);
const linkLevel = ref("candidate_link");
const evidenceIds = ref("");
const mergeTargetId = ref<number | undefined>(undefined);

watch(() => props.projectId, () => void load(), { immediate: true });
watch(() => [props.projectId, detail.value?.id], () => {
  reason.value = ""; channel.value = ""; externalReference.value = "";
  retestRunId.value = undefined; entityId.value = undefined; evidenceIds.value = ""; mergeTargetId.value = undefined;
}, { flush: "sync" });
watch(() => props.projectId, () => { actor.value = ""; }, { flush: "sync" });

const reviewStatuses = computed(() => Object.entries(INCIDENT_STATUS_LABELS).filter(([key]) => key !== "merged"));
const entities = computed(() => (detail.value?.entities as Record<string, unknown>[] | undefined) ?? []);
const events = computed(() => (detail.value?.events as Record<string, unknown>[] | undefined) ?? []);

async function submit(kind: ActionForm["action"]): Promise<void> {
  const form: ActionForm = { action: kind, actor: actor.value, reason: reason.value };
  if (kind === "review") form.status = reviewStatus.value;
  if (kind === "record_action") {
    form.handling = handling.value;
    form.channel = channel.value;
    form.externalReference = externalReference.value;
    form.retestRunId = retestRunId.value;
  }
  if (kind === "link_entity") {
    form.entityId = entityId.value;
    form.linkLevel = linkLevel.value;
    form.evidenceObservationIds = evidenceIds.value.split(/[,，\s]+/).filter(Boolean).map(Number).filter(Number.isInteger);
  }
  if (await act(form)) reason.value = "";
}

async function submitSplit(): Promise<void> {
  if (await split(actor.value, reason.value)) reason.value = "";
}

async function submitMerge(): Promise<void> {
  if (await merge(actor.value, reason.value)) {
    reason.value = "";
    mergeTargetId.value = undefined;
  }
}
</script>

<template>
  <section class="panel exposure-incidents">
    <header>
      <div><strong>{{ tr('事件台账', 'Incident ledger') }}</strong><small>{{ notice }}</small></div>
      <label v-if="available">
        <span>{{ tr('状态', 'Status') }}</span>
        <select v-model="statusFilter" @change="load()">
          <option value="open">{{ tr('未关闭（候选/复核中/已核实）', 'Open (candidate / under review / verified)') }}</option>
          <option value="">{{ tr('全部（不含已合并）', 'All (excluding merged)') }}</option>
          <option v-for="(label, key) in INCIDENT_STATUS_LABELS" :key="key" :value="key">{{ label }}</option>
        </select>
      </label>
      <label v-if="available" class="exposure-check"><input v-model="reviewNeededOnly" type="checkbox" @change="load()" />{{ tr('只看需重新复核', 'Needs re-review only') }}</label>
      <button v-if="available" type="button" :disabled="loading || proposing" :aria-busy="proposing" @click="proposeHistory">{{proposing?tr('正在整理历史轮次…', 'Processing past runs…'):tr('为历史轮次补建议', 'Add suggestions for past runs')}}</button>
    </header>
    <p class="exposure-gap">{{ handlingNotice() }}</p>
    <div v-for="item in items" :key="item.id" :class="['exposure-incident-row', { selected: detail && detail.id === item.id }]">
      <strong>#{{ item.id }} {{ item.title }}</strong>
      <span>{{ item.line }}</span>
      <em v-if="item.reviewNeeded" class="exposure-gap">{{ tr('有新位置或内容变化，需要重新复核', 'New locations or content changes; needs re-review') }}</em>
      <button @click="open(item.id)">{{ tr('查看', 'View') }}</button>
    </div>
    <div v-if="loading" class="exposure-empty">{{ tr('正在加载…', 'Loading…') }}</div>
    <footer v-if="hasMore" class="exposure-more"><button :disabled="loading" @click="loadMore">{{ tr('加载更多事件', 'Load more incidents') }}</button></footer>

    <p v-if="detailLoading" class="exposure-empty" role="status">{{ tr('正在读取所选事件…', 'Loading the selected incident…') }}</p>
    <article v-if="detail" class="exposure-incident-detail">
      <button class="button ghost" @click="close">{{ tr('收起事件详情', 'Hide incident details') }}</button>
      <h3>#{{ detail.id }} {{ detail.title }}（v{{ detail.revision }}）</h3>
      <p>
        {{ INCIDENT_STATUS_LABELS[String(detail.status)] ?? detail.status }} · {{ tr(`对应旧线索状态 ${detail.legacyFindingStatus || "—"}`, `Legacy lead status ${detail.legacyFindingStatus || "—"}`) }}
        · {{ tr(`依据 ${BASIS_LABELS[String(detail.basis)] ?? detail.basis}`, `Basis ${BASIS_LABELS[String(detail.basis)] ?? detail.basis}`) }} · {{ EVIDENCE_GRADE_LABELS[String(detail.evidenceGrade)] ?? detail.evidenceGrade }}
        · {{ HANDLING_LABELS[String(detail.handlingState)] ?? detail.handlingState }}
      </p>
      <p v-if="detail.reviewNeeded" class="exposure-gap">{{ detail.reviewNeededReason }}</p>
      <h4>{{ tr(`传播位置（已显示 ${locations.rows.length} / 共 ${locations.total}）`, `Spread locations (showing ${locations.rows.length} of ${locations.total})`) }}</h4>
      <div v-for="row in locations.rows" :key="String(row.id)" class="exposure-incident-location">
        <label v-if="available" class="exposure-check">
          <input
            type="checkbox"
            :data-location="String(row.id)"
            :checked="selectedLocations.includes(Number(row.id))"
            @change="toggleLocation(Number(row.id))"
          />{{ tr('拆出', 'Split out') }}
        </label>
        <span>{{ row.locationKind }} · {{ row.collectionMode }} · {{ row.canonicalUrl }}</span>
        <small>{{ tr(`第 ${row.runId} 轮 · 观察 ${row.observationId ?? "—"} · ${row.observedAt}`, `Run ${row.runId} · observation ${row.observationId ?? "—"} · ${row.observedAt}`) }}<template v-if="row.indexedAt"> · {{ tr(`收录 ${row.indexedAt}`, `indexed ${row.indexedAt}`) }}</template><template v-if="row.truncated"> · {{ tr('已截断', 'truncated') }}</template></small>
      </div>
      <footer v-if="locations.hasMore" class="exposure-more">
        <button data-action="more-locations" :disabled="locationsLoading" @click="loadMoreLocations">{{ tr(`加载更多位置（还有 ${locations.total - locations.rows.length} 条）`, `Load more locations (${locations.total - locations.rows.length} remaining)`) }}</button>
      </footer>
      <h4>{{ tr('涉及单位', 'Organizations involved') }}</h4>
      <div v-for="row in entities" :key="String(row.entityId) + String(row.origin)">
        {{ row.legalName }} · {{ LINK_LEVEL_LABELS[String(row.linkLevel)] ?? row.linkLevel }} · {{ row.origin }} · {{ tr(`目录 v${row.directoryVersion}`, `directory v${row.directoryVersion}`) }}
      </div>
      <div v-if="!entities.length" class="exposure-empty">{{ tr('单位未知（没有权威的逐资产单位归属，不按名称或地区推断）', 'Organization unknown (there is no authoritative per-asset organization ownership; it is not inferred from names or regions)') }}</div>
      <h4>{{ tr('动作记录', 'Action log') }}</h4>
      <div v-for="row in events" :key="String(row.id)"><small>{{ row.createdAt }} · {{ row.actor }} · {{ row.action }} · {{ row.reason }}</small></div>
      <p v-if="detail.eventsTruncated" class="exposure-gap">{{ tr('只显示最近 50 条动作', 'Showing only the latest 50 actions') }}</p>

      <div v-if="available" class="exposure-incident-actions">
        <label><span>{{ tr('操作人', 'Operator') }}</span><input v-model="actor" /></label>
        <label class="wide"><span>{{ tr('理由（必填）', 'Reason (required)') }}</span><input v-model="reason" /></label>
        <div>
          <select v-model="reviewStatus">
            <option v-for="row in reviewStatuses" :key="row[0]" :value="row[0]">{{ row[1] }}</option>
          </select>
          <button :disabled="acting" @click="submit('review')">{{ tr('写入复核结论', 'Save review decision') }}</button>
        </div>
        <div>
          <select v-model="handling">
            <option v-for="row in HANDLING_ACTIONS" :key="row.value" :value="row.value">{{ row.label }}</option>
          </select>
          <input v-model="channel" :placeholder="tr('通知对象与渠道（人工已完成）', 'Recipient and channel (already done manually)')" />
          <input v-model="externalReference" :placeholder="tr('外部工单号（可选）', 'External ticket number (optional)')" />
          <input v-model.number="retestRunId" type="number" :placeholder="tr('复测轮次号', 'Re-probe run number')" />
          <button :disabled="acting" @click="submit('record_action')">{{ tr('登记', 'Record') }}</button>
        </div>
        <div>
          <span>{{ tr(`已勾选 ${selectedLocations.length} 个位置`, `${selectedLocations.length} locations selected`) }}</span>
          <button data-action="split" :disabled="acting || !selectedLocations.length" @click="submitSplit">{{ tr('拆分为新事件（单位关联随证据带走）', 'Split into a new incident (organization links move with the evidence)') }}</button>
        </div>
        <div>
          <input v-model.number="mergeTargetId" data-field="merge-target" type="number" :placeholder="tr('并入的目标事件编号', 'Target incident ID to merge into')" />
          <button data-action="read-target" :disabled="acting" @click="readMergeTarget(Number(mergeTargetId))">{{ tr('读取目标事件', 'Load target incident') }}</button>
          <small v-if="mergeTarget">{{ tr(`目标 #${mergeTarget.id} ${mergeTarget.title}（v${mergeTarget.revision}，${INCIDENT_STATUS_LABELS[mergeTarget.status] ?? mergeTarget.status}）`, `Target #${mergeTarget.id} ${mergeTarget.title} (v${mergeTarget.revision}, ${INCIDENT_STATUS_LABELS[mergeTarget.status] ?? mergeTarget.status})`) }}</small>
          <button data-action="merge" :disabled="acting || !mergeTarget" @click="submitMerge">{{ tr('合并到目标事件', 'Merge into target incident') }}</button>
        </div>
        <div>
          <input v-model.number="entityId" type="number" :placeholder="tr('组织目录实体编号', 'Organization directory entity ID')" />
          <select v-model="linkLevel"><option v-for="(label, key) in LINK_LEVEL_LABELS" :key="key" :value="key">{{ label }}</option></select>
          <input v-model="evidenceIds" :placeholder="tr('证据观察编号（确认关联必填）', 'Evidence observation IDs (required to confirm a link)')" />
          <button :disabled="acting" @click="submit('link_entity')">{{ tr('关联单位', 'Link organization') }}</button>
        </div>
      </div>
    </article>
  </section>
</template>
