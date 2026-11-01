<script setup lang="ts">
import { Network } from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { SentinelFinding } from "../../types";
import { endpointUrl, json, kindLabel, statusTone } from "../../presentation";

const { tr } = useI18n();

defineProps<{
  endpointRows: SentinelFinding[];
  selectedUrl: string;
}>();
</script>

<template>
  <div class="result-section-stack">
    <section class="result-block">
      <div class="block-title">
        <Network :size="16" />
        <div>
          <strong>{{ tr('已验证端点', 'Verified endpoints') }}</strong
          ><small>{{ tr('状态码颜色只表示 HTTP 响应状态。', 'Status code colors reflect only the HTTP response status.') }}</small>
        </div>
      </div>
      <div class="endpoint-table">
        <div class="table-head">
          <span>{{ tr('状态', 'Status') }}</span><span>{{ tr('方法', 'Method') }}</span><span>{{ tr('完整 URL', 'Full URL') }}</span
          ><span>{{ tr('来源', 'Source') }}</span><span>{{ tr('耗时 / 大小', 'Time / size') }}</span
          ><span>{{ tr('说明', 'Notes') }}</span>
        </div>
        <div v-for="item in endpointRows" :key="item.id">
          <b
            :class="`http-status ${statusTone(json(item.recordJson).statusCode)}`"
            >{{ json(item.recordJson).statusCode || "—" }}</b
          ><span class="method-badge">{{
            json(item.recordJson).method || "GET"
          }}</span
          ><code>{{
            endpointUrl(
              selectedUrl,
              json(item.recordJson).url ||
                json(item.recordJson).path ||
                "/",
            )
          }}</code
          ><span>{{
            json(item.recordJson).source || kindLabel(item.kind)
          }}</span
          ><span
            >{{ json(item.recordJson).responseTime || "—" }} ms ·
            {{ json(item.recordJson).bodyLength || "—" }} B</span
          >
          <p>
            {{
              json(item.recordJson).note ||
              json(item.recordJson).detail ||
              json(item.recordJson).bodySnippet ||
              "—"
            }}
          </p>
        </div>
        <div v-if="!endpointRows.length" class="empty-inline">
          {{ tr('没有端点验证数据', 'No endpoint validation data') }}
        </div>
      </div>
    </section>
  </div>
</template>
