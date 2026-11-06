<script setup lang="ts">
import {
  Archive,
  ClipboardCheck,
  ClipboardCopy,
  Eye,
  RefreshCw,
  Save,
  ShieldAlert,
} from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { SentinelFuseEntry } from "../types";
import InlineConfirm from "../../../shared/components/InlineConfirm.vue";
import {
  createSentinelLabels,
  fuseVerdictLabel,
  json,
  kindLabel,
  kindTone,
} from "../presentation";

const { tr } = useI18n();
const { severityLabel, statusLabel, verdictLabel } =
  createSentinelLabels(tr);

const fuseFilter = defineModel<string>("fuseFilter", { required: true });
const fuseCategoryFilter = defineModel<string>("fuseCategoryFilter", {
  required: true,
});
const fuseEditor = defineModel<SentinelFuseEntry>("fuseEditor");
const pendingFuseRemoval = defineModel<SentinelFuseEntry>("pendingFuseRemoval");

defineProps<{
  fuseEntries: SentinelFuseEntry[];
  visibleFuseEntries: SentinelFuseEntry[];
  fuseBusy: boolean;
  fuseForm: Record<string, any>;
  fuseDetailTabs: Array<[string, string]>;
  copyText: (...args: any[]) => any;
  editFuse: (...args: any[]) => any;
  fuseCategoryLabel: (...args: any[]) => any;
  fuseReasonCategory: (...args: any[]) => any;
  fuseReasonParts: (...args: any[]) => any;
  fuseRecommendedAction: (...args: any[]) => any;
  fuseRows: (...args: any[]) => any;
  fuseState: (...args: any[]) => any;
  fuseTarget: (...args: any[]) => any;
  fuseValidationRows: (...args: any[]) => any;
  removeFuse: (...args: any[]) => any;
  sameTargetUrl: (...args: any[]) => any;
  saveFuse: (...args: any[]) => any;
  toggleFuseDetail: (...args: any[]) => any;
}>();
</script>

<template>
  <section class="panel fuse-zone-panel">
    <div class="panel-heading">
      <div>
        <h3>{{ tr('停止与 URL 处置队列', 'Stopped URLs and dispositions') }}</h3>
        <p>
          {{ tr('这里处理“为什么停止、是否恢复”，不是判断漏洞真假。原生 Agent 遇到拦截、成本失控或无进展时只停止当前 URL，其余队列继续执行。', 'Decide here why a URL stopped and whether to resume it, not whether a vulnerability is real. When the native Agent is blocked, overruns cost or stops making progress, it stops only the current URL and the rest of the queue keeps running.') }}
        </p>
      </div>
      <div class="fuse-filters">
        <select v-model="fuseCategoryFilter" class="toolbar-select"><option value="all">{{ tr('全部停止原因', 'All stop reasons') }}</option><option value="budget">{{ tr('成本 / 无进展', 'Cost / no progress') }}</option><option value="access">{{ tr('缺少访问条件', 'Missing access') }}</option><option value="blocked">{{ tr('遭到拦截', 'Blocked') }}</option><option value="failure">{{ tr('网络 / 执行异常', 'Network / run error') }}</option><option value="low_value">{{ tr('价值不足', 'Low value') }}</option></select>
        <select v-model="fuseFilter" class="toolbar-select"><option value="active">{{ tr('待处置', 'Pending') }}</option><option value="archived">{{ tr('已归档', 'Archived') }}</option><option value="all">{{ tr('全部', 'All') }}</option></select>
      </div>
    </div>
    <div class="fuse-zone-stats">
      <article class="attention">
        <span>{{ tr('待决定是否恢复', 'Awaiting resume decision') }}</span
        ><strong>{{
          fuseEntries.filter((item) => !item.archived && item.verdict === 'pending').length
        }}</strong>
      </article>
      <article>
        <span>{{ tr('补充条件后重试', 'Retry after adding conditions') }}</span
        ><strong>{{ fuseEntries.filter((item) => !item.archived && item.verdict === 'needs_followup').length }}</strong>
      </article>
      <article>
        <span>{{ tr('已完成归档', 'Archived') }}</span
        ><strong>{{
          fuseEntries.filter((item) => item.archived).length
        }}</strong>
      </article>
      <article>
        <span>{{ tr('当前显示', 'Showing') }}</span
        ><strong>{{ visibleFuseEntries.length }}</strong>
      </article>
    </div>
    <div class="fuse-entry-list">
      <article
        v-for="item in visibleFuseEntries"
        :key="item.id"
        :class="{ archived: item.archived, attention: !item.archived && item.verdict === 'pending' }"
      >
        <header>
          <span class="fuse-icon"><ShieldAlert :size="16" /></span>
          <div>
            <strong>{{ item.company || tr("未提供公司", "No company provided") }}</strong>
            <div class="long-value-cell">
              <code :title="item.url">{{ item.url }}</code
              ><button
                class="icon-button compact"
                :title="tr('复制 URL', 'Copy URL')"
                @click="copyText(item.url)"
              >
                <ClipboardCopy :size="13" />
              </button>
            </div>
          </div>
          <span :class="`fuse-verdict ${item.verdict}`">{{
            fuseVerdictLabel(item.verdict)
          }}</span>
        </header>
        <div class="fuse-disposition-callout">
          <span :class="`fuse-category category-${fuseReasonCategory(item)}`">{{ fuseCategoryLabel(fuseReasonCategory(item)) }}</span>
          <p><b>{{ tr('建议动作', 'Recommended action') }}</b>{{ fuseRecommendedAction(item) }}</p>
        </div>
        <dl>
          <div class="fuse-reason-column">
            <dt>{{ tr('停止原因', 'Stop reason') }}</dt>
            <dd>
              <div class="fuse-reason-parts">
                <span
                  v-for="part in fuseReasonParts(item)"
                  :key="`${part.label}-${part.text}`"
                  :class="`reason-${part.tone}`"
                  ><b>{{ part.label }}</b
                  >{{ part.text }}</span
                >
              </div>
            </dd>
          </div>
          <div>
            <dt>{{ tr('来源任务', 'Source task') }}</dt>
            <dd>
              <code>{{ item.sourceScanId }}</code>
            </dd>
          </div>
          <div>
            <dt>{{ tr('更新时间', 'Updated') }}</dt>
            <dd>{{ item.updatedAt }}</dd>
          </div>
        </dl>
        <div v-if="item.note || item.evidence" class="fuse-review-summary">
          <p>{{ item.note || tr("无备注", "No note") }}</p>
          <pre>{{ item.evidence || tr("无证据记录", "No evidence recorded") }}</pre>
        </div>
        <footer>
          <button
            class="button secondary compact"
            @click="toggleFuseDetail(item)"
          >
            <Eye :size="13" />{{
              fuseState(item).open ? tr("收起完整情报", "Hide full intel") : tr("查看完整情报", "View full intel")
            }}</button
          ><button class="button primary compact" @click="editFuse(item)">
            <ClipboardCheck :size="13" />{{
              item.verdict === "pending" ? tr("记录处置决定", "Record decision") : tr("编辑处置记录", "Edit disposition")
            }}</button
          ><button
            v-if="!item.archived && item.verdict !== 'pending'"
            class="button ghost compact"
            @click="
              editFuse(item);
              saveFuse(true);
            "
          >
            <Archive :size="13" />{{ tr('完成并归档', 'Complete and archive') }}</button
          ><button
            class="button warning compact"
            @click="pendingFuseRemoval = item"
          >
            <RefreshCw :size="13" />{{ tr('恢复并自动重试', 'Resume and auto-retry') }}
          </button>
        </footer>
        <section v-if="fuseState(item).open" class="fuse-intel-panel">
          <nav class="fuse-intel-tabs" :aria-label="tr('已停止目标情报分类', 'Stopped target intel categories')">
            <button
              v-for="entry in fuseDetailTabs"
              :key="entry[0]"
              :class="{ active: fuseState(item).tab === entry[0] }"
              @click="fuseState(item).tab = entry[0]"
            >
              {{ entry[1] }}
            </button>
          </nav>
          <div v-if="fuseState(item).loading" class="empty-inline">
            {{ tr('正在加载该 URL 的完整情报…', 'Loading full intel for this URL…') }}
          </div>
          <template v-else>
            <div
              v-if="fuseState(item).tab === 'summary'"
              class="fuse-intel-summary"
            >
              <dl>
                <div>
                  <dt>URL</dt>
                  <dd class="scroll-value">{{ item.url }}</dd>
                </div>
                <div>
                  <dt>{{ tr('执行状态', 'Run status') }}</dt>
                  <dd>
                    {{ statusLabel(fuseTarget(item)?.status || "limited") }}
                  </dd>
                </div>
                <div>
                  <dt>{{ tr('前端价值', 'Frontend value') }}</dt>
                  <dd>{{ fuseTarget(item)?.valueScore ?? "—" }} / 100</dd>
                </div>
                <div>
                  <dt>{{ tr('扫描模式', 'Scan mode') }}</dt>
                  <dd>
                    {{ (fuseTarget(item)?.scanMode || "—").toUpperCase() }}
                  </dd>
                </div>
                <div class="fuse-summary-reason">
                  <dt>{{ tr('停止原因', 'Stop reason') }}</dt>
                  <dd>
                    <div class="fuse-reason-parts compact">
                      <span
                        v-for="part in fuseReasonParts(item)"
                        :key="`summary-${part.label}-${part.text}`"
                        :class="`reason-${part.tone}`"
                        ><b>{{ part.label }}</b
                        >{{ part.text }}</span
                      >
                    </div>
                  </dd>
                </div>
                <div>
                  <dt>{{ tr('记录总数', 'Total records') }}</dt>
                  <dd>
                    {{
                      fuseState(item).findings.filter((row: any) =>
                        sameTargetUrl(row.targetUrl, item.url),
                      ).length
                    }}
                  </dd>
                </div>
              </dl>
              <div
                v-for="row in fuseRows(
                  item,
                  'summary_target',
                  'risk_summary',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <strong>{{ row.title || kindLabel(row.kind) }}</strong>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
            </div>
            <div
              v-else-if="fuseState(item).tab === 'fingerprint'"
              class="fuse-intel-records"
            >
              <div
                v-for="row in fuseRows(
                  item,
                  'fingerprint',
                  'tech_stack',
                  'security_header',
                  'cookie',
                  'external_service',
                  'info_disclosure',
                  'open_port',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <header>
                  <span>{{ kindLabel(row.kind) }}</span
                  ><strong>{{ row.title || row.recordKey }}</strong>
                </header>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
              <div
                v-if="
                  !fuseRows(
                    item,
                    'fingerprint',
                    'tech_stack',
                    'security_header',
                    'cookie',
                    'external_service',
                    'info_disclosure',
                    'open_port',
                  ).length
                "
                class="empty-inline"
              >
                {{ tr('没有指纹配置记录', 'No fingerprint records') }}
              </div>
            </div>
            <div
              v-else-if="fuseState(item).tab === 'assets'"
              class="fuse-intel-records"
            >
              <div
                v-for="row in fuseRows(
                  item,
                  'js_file',
                  'api',
                  'route',
                  'runtime_signal',
                  'crypto_signal',
                  'sensitive_info',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <header>
                  <span>{{ kindLabel(row.kind) }}</span
                  ><strong class="scroll-value">{{
                    row.title || row.recordKey
                  }}</strong>
                </header>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
              <div
                v-if="
                  !fuseRows(
                    item,
                    'js_file',
                    'api',
                    'route',
                    'runtime_signal',
                    'crypto_signal',
                    'sensitive_info',
                  ).length
                "
                class="empty-inline"
              >
                {{ tr('没有 JS / API 情报', 'No JS / API intel') }}
              </div>
            </div>
            <div
              v-else-if="fuseState(item).tab === 'endpoints'"
              class="fuse-intel-records"
            >
              <div
                v-for="row in fuseRows(
                  item,
                  'endpoint',
                  'endpoint_expanded',
                  'directory_find',
                  'rest_endpoint',
                  'login_endpoint',
                  'parameter_json',
                  'parameter_xml',
                  'parameter_form',
                  'parameter_upload',
                  'parameter_path',
                  'parameter_query',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <header>
                  <span>{{ kindLabel(row.kind) }}</span
                  ><strong class="scroll-value">{{
                    row.title || row.recordKey
                  }}</strong>
                </header>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
              <div
                v-if="
                  !fuseRows(
                    item,
                    'endpoint',
                    'endpoint_expanded',
                    'directory_find',
                    'rest_endpoint',
                    'login_endpoint',
                    'parameter_json',
                    'parameter_xml',
                    'parameter_form',
                    'parameter_upload',
                    'parameter_path',
                    'parameter_query',
                  ).length
                "
                class="empty-inline"
              >
                {{ tr('没有端点验证记录', 'No endpoint validation records') }}
              </div>
            </div>
            <div v-else class="fuse-intel-records">
              <div
                v-for="row in fuseRows(
                  item,
                  'vulnerability',
                  'poc_test',
                  'risk_summary',
                )"
                :key="row.id"
                class="fuse-intel-record"
                :class="kindTone(row.kind)"
              >
                <header>
                  <span
                    >{{ kindLabel(row.kind) }} ·
                    {{ severityLabel(row.severity) }}</span
                  ><strong>{{ row.title || row.recordKey }}</strong>
                </header>
                <pre>{{
                  JSON.stringify(json(row.recordJson), null, 2)
                }}</pre>
              </div>
              <div
                v-for="row in fuseValidationRows(item)"
                :key="`validation-${row.id}`"
                class="fuse-intel-record validation"
              >
                <header>
                  <span>{{ tr(`人工验证 · ${verdictLabel(row.verdict)}`, `Manual validation · ${verdictLabel(row.verdict)}`) }}</span
                  ><strong>{{ row.findingKind }}</strong>
                </header>
                <p>{{ row.note || tr("无备注", "No note") }}</p>
                <pre>{{ row.evidence || tr("无证据记录", "No evidence recorded") }}</pre>
              </div>
              <div
                v-if="
                  !fuseRows(
                    item,
                    'vulnerability',
                    'poc_test',
                    'risk_summary',
                  ).length && !fuseValidationRows(item).length
                "
                class="empty-inline"
              >
                {{ tr('没有漏洞或证明记录', 'No vulnerability or proof records') }}
              </div>
            </div>
          </template>
        </section>
        <div v-if="fuseEditor?.id === item.id" class="fuse-review-editor">
          <label class="field"
            ><span>{{ tr('URL 处置状态', 'URL disposition') }}</span
            ><select v-model="fuseForm.verdict">
              <option value="pending">{{ tr('暂不决定', 'Decide later') }}</option>
              <option value="manual_verified">{{ tr('已人工接管', 'Taken over manually') }}</option>
              <option value="needs_followup">{{ tr('补充条件后重试', 'Retry after adding conditions') }}</option>
              <option value="not_reproducible">{{ tr('保持排除', 'Keep excluded') }}</option>
            </select></label
          >
          <label class="field"
            ><span>{{ tr('处置说明', 'Disposition note') }}</span
            ><textarea
              v-model="fuseForm.note"
              rows="3"
              :placeholder="tr('为什么停止、缺少什么条件、是否值得恢复', 'Why it stopped, what is missing, whether it is worth resuming')"
            ></textarea>
          </label>
          <label class="field span-two"
            ><span>{{ tr('补充上下文 / 请求响应 / 证据路径', 'Extra context / requests and responses / evidence paths') }}</span
            ><textarea
              v-model="fuseForm.evidence"
              rows="5"
              :placeholder="tr('记录关键请求响应或本地证据文件', 'Record key requests and responses or local evidence files')"
            ></textarea>
          </label>
          <footer>
            <button class="button ghost" @click="fuseEditor = undefined">
              {{ tr('取消', 'Cancel') }}</button
            ><button
              class="button primary"
              :disabled="fuseBusy"
              @click="saveFuse(false)"
            >
              <Save :size="14" />{{ tr('保存处置', 'Save disposition') }}</button
            ><button
              class="button secondary"
              :disabled="fuseBusy"
              @click="saveFuse(true)"
            >
              <Archive :size="14" />{{ tr('完成并归档', 'Complete and archive') }}
            </button>
          </footer>
        </div>
        <InlineConfirm
          v-if="pendingFuseRemoval?.id === item.id"
          :title="tr('恢复该 URL 并立即重试？', 'Resume this URL and retry now?')"
          :detail="tr('会在原任务工作流中创建只包含该 URL 的续跑执行，复用已保存的前端证据；历史记录与累计成本都会保留。', 'Creates a resume run with only this URL in the original task workflow, reusing the saved frontend evidence. History and accumulated cost are kept.')"
          :busy="fuseBusy"
          @cancel="pendingFuseRemoval = undefined"
          @confirm="removeFuse"
        />
      </article>
      <div v-if="!visibleFuseEntries.length" class="empty-state">
        {{ tr('当前没有匹配的已停止目标。', 'No matching stopped targets.') }}
      </div>
    </div>
  </section>
</template>
