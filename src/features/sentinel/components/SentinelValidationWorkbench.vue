<script setup lang="ts">
import { Eye, Save } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { SentinelValidationWorkItem } from "../types";
import {
  createSentinelLabels,
  json,
  safeSeverity,
  text,
} from "../presentation";

defineProps<{
  filter: string;
  stats: { pending: number; confirmed: number; rejected: number };
  items: SentinelValidationWorkItem[];
  editor?: SentinelValidationWorkItem;
  form: { verdict: string; severity: string; note: string; evidence: string };
}>();
const emit = defineEmits<{
  "update:filter": [value: string];
  select: [item: SentinelValidationWorkItem];
  evidence: [item: SentinelValidationWorkItem];
  save: [];
}>();
const { tr } = useI18n();
const { severityLabel, verdictLabel } = createSentinelLabels(tr);
</script>

<template>
  <section class="panel sentinel-panel validation-workbench-panel">
    <div class="panel-heading">
      <div>
        <h3>{{ tr('漏洞验证工作台', 'Vulnerability validation workbench') }}</h3>
        <p>{{ tr('把扫描发现和人工结论放在同一条队列；未验证、需补证和已定性的漏洞都不会再丢在结果页里。', 'Scan findings and manual verdicts share one queue, so unvalidated, under-evidenced, and classified vulnerabilities no longer get lost on the results page.') }}</p>
      </div>
      <select :value="filter" class="toolbar-select" @change="emit('update:filter', ($event.target as HTMLSelectElement).value)">
        <option value="pending">{{ tr('待处理与需补证', 'Pending and needs evidence') }}</option>
        <option value="true_positive">{{ tr('真实漏洞', 'True positives') }}</option>
        <option value="false_positive">{{ tr('误报', 'False positives') }}</option>
        <option value="all">{{ tr('全部漏洞', 'All vulnerabilities') }}</option>
      </select>
    </div>
    <div class="validation-work-stats">
      <article class="attention"><span>{{ tr('待处理 / 补证', 'Pending / needs evidence') }}</span><strong>{{ stats.pending }}</strong></article>
      <article><span>{{ tr('确认真实漏洞', 'Confirmed true positives') }}</span><strong>{{ stats.confirmed }}</strong></article>
      <article><span>{{ tr('确认误报', 'Confirmed false positives') }}</span><strong>{{ stats.rejected }}</strong></article>
      <article><span>{{ tr('当前显示', 'Showing') }}</span><strong>{{ items.length }}</strong></article>
    </div>
    <div class="validation-workbench-layout">
      <div class="validation-work-queue">
        <button
          v-for="item in items"
          :key="item.findingId"
          :class="{ active: editor?.findingId === item.findingId }"
          @click="emit('select', item)"
        >
          <span :class="`severity-badge ${safeSeverity(item.confirmedSeverity || item.originalSeverity)}`">{{ severityLabel(item.confirmedSeverity || item.originalSeverity) }}</span>
          <div>
            <strong>{{ item.title || tr("未命名漏洞", "Untitled vulnerability") }}</strong>
            <code>{{ item.url || tr("全局 / 源码发现", "Global / source finding") }}</code>
            <small>{{ item.projectName }} · {{ item.taskName || item.scanId }} · {{ item.updatedAt }}</small>
          </div>
          <span :class="`validation-chip ${item.validationId ? item.verdict : 'pending'}`">{{ item.validationId ? verdictLabel(item.verdict) : tr("待验证", "Pending validation") }}</span>
        </button>
        <div v-if="!items.length" class="empty-state">
          {{ tr('当前筛选下没有待处理漏洞；切换到“全部漏洞”可查看历史结论。', 'No pending vulnerabilities for this filter. Switch to “All vulnerabilities” to see past verdicts.') }}
        </div>
      </div>
      <aside class="validation-work-detail">
        <template v-if="editor">
          <header>
            <div><h3>{{ editor.title || tr("未命名漏洞", "Untitled vulnerability") }}</h3></div>
            <button class="button ghost compact" @click="emit('evidence', editor)"><Eye :size="13" />{{ tr('打开完整证据', 'Open full evidence') }}</button>
          </header>
          <dl class="validation-context-grid">
            <div><dt>{{ tr('项目 / 任务', 'Project / task') }}</dt><dd>{{ editor.projectName }} · {{ editor.taskName || editor.scanId }}</dd></div>
            <div><dt>{{ tr('目标', 'Target') }}</dt><dd><code>{{ editor.url || tr("全局 / 源码发现", "Global / source finding") }}</code></dd></div>
            <div><dt>{{ tr('扫描严重度', 'Scan severity') }}</dt><dd>{{ severityLabel(editor.originalSeverity) }}</dd></div>
            <div><dt>Finding Key</dt><dd><code>{{ editor.findingKey }}</code></dd></div>
          </dl>
          <div class="validation-evidence-preview">
            <article><span>{{ tr('漏洞描述', 'Description') }}</span><p>{{ json(editor.recordJson).description || json(editor.recordJson).detail || tr("扫描结果未提供结构化描述", "The scan result has no structured description") }}</p></article>
            <article><span>{{ tr('关键证据 / 复现', 'Key evidence / reproduction') }}</span><pre>{{ text(json(editor.recordJson).evidence || json(editor.recordJson).pocRequest || json(editor.recordJson).poc_description || tr("尚无证据摘要，请打开完整证据查看上下文", "No evidence summary yet; open the full evidence for context")) }}</pre></article>
          </div>
          <div class="verdict-picker validation-work-verdicts">
            <button v-for="choice in [{ v: 'true_positive', l: tr('确认真实漏洞', 'Confirm true positive') }, { v: 'false_positive', l: tr('确认误报', 'Confirm false positive') }, { v: 'needs_more', l: tr('需要补证', 'Needs more evidence') }]" :key="choice.v" :class="{ active: form.verdict === choice.v }" @click="form.verdict = choice.v">{{ choice.l }}</button>
          </div>
          <div class="validation-form-grid validation-work-form">
            <label class="field"><span>{{ tr('确认后严重度', 'Confirmed severity') }}</span><select v-model="form.severity"><option value="critical">{{ tr('严重', 'Critical') }}</option><option value="high">{{ tr('高危', 'High') }}</option><option value="medium">{{ tr('中危', 'Medium') }}</option><option value="low">{{ tr('低危', 'Low') }}</option><option value="info">{{ tr('信息', 'Info') }}</option></select></label>
            <label class="field"><span>{{ tr('结论与复现备注', 'Verdict and reproduction notes') }}</span><textarea v-model="form.note" rows="4" :placeholder="tr('写清复现步骤、判断理由、限制条件和下一步', 'Describe the reproduction steps, reasoning, limitations, and next steps')"></textarea></label>
            <label class="field span-two"><span>{{ tr('请求响应 / 截图 / 本地证据路径', 'Request and response / screenshots / local evidence paths') }}</span><textarea v-model="form.evidence" rows="6" :placeholder="tr('粘贴关键请求响应，或填写本地证据文件路径', 'Paste the key request and response, or enter local evidence file paths')"></textarea></label>
          </div>
          <footer><span>{{ tr('保存会立即更新风险统计，不会修改扫描原始证据。', 'Saving updates the risk stats immediately and does not change the original scan evidence.') }}</span><button class="button primary" @click="emit('save')"><Save :size="14" />{{ tr('保存漏洞结论', 'Save verdict') }}</button></footer>
        </template>
        <div v-else class="empty-state">{{ tr('从左侧选择一个漏洞开始验证。', 'Select a vulnerability on the left to start validating.') }}</div>
      </aside>
    </div>
  </section>
</template>
