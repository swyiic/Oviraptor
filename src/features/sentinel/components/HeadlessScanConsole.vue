<script setup lang="ts">
import { ref } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const props = defineProps<{ projectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();

const url = ref("");
const taskName = ref("");
const wait = ref(true);
const maxWaitSecs = ref(900);
const busy = ref(false);
const lastResult = ref<Record<string, unknown>>();
const waitStatus = ref("");

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function pollHeadlessJob(jobId: string, maxSecs: number) {
  const deadline = Date.now() + maxSecs * 1000;
  while (Date.now() < deadline) {
    const job = await sentinelApi.getHeadlessWaitJob(jobId);
    const status = String(job.status ?? "running");
    waitStatus.value = status;
    if (status !== "running") {
      lastResult.value = {
        ...(lastResult.value ?? {}),
        ...(typeof job.result === "object" && job.result ? (job.result as Record<string, unknown>) : {}),
        job,
      };
      return job;
    }
    await sleep(2000);
  }
  waitStatus.value = "poll_timeout";
  return null;
}

async function run() {
  const target = url.value.trim();
  if (!props.projectId || !target || busy.value) return;
  busy.value = true;
  waitStatus.value = "";
  try {
    const result = await sentinelApi.runHeadlessUrlScan({
      projectId: props.projectId,
      taskName: taskName.value.trim() || target,
      urls: [target],
      scanMode: "standard",
      wait: wait.value,
      maxWaitSecs: maxWaitSecs.value,
    });
    lastResult.value = result as Record<string, unknown>;
    const jobId = (result as { jobId?: string }).jobId;
    if (wait.value && jobId) {
      waitStatus.value = "running";
      emit("notify", "info", tr("已在后台等待扫描结束…", "Waiting for scan in background…"));
      const finished = await pollHeadlessJob(jobId, maxWaitSecs.value + 30);
      const snap = (finished?.result as { snapshot?: { status?: string; findingCount?: number } } | undefined)?.snapshot
        ?? (lastResult.value?.snapshot as { status?: string; findingCount?: number } | undefined);
      emit(
        "notify",
        finished ? "success" : "error",
        tr(
          `后台等待结束 · 状态 ${snap?.status ?? "?"} · 漏洞 ${snap?.findingCount ?? 0}`,
          `Background wait done · status ${snap?.status ?? "?"} · findings ${snap?.findingCount ?? 0}`,
        ),
      );
    } else {
      const snap = (result as { snapshot?: { status?: string; findingCount?: number } }).snapshot;
      emit(
        "notify",
        "success",
        tr(
          `已启动 · 状态 ${snap?.status ?? "?"} · 漏洞 ${snap?.findingCount ?? 0}`,
          `Started · status ${snap?.status ?? "?"} · findings ${snap?.findingCount ?? 0}`,
        ),
      );
    }
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    busy.value = false;
  }
}

async function exportProject() {
  if (!props.projectId || busy.value) return;
  busy.value = true;
  try {
    const json = await sentinelApi.exportProjectAgentStats(props.projectId, 100);
    const blob = new Blob([json], { type: "application/json" });
    const href = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = href;
    a.download = `project-${props.projectId}-agent-stats.json`;
    a.click();
    URL.revokeObjectURL(href);
    emit("notify", "success", tr("已导出工作区统计", "Workspace stats exported"));
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <section class="headless panel">
    <header>
      <strong>{{ tr("无人值守 URL 扫描", "Headless URL scan") }}</strong>
      <ul class="entry-diff">
        <li>{{ tr("聊天：发送「测 https://…」会立刻建任务并启动，不等待结束。", "Chat: sending “测 https://…” creates and starts a task immediately and does not wait.") }}</li>
        <li>{{ tr("这里：同样建任务并启动。勾选等待时在后台轮询，界面不卡住。", "Here: also creates and starts a task. If you wait, polling stays in the background and the UI does not block.") }}</li>
        <li>{{ tr("命令行 oviraptor-headless：给脚本用。加 --wait 会一直等到结束或超时。", "CLI oviraptor-headless: for scripts. --wait blocks until the scan finishes or times out.") }}</li>
      </ul>
    </header>
    <div v-if="!projectId" class="muted">{{ tr("请先选择工作区/项目", "Select a workspace project first") }}</div>
    <template v-else>
      <input v-model="url" type="url" :placeholder="tr('https://目标/', 'https://target/')" />
      <input v-model="taskName" :placeholder="tr('任务名（可选）', 'Task name (optional)')" />
      <label><input v-model="wait" type="checkbox" />{{ tr("等待执行结束（后台）", "Wait until finished (background)") }}</label>
      <label v-if="wait">{{ tr("最长等待（秒）", "Max wait (sec)") }}<input v-model.number="maxWaitSecs" type="number" min="30" max="7200" /></label>
      <p v-if="waitStatus" class="muted">{{ tr("等待状态", "Wait status") }}: {{ waitStatus }}</p>
      <div class="actions">
        <button type="button" class="button primary compact" :disabled="busy || !url.trim()" @click="run">{{ busy ? tr("运行中…", "Running…") : tr("运行", "Run") }}</button>
        <button type="button" class="button ghost compact" :disabled="busy" @click="exportProject">{{ tr("导出工作区统计", "Export workspace stats") }}</button>
      </div>
      <details v-if="lastResult"><summary>{{ tr("上次结果 JSON", "Last result JSON") }}</summary><pre>{{ JSON.stringify(lastResult, null, 2) }}</pre></details>
    </template>
  </section>
</template>

<style scoped>
.headless { display: flex; flex-direction: column; gap: 8px; padding: 14px; margin: 12px 0; border: 1px solid var(--border, #e0e0e0); border-radius: 10px; }
.entry-diff { margin: 6px 0 0; padding-left: 18px; color: var(--muted, #666); font-size: 12px; line-height: 1.5; }
.headless input[type="url"], .headless input:not([type="checkbox"]):not([type="number"]) { font: inherit; padding: 6px 8px; border: 1px solid var(--border, #ddd); border-radius: 6px; }
.actions { display: flex; flex-wrap: wrap; gap: 8px; }
pre { max-height: 240px; overflow: auto; font-size: 11px; }
.muted { color: var(--muted, #777); }
</style>
