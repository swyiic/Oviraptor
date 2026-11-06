<script setup lang="ts">
import { computed } from "vue";
import { ArrowRight } from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { SentinelScan } from "../../types";
import { createSentinelLabels, formatNumber, scanTitle } from "../../presentation";

// One card per task (project + vulnerability count); a click opens that task's
// 结果与证据. Drafts are left out: they have no results to open yet.
const props = withDefaults(defineProps<{
  scans: SentinelScan[];
  vulnerabilityCount: (scanId: string) => number | undefined;
  targetCount: (scanId: string) => number;
  activeId?: string;
  limit?: number;
  title?: string;
}>(), { limit: 8, activeId: "", title: "" });
const emit = defineEmits<{ open: [scan: SentinelScan]; all: [] }>();
const { tr } = useI18n();
const { statusLabel } = createSentinelLabels(tr);

const items = computed(() => props.scans.filter((scan) => scan.status !== "draft").slice(0, props.limit));
function countText(scan: SentinelScan) {
  const count = props.vulnerabilityCount(scan.id);
  return count === undefined ? "—" : formatNumber(count);
}
function stateTone(status: string) {
  if (status === "completed") return "ok";
  if (["partial", "paused", "pausing", "completed_with_gaps"].includes(status)) return "warn";
  if (["failed", "cancelled", "interrupted"].includes(status)) return "bad";
  if (["queued", "scanning", "running", "pending", "cancel_requested"].includes(status)) return "live";
  return "idle";
}
</script>

<template>
  <section class="task-strip" :aria-label="title || tr('最近任务', 'Recent tasks')">
    <header class="task-strip-head">
      <div>
        <h3>{{ title || tr("最近任务", "Recent tasks") }}</h3>
      </div>
      <button type="button" class="button ghost compact" @click="emit('all')">
        {{ tr("全部任务", "All tasks") }}<ArrowRight :size="13" />
      </button>
    </header>
    <div v-if="items.length" class="task-strip-track">
      <button
        v-for="scan in items"
        :key="scan.id"
        type="button"
        class="task-strip-card"
        :class="{ active: scan.id === activeId, found: (vulnerabilityCount(scan.id) ?? 0) > 0 }"
        :title="`${scanTitle(scan)} · ${scan.id}`"
        @click="emit('open', scan)"
      >
        <span class="task-strip-name">{{ scan.projectName || scanTitle(scan) }}</span>
        <span v-if="scan.taskName && scan.taskName !== scan.projectName" class="task-strip-task">{{ scan.taskName }}</span>
        <span class="task-strip-count">
          <strong>{{ countText(scan) }}</strong><small>{{ tr("漏洞", "vulns") }}</small>
        </span>
        <span class="task-strip-meta">
          <i class="task-strip-dot" :class="stateTone(scan.status)" aria-hidden="true"></i>{{ statusLabel(scan.status) }}<template v-if="targetCount(scan.id)"> · {{ tr("目标", "targets") }} {{ formatNumber(targetCount(scan.id)) }}</template>
        </span>
      </button>
    </div>
    <p v-else class="empty-inline">{{ tr("还没有可查看结果的任务。", "No task has results yet.") }}</p>
  </section>
</template>

<style scoped>
.task-strip { display: grid; gap: var(--space-3, 12px); min-width: 0; margin-bottom: 16px; padding: 16px 20px 20px; border: 1px solid var(--ui-line); border-radius: var(--radius-lg); background: var(--ui-surface); box-shadow: var(--ui-shadow); }
.task-strip-head { display: flex; align-items: flex-end; justify-content: space-between; gap: 12px; }
.task-strip-head h3 { margin: 2px 0 0; color: var(--ui-ink); font-size: var(--text-heading); font-weight: 600; }
.task-strip-head .button { display: inline-flex; align-items: center; gap: 4px; }
.task-strip-track { display: grid; grid-auto-flow: column; grid-auto-columns: minmax(176px, 1fr); gap: 10px; overflow-x: auto; padding-bottom: 2px; scroll-snap-type: x proximity; }
.task-strip-card { display: grid; grid-template-rows: auto auto 1fr auto; gap: 2px; min-width: 0; min-height: 132px; padding: 12px 14px; border: 1px solid var(--ui-line); border-radius: var(--radius-md); background: var(--ui-surface); color: var(--ui-ink); font: inherit; text-align: left; cursor: pointer; scroll-snap-align: start; transition: border-color .15s ease, background-color .15s ease; }
.task-strip-card:hover { border-color: var(--ui-line-strong); background: var(--ui-surface-raised); }
.task-strip-card:focus-visible { outline: 2px solid var(--ui-accent); outline-offset: 2px; }
.task-strip-card.active { border-color: color-mix(in srgb, var(--ui-accent) 55%, var(--ui-line)); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ui-accent) 35%, transparent); }
.task-strip-name { overflow: hidden; color: var(--ui-ink); font-size: var(--text-strong); font-weight: 600; line-height: 1.45; text-overflow: ellipsis; white-space: nowrap; }
.task-strip-task { overflow: hidden; color: var(--ui-muted); font-size: var(--text-meta); line-height: 1.5; text-overflow: ellipsis; white-space: nowrap; }
.task-strip-count { display: flex; align-items: baseline; gap: 6px; align-self: end; margin-top: 8px; }
.task-strip-count strong { color: var(--ui-muted); font: 650 var(--text-display)/1.1 var(--font-sans); font-variant-numeric: tabular-nums; letter-spacing: -.02em; }
.task-strip-count small { color: var(--ui-muted); font-size: var(--text-meta); }
.task-strip-card.found .task-strip-count strong,
.task-strip-card.found .task-strip-count small { color: color-mix(in srgb, var(--red) 84%, var(--ui-ink)); }
.task-strip-meta { display: flex; align-items: center; gap: 6px; margin-top: 6px; overflow: hidden; color: var(--ui-muted); font-size: var(--text-meta); white-space: nowrap; }
.task-strip-dot { --dot: var(--ui-faint); flex: none; width: 7px; height: 7px; border-radius: 50%; background: var(--dot); }
.task-strip-dot.ok { --dot: var(--green); }
.task-strip-dot.warn { --dot: var(--amber); }
.task-strip-dot.bad { --dot: var(--red); }
.task-strip-dot.live { --dot: var(--blue); }
@media (max-width: 600px) { .task-strip { padding: 14px; } .task-strip-track { grid-auto-columns: minmax(150px, 72%); } }
</style>
