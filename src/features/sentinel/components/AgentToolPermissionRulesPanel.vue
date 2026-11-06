<script setup lang="ts">
import { onMounted, ref } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const props = defineProps<{ compact?: boolean }>();
void props;
const emit = defineEmits<{ notify: [type: "success" | "error", text: string] }>();
const { tr } = useI18n();

type RuleRow = { pattern: string; decision: "allow" | "ask" | "deny" };

const rules = ref<RuleRow[]>([]);
const loading = ref(false);
const saving = ref(false);
const error = ref("");

const presets: RuleRow[] = [
  { pattern: "inspect_evidence", decision: "allow" },
  { pattern: "ask_operator", decision: "allow" },
  { pattern: "call_mcp_tool", decision: "ask" },
  { pattern: "run_analysis_script", decision: "ask" },
  { pattern: "run_target_poc", decision: "ask" },
  { pattern: "probe_command_execution", decision: "ask" },
  { pattern: "probe_*", decision: "ask" },
];

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const rows = await sentinelApi.getAgentToolPermissionRules();
    rules.value = rows.map((row) => ({
      pattern: row.pattern,
      decision: (row.decision as RuleRow["decision"]) || "ask",
    }));
    if (!rules.value.length) rules.value = [...presets];
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

function addRow() {
  rules.value.push({ pattern: "", decision: "ask" });
}

function removeRow(index: number) {
  rules.value.splice(index, 1);
}

function loadPresets() {
  rules.value = [...presets];
}

async function save() {
  const payload = rules.value
    .map((row) => ({ pattern: row.pattern.trim(), decision: row.decision }))
    .filter((row) => row.pattern.length > 0);
  if (!payload.length) {
    error.value = tr("至少保留一条规则", "Keep at least one rule");
    return;
  }
  saving.value = true;
  error.value = "";
  try {
    await sentinelApi.setAgentToolPermissionRules(payload);
    emit("notify", "success", tr("工具权限规则已保存", "Tool permission rules saved"));
    await load();
  } catch (e) {
    error.value = String(e);
    emit("notify", "error", String(e));
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <section class="perm-rules" :class="{ compact }">
    <header>
      <strong>{{ tr("Agent 工具权限规则", "Agent tool permission rules") }}</strong>
      <small v-if="!compact">{{ tr("glob 模式、后者覆盖前者；deny 硬拒绝，ask 强制逐次确认（叠在目标授权之上）。", "Glob patterns, last match wins. deny blocks; ask forces confirmation on top of target authorization.") }}</small>
    </header>
    <p class="perm-note">{{ tr("POST/上传开关和当场确认在任务页的「高风险操作」旁，按任务单独控制，默认关闭。这里只改工具规则。", "The POST/upload switch and live confirmation are on the task, next to High-risk ops. They are per task and off by default. This panel only edits tool rules.") }}</p>
    <table>
      <thead><tr><th>{{ tr("模式", "Pattern") }}</th><th>{{ tr("决策", "Decision") }}</th><th /></tr></thead>
      <tbody>
        <tr v-for="(row, index) in rules" :key="index">
          <td><input v-model="row.pattern" :placeholder="tr('如 replay_http 或 probe_*', 'e.g. replay_http or probe_*')" /></td>
          <td>
            <select v-model="row.decision">
              <option value="allow">{{ tr("允许", "allow") }}</option>
              <option value="ask">{{ tr("询问", "ask") }}</option>
              <option value="deny">{{ tr("拒绝", "deny") }}</option>
            </select>
          </td>
          <td><button type="button" class="text-button" @click="removeRow(index)">×</button></td>
        </tr>
      </tbody>
    </table>
    <div class="perm-rules-actions">
      <button type="button" class="button ghost compact" @click="addRow">+ {{ tr("规则", "Rule") }}</button>
      <button type="button" class="button ghost compact" @click="loadPresets">{{ tr("恢复推荐", "Recommended") }}</button>
      <button type="button" class="button primary compact" :disabled="saving || loading" @click="save">{{ saving ? tr("保存中…", "Saving…") : tr("保存", "Save") }}</button>
    </div>
    <p v-if="error" class="error">{{ error }}</p>
  </section>
</template>

<style scoped>
.perm-rules { display: flex; flex-direction: column; gap: 10px; padding: 14px; border: 1px solid var(--border, #e0e0e0); border-radius: 10px; margin: 12px 0; }
.perm-rules.compact { padding: 10px; }
.perm-rules header small, .perm-note { display: block; color: var(--muted, #666); line-height: 1.5; margin-top: 4px; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { border-bottom: 1px solid var(--border, #eee); padding: 6px 4px; text-align: left; }
input, select { width: 100%; font: inherit; padding: 4px 6px; border: 1px solid var(--border, #ddd); border-radius: 6px; }
.perm-rules-actions { display: flex; flex-wrap: wrap; gap: 8px; }
.error { color: #b4332f; font-size: 12px; }
</style>
