<script setup lang="ts">
import { ClipboardCopy, FileJson } from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { SentinelFinding } from "../../types";
import { formatNumber, json, scriptTone } from "../../presentation";

const { tr } = useI18n();

defineProps<{
  jsRows: SentinelFinding[];
  copyText: (value: string) => Promise<void>;
}>();
</script>

<template>
  <section class="result-block kind-js-file">
    <div class="block-title">
      <FileJson :size="16" />
      <div>
        <strong>{{ tr('JS 文件', 'JS files') }}</strong
        ><small
          >{{ tr('业务包深度分析；runtime 只发现分包；vendor 与公共依赖不进入原生 Agent。', 'Business bundles get deep analysis; runtime bundles are only used to discover chunks; vendor and shared dependencies never reach the native Agent.') }}</small
        >
      </div>
    </div>
    <div class="js-file-list">
      <article
        v-for="item in jsRows"
        :key="item.id"
        :class="scriptTone(json(item.recordJson).type)"
      >
        <header>
          <span>{{ json(item.recordJson).type || "script" }}</span
          ><b>{{
            json(item.recordJson).statusCode ||
            json(item.recordJson).priority ||
            "info"
          }}</b>
        </header>
        <div class="long-value-cell">
          <code
            class="scroll-value"
            :title="json(item.recordJson).url"
            >{{ json(item.recordJson).url }}</code
          ><button
            class="icon-button compact"
            :title="tr('复制 JS 地址', 'Copy JS URL')"
            @click="copyText(json(item.recordJson).url)"
          >
            <ClipboardCopy :size="13" />
          </button>
        </div>
        <p>
          {{ formatNumber(json(item.recordJson).size || 0) }}
          bytes ·
          {{ json(item.recordJson).isMinified ? tr("已压缩", "minified") : tr("未压缩", "not minified") }}
          <template v-if="json(item.recordJson).discoveredFrom">
            {{
              tr(
                `· 来源 ${json(item.recordJson).discoveredFrom === "html" ? "HTML" : json(item.recordJson).discoveredFrom}`,
                `· Source ${json(item.recordJson).discoveredFrom === "html" ? "HTML" : json(item.recordJson).discoveredFrom}`
              )
            }}
          </template>
        </p>
        <div v-if="json(item.recordJson).analysis" class="js-analysis-tags">
          <span :class="{ active: json(item.recordJson).analysis.sourceMapReference }"
            >Source Map
            {{ json(item.recordJson).analysis.sourceMapReference ? tr("存在", "present") : tr("未发现", "not found") }}</span
          ><span :class="{ active: json(item.recordJson).analysis.module }"
            >ES Module {{ json(item.recordJson).analysis.module ? tr("是", "yes") : tr("否", "no") }}</span
          ><span v-if="json(item.recordJson).analysis.moduleCount" class="active"
            >{{ tr(`模块 ${json(item.recordJson).analysis.moduleCount}`, `Modules ${json(item.recordJson).analysis.moduleCount}`) }}</span
          ><span v-if="json(item.recordJson).analysis.businessScore" class="active"
            >{{ tr(`业务信号 ${json(item.recordJson).analysis.businessScore}`, `Business signal ${json(item.recordJson).analysis.businessScore}`) }}</span
          ><span>{{ json(item.recordJson).analysis.extractionEngine || "inventory" }}</span>
        </div>
        <p v-if="json(item.recordJson).error" class="form-error">
          {{ json(item.recordJson).error }}
        </p>
      </article>
      <div v-if="!jsRows.length" class="empty-inline">{{ tr('没有 JS 分析记录', 'No JS analysis records') }}</div>
    </div>
  </section>
</template>
