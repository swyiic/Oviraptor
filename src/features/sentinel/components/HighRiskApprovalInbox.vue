<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

// The third gate's operator surface: high-risk actions the agent has requested
// and that are waiting on a human decision. Each is a SINGLE action bound to the
// scan attempt and the live fencing token; approval is single-use. Renders
// nothing when there is nothing pending, so it only appears when a decision is due.
const props = defineProps<{ scanId: string }>();
const { tr } = useI18n();

interface PendingAction {
  requestId: string;
  attemptNumber: number;
  capabilityClass: string;
  actionSummary: string;
  target: string;
  reversible: boolean;
  createdAt: string;
}

const pending = ref<PendingAction[]>([]);
const deciding = ref("");
const error = ref("");
let timer: ReturnType<typeof setInterval> | undefined;
let disposed = false;

function classLabel(cls: string): string {
  switch (cls) {
    case "code_exec": return tr("命令执行 / RCE", "Command execution / RCE");
    case "target_write": return tr("对目标写入", "Write to target");
    case "egress": return tr("出网 / OOB 外带", "Egress / OOB");
    case "internal_scope": return tr("内网 / SSRF", "Internal / SSRF");
    case "persistence": return tr("持久化", "Persistence");
    case "exfil": return tr("数据外带", "Data exfiltration");
    case "waf_evasion": return tr("WAF 绕过", "WAF evasion");
    default: return cls;
  }
}

async function refresh() {
  if (!props.scanId) {
    pending.value = [];
    return;
  }
  try {
    const rows = (await sentinelApi.listPendingHighRiskActions(props.scanId)) as PendingAction[];
    if (!disposed) {
      pending.value = rows;
      error.value = "";
    }
  } catch (e) {
    if (!disposed) error.value = String(e);
  }
}

async function decide(requestId: string, approve: boolean) {
  if (deciding.value) return;
  deciding.value = requestId;
  error.value = "";
  try {
    if (approve) await sentinelApi.approveHighRiskAction(requestId);
    else await sentinelApi.rejectHighRiskAction(requestId);
    await refresh();
  } catch (e) {
    error.value = String(e);
  } finally {
    deciding.value = "";
  }
}

onMounted(() => {
  refresh();
  timer = setInterval(refresh, 5000);
});
onUnmounted(() => {
  disposed = true;
  if (timer) clearInterval(timer);
});
watch(() => props.scanId, () => refresh());
</script>

<template>
  <section v-if="pending.length" class="high-risk-inbox">
    <header class="hri-head">
      <strong>{{ tr("高危动作待批准", "High-risk actions awaiting approval") }}</strong>
      <small>{{ tr("每个动作由你单独批准一次；批准只对这一次有效。", "You approve each action once; approval is single-use.") }}</small>
    </header>
    <ul class="hri-list">
      <li v-for="item in pending" :key="item.requestId" class="hri-item">
        <div class="hri-row">
          <span class="hri-class">{{ classLabel(item.capabilityClass) }}</span>
          <span v-if="!item.reversible" class="hri-irreversible">{{ tr("不可逆", "irreversible") }}</span>
        </div>
        <p class="hri-summary">{{ item.actionSummary }}</p>
        <p v-if="item.target" class="hri-target">{{ item.target }}</p>
        <div class="hri-actions">
          <button class="button primary" :disabled="deciding === item.requestId" @click="decide(item.requestId, true)">
            {{ tr("批准一次", "Approve once") }}
          </button>
          <button class="button ghost" :disabled="deciding === item.requestId" @click="decide(item.requestId, false)">
            {{ tr("拒绝", "Reject") }}
          </button>
        </div>
      </li>
    </ul>
    <small v-if="error" class="hri-error">{{ error }}</small>
  </section>
</template>

<style scoped>
.high-risk-inbox {
  margin: 12px 0;
  padding: 14px 16px;
  border: 1px solid color-mix(in srgb, #d9534f 45%, var(--border, #d0d0d0));
  border-radius: 10px;
  background: color-mix(in srgb, #d9534f 6%, var(--surface, #fff));
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.hri-head { display: flex; flex-direction: column; gap: 3px; }
.hri-head strong { color: color-mix(in srgb, #b4332f 80%, var(--ink, #1a1a1a)); font-size: 0.95rem; }
.hri-head small { color: var(--muted, #6a6a6a); font-size: 0.8rem; line-height: 1.5; }
.hri-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
.hri-item { padding: 10px 12px; border: 1px solid var(--border, #e0e0e0); border-radius: 8px; background: var(--surface, #fff); display: flex; flex-direction: column; gap: 6px; }
.hri-row { display: flex; align-items: center; gap: 8px; }
.hri-class { font-weight: 600; color: var(--ink, #1a1a1a); font-size: 0.9rem; }
.hri-irreversible { font-size: 0.72rem; padding: 1px 7px; border-radius: 999px; background: color-mix(in srgb, #d9534f 16%, transparent); color: #b4332f; }
.hri-summary { margin: 0; color: var(--ink, #1a1a1a); font-size: 0.86rem; line-height: 1.5; word-break: break-word; }
.hri-target { margin: 0; color: var(--muted, #6a6a6a); font-size: 0.78rem; word-break: break-all; }
.hri-actions { display: flex; gap: 8px; margin-top: 2px; }
.hri-error { color: #b4332f; font-size: 0.78rem; }
</style>
