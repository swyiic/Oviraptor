<script setup lang="ts">
import { Check, X, BrainCircuit, ChevronRight } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { ToolTimelineStep } from "../toolTimeline";

const props = defineProps<{ items: ToolTimelineStep[]; running: boolean; totalElapsedMs: number }>();
const { tr } = useI18n();

function seconds(ms: number) { return Math.max(1, Math.round(ms / 1000)); }
function statusLabel(item: ToolTimelineStep) {
  if (item.kind === "thinking") return tr("已思考", "Thought");
  if (item.status === "running") return tr(`运行中 · ${seconds(item.elapsedMs)}秒`, `Running · ${seconds(item.elapsedMs)}s`);
  if (item.status === "failed") return tr("已失败", "Failed");
  return item.elapsedMs > 0 ? tr(`已运行 · ${seconds(item.elapsedMs)}秒`, `Ran · ${seconds(item.elapsedMs)}s`) : tr("已运行", "Ran");
}
</script>
<template>
  <div class="ahc-timeline">
    <p v-if="props.running" class="ahc-timeline-total" aria-live="polite">
      <span class="ahc-pulse" />{{ tr(`正在执行中 · ${seconds(props.totalElapsedMs)}秒`, `Running · ${seconds(props.totalElapsedMs)}s`) }}
    </p>
    <ol class="ahc-timeline-list">
      <li v-for="item in props.items" :key="item.id" :class="['ahc-timeline-step', `is-${item.status}`]">
        <details :open="item.status === 'running'">
          <summary>
            <span class="ahc-timeline-icon">
              <Check v-if="item.status === 'done'" :size="13" />
              <X v-else-if="item.status === 'failed'" :size="13" />
              <BrainCircuit v-else-if="item.kind === 'thinking'" :size="13" />
              <span v-else class="ahc-pulse" />
            </span>
            <span class="ahc-timeline-title">{{ item.title || item.actionSummary }}</span>
            <span v-if="item.isDecision" class="ahc-timeline-decision" :title="tr('jev 决策工具:建议性,不授予任何权限', 'jev decision tool: advisory only, grants no authority')">{{ tr('决策', 'Decision') }}</span>
            <small class="ahc-timeline-status">{{ statusLabel(item) }}</small>
            <ChevronRight :size="12" class="ahc-timeline-chevron" aria-hidden="true" />
          </summary>
          <div class="ahc-timeline-detail">
            <p v-if="item.actionSummary" class="ahc-timeline-action"><strong>{{ tr("命令说明", "Action") }}</strong>{{ item.actionSummary }}</p>
            <p v-if="item.detail && item.detail !== item.actionSummary" class="ahc-timeline-preview"><strong>{{ item.status === "running" ? tr("最新输出预览", "Latest output") : tr("回执", "Receipt") }}</strong>{{ item.detail }}</p>
          </div>
        </details>
      </li>
    </ol>
  </div>
</template>
<style scoped>
.ahc-timeline { display: flex; flex-direction: column; gap: 6px; }
.ahc-timeline-total { display: flex; align-items: center; gap: 7px; margin: 0 0 4px; font-size: var(--text-micro); color: var(--app-muted, #687078); font-weight: 550; }
.ahc-timeline-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.ahc-timeline-step > details { border-radius: 8px; }
.ahc-timeline-step > details > summary { cursor: pointer; list-style: none; display: flex; align-items: center; gap: 7px; padding: 4px 2px; }
.ahc-timeline-step > details > summary::-webkit-details-marker { display: none; }
.ahc-timeline-icon { display: inline-flex; align-items: center; justify-content: center; width: 15px; color: var(--green, #22a06b); flex: none; }
.ahc-timeline-step.is-failed .ahc-timeline-icon { color: var(--orange, #a56b36); }
.ahc-timeline-step.is-thinking .ahc-timeline-icon { color: var(--app-muted, #687078); }
.ahc-timeline-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12.5px; }
.ahc-timeline-decision { flex: none; font-size: var(--text-micro); font-weight: 600; color: var(--purple, #7a5bbf); border: 1px solid var(--purple, #7a5bbf); border-radius: 5px; padding: 0 5px; }
.ahc-timeline-status { margin-left: auto; flex: none; font-size: var(--text-micro); color: var(--app-muted, #687078); }
.ahc-timeline-chevron { flex: none; color: var(--app-muted, #687078); transition: transform .15s; }
.ahc-timeline-step > details[open] > summary .ahc-timeline-chevron { transform: rotate(90deg); }
.ahc-timeline-detail { padding: 0 2px 6px 22px; display: flex; flex-direction: column; gap: 4px; }
.ahc-timeline-detail p { margin: 0; font-size: 12px; line-height: 1.6; color: var(--app-muted, #687078); white-space: pre-wrap; overflow-wrap: anywhere; }
.ahc-timeline-detail strong { display: block; font-size: var(--text-micro); color: var(--app-ink, #172033); font-weight: 600; margin-bottom: 1px; }
@media (prefers-reduced-motion: reduce) { .ahc-timeline-chevron { transition: none; } }
</style>
