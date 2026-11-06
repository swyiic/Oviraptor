<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const emit = defineEmits<{ imported: [count: number] }>();
const { tr } = useI18n();
const busy = ref(false);
const message = ref("");
const failed = ref(false);
let disposed = false;
onBeforeUnmount(() => { disposed = true; });

async function importJson() {
  if (busy.value || disposed) return;
  busy.value = true;
  failed.value = false;
  message.value = "";
  try {
    const path = await open({ multiple: false, directory: false,
      filters: [{ name: tr("Oviraptor 历史 JSON", "Oviraptor history JSON"), extensions: ["json"] }] });
    if (disposed || path === null) return;
    if (typeof path !== "string" || !path.trim()) throw new Error("invalid_import_selection");
    const count = await sentinelApi.importSentinelProject(path);
    if (disposed) return;
    if (!Number.isSafeInteger(count) || count < 0) throw new Error("invalid_import_receipt");
    message.value = tr(`已读取 ${count} 条历史记录。已删除或较旧任务的记录可能不会出现在当前清单；没有创建或恢复扫描任务。`, `Read ${count} history records. Records of deleted or older tasks may not appear in the current list; no scan task was created or resumed.`);
    emit("imported", count);
  } catch {
    if (disposed) return;
    failed.value = true;
    message.value = tr("导入未完成。请选择 32 MiB 以内的 Oviraptor 任务包 v1、项目包 v2、源码审查报告 v1 或完整审查包 v1；文件需完整且各行任务归属一致，完整审查包的两种报告及内容摘要必须匹配。", "Import did not complete. Choose an Oviraptor task bundle v1, project bundle v2, source review report v1 or full review bundle v1 of up to 32 MiB. The file must be complete with consistent task ownership on every line, and a full review bundle's two reports and content digests must match.");
  } finally {
    if (!disposed) busy.value = false;
  }
}
</script>

<template>
  <div class="historical-json-import">
    <button type="button" class="button secondary compact" :disabled="busy" @click="importJson">{{ busy ? tr('正在导入历史 JSON…', 'Importing history JSON…') : tr('导入历史 JSON', 'Import history JSON') }}</button>
    <p>{{ tr('仅导入只读历史证据；原文保留，历史结论不等于当前 Reviewer 确认，也不授予执行权限。', 'Imports read-only historical evidence only. The original text is kept; historical verdicts are not current Reviewer confirmation and grant no execution permission.') }}</p>
    <p v-if="message" :role="failed ? 'alert' : 'status'">{{ message }}</p>
  </div>
</template>

<style scoped>
.historical-json-import { display: grid; gap: .4rem; margin-block: .75rem; }
.historical-json-import button { justify-self: start; }
.historical-json-import p { margin: 0; font-size: .82rem; line-height: 1.5; overflow-wrap: anywhere; }
</style>
