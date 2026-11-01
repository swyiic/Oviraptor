<script setup lang="ts">
// web→source→greybox orchestration panel. Shown on a web scan's 执行概览.
// Reflects the per-scan source-fetch consent, runs the controlled collection,
// and hands off a one-click greybox reproduction. The fetch itself is gated in
// the backend (no consent → refused); this panel never fetches on its own, it
// calls `orchestrate_source_audit`, which enforces the gate.
import { onMounted, ref, watch } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";
import type { SourceAuditReport } from "../types";

const props = defineProps<{
  scanId: string;
  projectId: number;
  status: string;
  refreshKey?: string | number;
}>();

const emit = defineEmits<{
  (event: "notify", level: "success" | "error", message: string): void;
}>();
const { tr } = useI18n();

const report = ref<SourceAuditReport | null>(null);
const busy = ref(false);
const starting = ref("");
// Auto-run the collection at most once per scan, and only after the scan has
// finished — never mid-run, never in a loop.
const autoRan = ref<string>("");

function isTerminal(status: string): boolean {
  return !["scanning", "draft", "queued", "paused"].includes(status);
}

async function load() {
  try {
    report.value = await sentinelApi.sourceAuditReport(props.scanId);
    maybeAutoRun();
  } catch (error) {
    report.value = null;
    emit("notify", "error", tr(`源码审计状态读取失败：${String(error)}`, `Failed to read source audit status: ${String(error)}`));
  }
}

function maybeAutoRun() {
  const current = report.value;
  if (!current || busy.value) return;
  if (autoRan.value === props.scanId) return;
  if (!current.consent || !isTerminal(props.status)) return;
  if (current.matched.length === 0) return;
  if (current.collected.length >= current.matched.length) return;
  autoRan.value = props.scanId;
  void runOrchestration();
}

async function runOrchestration() {
  if (busy.value) return;
  busy.value = true;
  try {
    report.value = await sentinelApi.orchestrateSourceAudit(props.scanId);
  } catch (error) {
    emit("notify", "error", tr(`源码编排失败：${String(error)}`, `Source orchestration failed: ${String(error)}`));
  } finally {
    busy.value = false;
  }
}

async function startReproduction(productId: string) {
  const current = report.value;
  const source = current?.collected.find((item) => item.productId === productId);
  if (!source || starting.value) return;
  starting.value = productId;
  try {
    // Build a greybox scan that correlates the collected source with the same
    // authorized URLs. This starts a scan (it spends budget) — an explicit user
    // action, still subject to the backend's budget/financial gates.
    await sentinelApi.startWorkbenchScan({
      projectId: props.projectId,
      taskName: `源码审计复现：${source.productName}`, // i18n-ignore (persisted task name sent to the backend)
      scanType: "greybox",
      urls: source.targetUrls,
      sourcePath: source.quarantinePath,
      skillIds: [],
      instruction: `复现对 ${source.productName}（上游 ${source.repoUrl}@${source.gitRef}）源码审计发现的问题，并与运行时 URL 关联验证。上游为近似参照，以实测为准。`, // i18n-ignore (Agent instruction sent to the backend)
      scanMode: "standard",
      scopeMode: "full",
      diffBase: "",
      environment: "staging",
      authProfileName: "",
      authType: "none",
      authHeaderName: "",
      authValue: "",
      authSessionId: "",
      authSessionIds: [],
      authSessionScopeId: "",
      ciProvider: "",
      repositoryUrl: "",
      branch: "",
      commitSha: "",
      buildId: "",
      maxCritical: 0,
      maxHigh: 0,
      blockRelease: false,
    });
    emit("notify", "success", tr(`已创建灰盒复现任务：${source.productName}。可在任务总览查看进度。`, `Greybox reproduction task created: ${source.productName}. Track progress in the task overview.`));
  } catch (error) {
    emit("notify", "error", tr(`创建灰盒复现任务失败：${String(error)}`, `Failed to create greybox reproduction task: ${String(error)}`));
  } finally {
    starting.value = "";
  }
}

onMounted(load);
watch(() => [props.scanId, props.refreshKey], load);
</script>

<template>
  <section v-if="report" class="source-audit-panel">
    <header class="source-audit-head">
      <div>
        <h3>{{ tr("源码审计编排", "Source audit orchestration") }}</h3>
        <p>{{ report.note }}</p>
      </div>
      <span class="consent-badge" :class="{ on: report.consent }">
        {{ report.consent ? tr("已授权拉源", "Source fetch authorized") : tr("未授权拉源", "Source fetch not authorized") }}
      </span>
    </header>

    <div v-if="report.consent" class="source-audit-body">
      <div v-if="report.matched.length" class="matched-row">
        <span class="label">{{ tr("匹配上游：", "Matched upstream:") }}</span>
        <span v-for="m in report.matched" :key="m.productId" class="chip">
          {{ m.productName }}<small>{{ m.host }}</small>
        </span>
      </div>
      <p v-else-if="report.identified.length" class="muted">
        {{ tr(`识别到 ${report.identified.join("、")}，但暂无可用上游映射。`, `Identified ${report.identified.join(", ")}, but no upstream mapping is available yet.`) }}
      </p>
      <p v-else class="muted">{{ tr("尚未识别到可映射的开源系统。", "No mappable open-source system identified yet.") }}</p>

      <button
        v-if="report.matched.length && report.collected.length < report.matched.length"
        type="button"
        class="button ghost"
        :disabled="busy"
        @click="runOrchestration"
      >
        {{ busy ? tr("正在受控拉源…", "Fetching source under controls…") : tr("运行源码编排（拉取下一个）", "Run source orchestration (fetch next)") }}
      </button>

      <ul v-if="report.collected.length" class="collected-list">
        <li v-for="c in report.collected" :key="c.productId">
          <div class="collected-meta">
            <strong>{{ c.productName }}</strong>
            <code class="repo">{{ c.repoUrl }}@{{ c.gitRef }}</code>
            <code class="path">{{ c.quarantinePath }}</code>
            <small>{{ tr(`${c.targetUrls.length} 个授权 URL · 隔离区源码，仅供灰盒/沙箱分析`, `${c.targetUrls.length} authorized URLs · Quarantined source, for greybox/sandbox analysis only`) }}</small>
          </div>
          <button
            type="button"
            class="button primary"
            :disabled="Boolean(starting) || !c.targetUrls.length"
            @click="startReproduction(c.productId)"
          >
            {{ starting === c.productId ? tr("创建中…", "Creating…") : tr("一键创建灰盒复现", "Create greybox reproduction") }}
          </button>
        </li>
      </ul>
    </div>
    <p v-else class="muted gated">
      {{ tr("创建任务时未勾选「允许拉源」，不会拉取任何源码，也不会进行源码审计复现。", '"Allow source fetch" was not selected when the task was created, so no source is fetched and no source audit reproduction runs.') }}
    </p>
  </section>
</template>

<style scoped>
.source-audit-panel {
  border: 1px solid var(--border, rgba(148, 163, 184, 0.28));
  border-radius: 12px;
  padding: 14px 16px;
  background: var(--surface-muted, rgba(148, 163, 184, 0.06));
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.source-audit-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
}
.source-audit-head .eyebrow {
  font-size: 11px;
  letter-spacing: 0.08em;
  opacity: 0.6;
}
.source-audit-head h3 {
  margin: 2px 0;
  font-size: 15px;
}
.source-audit-head p {
  margin: 0;
  font-size: 12px;
  opacity: 0.75;
}
.consent-badge {
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 999px;
  white-space: nowrap;
  background: rgba(148, 163, 184, 0.18);
  opacity: 0.8;
}
.consent-badge.on {
  background: rgba(34, 197, 94, 0.18);
  color: var(--success, #16a34a);
}
.source-audit-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.matched-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
  font-size: 12px;
}
.matched-row .label {
  opacity: 0.7;
}
.chip {
  display: inline-flex;
  gap: 5px;
  align-items: baseline;
  padding: 3px 8px;
  border-radius: 8px;
  background: rgba(99, 102, 241, 0.14);
}
.chip small {
  opacity: 0.6;
}
.muted {
  font-size: 12px;
  opacity: 0.7;
  margin: 0;
}
.muted.gated {
  font-style: italic;
}
.collected-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.collected-list li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 8px 10px;
  border-radius: 10px;
  background: var(--surface, rgba(255, 255, 255, 0.04));
}
.collected-meta {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.collected-meta code {
  font-size: 11px;
  opacity: 0.8;
  word-break: break-all;
}
.collected-meta small {
  font-size: 11px;
  opacity: 0.6;
}
</style>
