<script setup lang="ts">
import { Fingerprint, Layers3, Server, Shield } from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { SentinelFinding } from "../../types";
import {
  displayName,
  displayVersion,
  json,
  kindLabel,
  text,
} from "../../presentation";

const { tr } = useI18n();

defineProps<{
    fingerprintCards: Array<{ key: string; label: string; data: Record<string, any> }>;
  securityHeaders: Array<{ item: SentinelFinding; data: Record<string, any> }>;
  techStack: Record<string, any>;
  wordpress: Record<string, any>;
  identifiedProducts: Array<{ name: string; productId: string; stack?: string }>;
  rows: (...kinds: string[]) => SentinelFinding[];
}>();
</script>

<template>
  <div class="result-section-stack">
    <section class="result-block kind-fingerprint">
      <div class="block-title">
        <Server :size="16" />
        <div>
          <strong>{{ tr('技术栈详情', 'Tech stack details') }}</strong
          ><small
            >{{ tr('名称和证据已规范化；“未识别”表示没有足够证据，不等于不存在。', 'Names and evidence are normalized. "Not identified" means not enough evidence, not that it is absent.') }}</small
          >
        </div>
      </div>
      <div v-if="identifiedProducts && identifiedProducts.length" class="identified-products">
        <span class="identified-label">{{ tr('已识别系统', 'Identified system') }}</span>
        <span
          v-for="product in identifiedProducts"
          :key="product.productId || product.name"
          class="identified-chip"
          >{{ product.name }}<em v-if="product.stack">{{ product.stack }}</em></span
        >
      </div>
      <div class="fingerprint-detail-list">
        <article
          v-for="card in fingerprintCards"
          :key="card.key"
          :class="`tone-${card.key}`"
        >
          <header>
            <span>{{ card.label }}</span
            ><strong
              >{{ displayName(card.data) }}
              <small>{{ displayVersion(card.data) }}</small></strong
            ><em>{{ card.data?.confidence || "unknown" }}</em>
          </header>
          <div
            v-if="card.data?.libraries?.length"
            class="fingerprint-tags"
          >
            <span
              v-for="library in card.data.libraries"
              :key="library.name"
              >{{ library.name }} {{ library.version || "" }}</span
            >
          </div>
          <div
            v-if="card.data?.buildTools?.length"
            class="fingerprint-tags"
          >
            <span
              v-for="tool in card.data.buildTools"
              :key="tool"
              >{{ tool }}</span
            >
          </div>
          <p v-if="card.data?.evidence?.length">
            {{ tr(`识别依据：${card.data.evidence.join("；")}`, `Evidence: ${card.data.evidence.join("; ")}`) }}
          </p>
        </article>
        <article v-if="techStack.baseUrls?.length" class="tone-api">
          <header>
            <span>{{ tr('API 基础地址', 'API base URLs') }}</span
            ><strong>{{ tr(`${techStack.baseUrls.length} 个`, `${techStack.baseUrls.length}`) }}</strong>
          </header>
          <div class="fingerprint-tags">
            <span v-for="url in techStack.baseUrls" :key="url">{{
              url
            }}</span>
          </div>
        </article>
      </div>
    </section>
    <section
      v-if="Object.keys(wordpress).length"
      class="result-block"
    >
      <div class="block-title">
        <Fingerprint :size="16" />
        <div>
          <strong>WordPress</strong
          ><small>{{ tr('版本、插件、主题与入口', 'Version, plugins, themes and entry points') }}</small>
        </div>
      </div>
      <div class="wordpress-summary">
        <article>
          <span>{{ tr('核心版本', 'Core version') }}</span
          ><strong>{{ text(wordpress.version) }}</strong>
        </article>
        <article>
          <span>{{ tr('主题', 'Theme') }}</span
          ><strong
            >{{ text(wordpress.theme?.name) }}
            {{ text(wordpress.theme?.version) }}</strong
          >
        </article>
        <article>
          <span>{{ tr('插件', 'Plugins') }}</span
          ><strong>{{ wordpress.plugins?.length || 0 }}</strong>
        </article>
        <article>
          <span>REST / XML-RPC</span
          ><strong
            >{{
              wordpress.restApiEnabled ? tr("REST 开启", "REST on") : tr("REST 未知", "REST unknown")
            }}
            ·
            {{
              wordpress.xmlrpcEnabled
                ? tr("XML-RPC 开启", "XML-RPC on")
                : tr("XML-RPC 关闭", "XML-RPC off")
            }}</strong
          >
        </article>
      </div>
      <div class="plugin-list">
        <span
          v-for="plugin in wordpress.plugins || []"
          :key="plugin.name"
          ><b>{{ plugin.name }}</b
          >{{ plugin.version || tr("未知版本", "Unknown version") }}</span
        >
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Shield :size="16" />
        <div>
          <strong>{{ tr('安全响应头', 'Security headers') }}</strong
          ><small>{{ tr('橙色表示配置缺失，不直接判定为漏洞。', 'Orange means the header is missing; that alone is not a vulnerability.') }}</small>
        </div>
      </div>
      <div class="security-header-table">
        <div class="table-head">
          <span>{{ tr('响应头', 'Header') }}</span><span>{{ tr('状态', 'Status') }}</span><span>{{ tr('当前值', 'Current value') }}</span
          ><span>{{ tr('修复建议', 'Remediation') }}</span>
        </div>
        <div v-for="row in securityHeaders" :key="row.item.id">
          <strong>{{ row.item.title }}</strong
          ><span
            :class="
              row.data.present ? 'config-ok' : 'config-missing'
            "
            >{{ row.data.present ? tr("已配置", "Set") : tr("缺失", "Missing") }}</span
          ><code>{{ row.data.value || "—" }}</code>
          <p>{{ row.data.recommendation || "—" }}</p>
        </div>
        <div v-if="!securityHeaders.length" class="empty-inline">
          {{ tr('没有安全头数据', 'No security header data') }}
        </div>
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Layers3 :size="16" />
        <div><strong>{{ tr('Cookie / 外部服务 / 信息披露', 'Cookies / external services / information disclosure') }}</strong></div>
      </div>
      <div class="compact-record-grid">
        <article
          v-for="item in rows(
            'cookie',
            'external_service',
            'info_disclosure',
            'open_port',
          )"
          :key="item.id"
        >
          <span>{{ kindLabel(item.kind) }}</span
          ><strong>{{ item.title || item.recordKey }}</strong>
          <pre>{{
            JSON.stringify(json(item.recordJson), null, 2)
          }}</pre>
        </article>
        <div
          v-if="
            !rows(
              'cookie',
              'external_service',
              'info_disclosure',
              'open_port',
            ).length
          "
          class="empty-inline"
        >
          {{ tr('暂无记录', 'No records yet') }}
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.identified-products {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin: 10px 0 2px;
}
.identified-label {
  font-size: 12px;
  opacity: 0.7;
}
.identified-chip {
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
  padding: 4px 10px;
  border-radius: 999px;
  font-weight: 600;
  font-size: 13px;
  background: rgba(99, 102, 241, 0.16);
  color: var(--accent, #4f46e5);
}
.identified-chip em {
  font-style: normal;
  font-weight: 500;
  font-size: 12px;
  opacity: 0.75;
}
</style>
