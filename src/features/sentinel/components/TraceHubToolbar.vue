<script setup lang="ts">
import { BookOpen, Download, Layers, RefreshCw, Upload, Wrench } from "@lucide/vue";
import { useI18n } from "../../../i18n";

defineProps<{
  sourceInput: string;
  aggregateType: string;
  busy: string;
  loading: boolean;
  knowledgeCount: number;
}>();
const emit = defineEmits<{
  "update:sourceInput": [value: string];
  "update:aggregateType": [value: string];
  "ingest-source": [forceRefresh: boolean];
  refresh: [];
  aggregate: [];
  "import-knowledge": [];
  "export-knowledge": [];
}>();
const { tr } = useI18n();
</script>

<template>
  <div class="trace-toolbar">
    <div>
      <span class="eyebrow">MODEL ANALYSIS</span>
      <h2>{{ tr("模型分析", "Model analysis") }}</h2>
      <p>{{ tr(
        "按扫描任务查看最终模型请求、Agent、工具与 Token；可将单任务轨迹沉淀为知识，再聚合同类任务并生成可复用 Skill。",
        "Inspect exact model requests, agents, tools and tokens per scan; save task traces as knowledge, aggregate similar scans, and create reusable Skills.",
      ) }}</p>
    </div>
    <div class="trace-toolbar-actions">
      <div class="trace-source-row">
        <input :value="sourceInput" :placeholder="tr('任意安全文章 URL / 本地 HTML、Markdown 路径', 'Any security article URL / local HTML or Markdown path')" @input="emit('update:sourceInput', ($event.target as HTMLInputElement).value)" @keyup.enter="emit('ingest-source', false)" />
      </div>
      <div class="trace-action-row">
        <button class="button secondary" :disabled="busy === 'source'" @click="emit('ingest-source', false)"><BookOpen :size="15" />{{ tr("缓存并提炼", "Cache & distill") }}</button>
        <button class="button primary" :disabled="loading" @click="emit('refresh')"><RefreshCw :size="15" :class="{ spinning: loading }" />{{ tr("重新分析", "Refresh") }}</button>
        <details class="trace-advanced-tools">
          <summary><Wrench :size="14" />{{ tr("更多工具", "More tools") }}</summary>
          <div class="trace-advanced-grid">
            <label>
              <span>{{ tr("聚合类型", "Aggregate type") }}</span>
              <select :value="aggregateType" @change="emit('update:aggregateType', ($event.target as HTMLSelectElement).value)">
                <option value="web">Web URL</option>
                <option value="code">{{ tr("代码审计", "Code") }}</option>
                <option value="greybox">{{ tr("灰盒联测", "Grey-box") }}</option>
                <option value="cicd">CI/CD</option>
              </select>
            </label>
            <button class="button secondary" :disabled="busy === 'aggregate'" @click="emit('aggregate')"><Layers :size="15" />{{ tr("聚合同类轨迹", "Aggregate traces") }}</button>
            <button class="button ghost" :disabled="busy === 'source'" @click="emit('ingest-source', true)">{{ tr("强制刷新来源", "Force refresh source") }}</button>
            <button class="button ghost" @click="emit('import-knowledge')"><Upload :size="15" />{{ tr("导入知识", "Import knowledge") }}</button>
            <button class="button ghost" :disabled="!knowledgeCount" @click="emit('export-knowledge')"><Download :size="15" />{{ tr("导出知识", "Export knowledge") }}</button>
          </div>
        </details>
      </div>
    </div>
  </div>
</template>
