<script setup lang="ts">
import { Activity, Bug, CheckCircle2, ClipboardCheck, Copy, Save, Search, X } from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { FindingExploit, SentinelFinding, SentinelValidation } from "../../types";
import {
  createSentinelLabels,
  endpointUrl,
  json,
  safeSeverity,
  text,
} from "../../presentation";

const { tr } = useI18n();
const { severityLabel, verdictLabel } = createSentinelLabels(tr);
const emit = defineEmits<{
  "select-finding": [id: number];
  "close-validation": [];
  "deep-dive": [findingId: number];
}>();
const props = defineProps<{
  vulnerabilityRows: SentinelFinding[];
  focusedVulnerabilityRows: SentinelFinding[];
  pocRows: SentinelFinding[];
  selectedFindingId?: number;
  selectedUrl: string;
  validationEditor?: SentinelFinding;
  validationForm: { verdict: string; severity: string; note: string; evidence: string };
  editValidation: (item: SentinelFinding, verdict?: string) => void;
  saveValidation: () => Promise<void>;
  validationFor: (item: SentinelFinding) => SentinelValidation | undefined;
  effectiveSeverity: (item: SentinelFinding) => string;
  vulnerabilityUpdateHistory: (item: SentinelFinding) => any[];
  findingExploits?: Record<number, FindingExploit>;
  deepDiveBusyId?: number;
  deepDiveDisabledReason?: string;
}>();
// Human labels for the deep-dive verdicts (mirrors the backend CHECK constraint).
const DEEP_STATUS_LABELS: Record<string, [string, string]> = {
  pending: ["待深挖", "Pending"],
  in_progress: ["深挖中", "In progress"],
  exploited: ["已利用", "Exploited"],
  not_exploitable: ["不可利用", "Not exploitable"],
  waf_blocked: ["被 WAF 拦截", "WAF blocked"],
  error: ["深挖出错", "Error"],
};
const deepStatusLabel = (status: string) => {
  const pair = DEEP_STATUS_LABELS[status];
  return pair ? tr(pair[0], pair[1]) : status;
};
const exploitFor = (id: number): FindingExploit | undefined => props.findingExploits?.[id];
async function copyText(value: string) {
  try { await navigator.clipboard?.writeText(value); } catch { /* clipboard may be unavailable; no-op */ }
}
</script>

<template>
  <div class="result-section-stack">
    <section class="result-block">
      <div class="block-title">
        <Bug :size="16" />
        <div>
          <strong>{{ tr('漏洞发现', 'Vulnerability findings') }}</strong
          ><small
            >{{ tr('历史与其他来源记录在此保留供核查；人工验证不等于原生 Reviewer 确认。主看板仅统计 Reviewer 确认。', 'History and records from other sources stay here for checking. Manual validation is not native Reviewer confirmation; the main board counts only Reviewer-confirmed findings.') }}</small
          >
        </div>
      </div>
      <div class="vulnerability-master-detail">
        <aside class="vulnerability-index-list">
          <button
            v-for="item in vulnerabilityRows"
            :key="`vuln-index-${item.id}`"
            :class="{ active: selectedFindingId === item.id }"
            @click="emit('select-finding', item.id)"
          >
            <span :class="`severity-badge ${effectiveSeverity(item)}`">{{ severityLabel(effectiveSeverity(item)) }}</span>
            <div><strong>{{ item.title || json(item.recordJson).title || item.recordKey }}</strong><small>{{ json(item.recordJson).method || "GET" }} {{ json(item.recordJson).url || "/" }}</small></div>
            <em class="validation-chip">{{ tr('原始记录 · 未经原生 Reviewer 审核', 'Raw record · not reviewed by the native Reviewer') }}</em>
            <em v-if="validationFor(item)" :class="`validation-chip ${validationFor(item)?.verdict}`">{{ verdictLabel(validationFor(item)?.verdict || "") }}</em>
          </button>
          <div v-if="!vulnerabilityRows.length" class="empty-inline">{{ tr('当前 URL 没有漏洞记录', 'No vulnerability records for this URL') }}</div>
        </aside>
      <div class="vulnerability-list focused">
        <article
          v-for="item in focusedVulnerabilityRows"
          :key="item.id"
          :class="`vuln-card severity-border-${effectiveSeverity(item)}`"
        >
          <header>
            <span
              :class="`severity-badge ${effectiveSeverity(item)}`"
              >{{ severityLabel(effectiveSeverity(item)) }}</span
            >
            <div>
              <strong>{{
                item.title ||
                json(item.recordJson).title ||
                item.recordKey
              }}</strong
              ><small
                >{{ tr(
                  `原始记录 · 未经原生 Reviewer 审核 · ${json(item.recordJson).type || "vulnerability"} · 原始等级 ${severityLabel(safeSeverity(item.severity))} · CVSS ${json(item.recordJson).cvss ?? "—"} · 置信度 ${json(item.recordJson).confidence || "未说明"} · ${json(item.recordJson).method || "GET"} ${json(item.recordJson).url || "/"}`,
                  `Raw record · not reviewed by the native Reviewer · ${json(item.recordJson).type || "vulnerability"} · Original severity ${severityLabel(safeSeverity(item.severity))} · CVSS ${json(item.recordJson).cvss ?? "—"} · Confidence ${json(item.recordJson).confidence || "not stated"} · ${json(item.recordJson).method || "GET"} ${json(item.recordJson).url || "/"}`
                ) }}</small
              ><small
                v-if="
                  json(item.recordJson).cve ||
                  json(item.recordJson).cwe
                "
                >{{ tr(
                  `${json(item.recordJson).cve || "无 CVE"} · ${json(item.recordJson).cwe || "无 CWE"} · 修复工作量 ${json(item.recordJson).fix_effort || "未知"}`,
                  `${json(item.recordJson).cve || "No CVE"} · ${json(item.recordJson).cwe || "No CWE"} · Fix effort ${json(item.recordJson).fix_effort || "unknown"}`
                ) }}</small
              >
            </div>
            <span
              v-if="validationFor(item)"
              :class="`validation-chip ${validationFor(item)?.verdict}`"
              ><CheckCircle2 :size="13" />{{
                verdictLabel(validationFor(item)?.verdict || "")
              }}
              · {{ severityLabel(effectiveSeverity(item)) }}</span
            >
          </header>
          <div class="vuln-columns">
            <div>
              <span>{{ tr('漏洞描述', 'Description') }}</span>
              <p>{{ json(item.recordJson).description || "—" }}</p>
            </div>
            <div>
              <span>{{ tr('技术分析', 'Technical analysis') }}</span>
              <p>
                {{
                  json(item.recordJson).technical_analysis || "—"
                }}
              </p>
            </div>
            <div>
              <span>{{ tr('证据', 'Evidence') }}</span>
              <pre>{{ text(json(item.recordJson).evidence) }}</pre>
            </div>
            <div>
              <span>{{ tr('影响', 'Impact') }}</span>
              <p>
                {{
                  json(item.recordJson).impact ||
                  json(item.recordJson).detail ||
                  "—"
                }}
              </p>
            </div>
            <div>
              <span>{{ tr('修复建议', 'Remediation') }}</span>
              <p>
                {{
                  json(item.recordJson).recommendation ||
                  json(item.recordJson).remediation_steps ||
                  "—"
                }}
              </p>
            </div>
            <div>
              <span>{{ tr('PoC / 复现', 'PoC / reproduction') }}</span>
              <pre>{{
                json(item.recordJson).pocRequest ||
                json(item.recordJson).poc_description ||
                "—"
              }}</pre>
            </div>
            <div
              v-if="
                json(item.recordJson).counterevidence ||
                json(item.recordJson).counterEvidence
              "
            >
              <span>{{ tr('反证检查', 'Counter-evidence check') }}</span>
              <p>{{
                json(item.recordJson).counterevidence ||
                json(item.recordJson).counterEvidence
              }}</p>
            </div>
            <div
              v-if="
                json(item.recordJson).confidence_rationale ||
                json(item.recordJson).confidenceRationale
              "
            >
              <span>{{ tr('置信度依据', 'Confidence rationale') }}</span>
              <p>{{
                json(item.recordJson).confidence_rationale ||
                json(item.recordJson).confidenceRationale
              }}</p>
            </div>
            <div
              v-if="
                json(item.recordJson).severity_change_conditions ||
                json(item.recordJson).severityChangeConditions
              "
            >
              <span>{{ tr('等级变化条件', 'Severity change conditions') }}</span>
              <p>{{
                json(item.recordJson).severity_change_conditions ||
                json(item.recordJson).severityChangeConditions
              }}</p>
            </div>
            <div
              v-if="
                json(item.recordJson).fix_verification ||
                json(item.recordJson).fixVerification
              "
            >
              <span>{{ tr('修复验证', 'Fix verification') }}</span>
              <p>{{
                json(item.recordJson).fix_verification ||
                json(item.recordJson).fixVerification
              }}</p>
            </div>
            <div v-if="json(item.recordJson).cvss_breakdown">
              <span>{{ tr('CVSS 明细', 'CVSS breakdown') }}</span>
              <pre>{{
                JSON.stringify(
                  json(item.recordJson).cvss_breakdown,
                  null,
                  2,
                )
              }}</pre>
            </div>
            <div v-if="json(item.recordJson).code_locations">
              <span>{{ tr('代码位置 / 修复差异', 'Code locations / fix diff') }}</span>
              <pre>{{
                JSON.stringify(
                  json(item.recordJson).code_locations,
                  null,
                  2,
                )
              }}</pre>
            </div>
            <div v-if="json(item.recordJson).assumptions">
              <span>{{ tr('前提与限制', 'Assumptions and limits') }}</span>
              <p>{{ json(item.recordJson).assumptions }}</p>
            </div>
            <div v-if="json(item.recordJson).dependency_metadata">
              <span>{{ tr('依赖信息', 'Dependency info') }}</span>
              <pre>{{
                JSON.stringify(
                  json(item.recordJson).dependency_metadata,
                  null,
                  2,
                )
              }}</pre>
            </div>
            <div
              v-if="vulnerabilityUpdateHistory(item).length"
              class="vulnerability-update-history"
            >
              <span>{{ tr('结论修订历史', 'Verdict revision history') }}</span>
              <ol>
                <li
                  v-for="(revision, index) in vulnerabilityUpdateHistory(item)"
                  :key="`${item.id}-revision-${index}`"
                >
                  <b>{{ revision.updated_at || revision.timestamp || tr(`修订 ${index + 1}`, `Revision ${index + 1}`) }}</b>
                  <p>{{ revision.update_reason || revision.reason || revision.summary || tr("结论已修订", "Verdict revised") }}</p>
                  <small v-if="revision.dropped_fields?.length">{{ tr(`替换字段：${revision.dropped_fields.join("、")}`, `Replaced fields: ${revision.dropped_fields.join(", ")}`) }}</small>
                </li>
              </ol>
            </div>
          </div>
          <footer>
            <button
              class="button primary compact"
              @click="editValidation(item)"
            >
              <ClipboardCheck :size="13" />{{
                validationFor(item)
                  ? tr("修改验证结论", "Edit verdict")
                  : tr("开始人工验证", "Start manual validation")
              }}</button
            ><span
              v-if="validationFor(item)"
              class="validation-saved-note"
              >{{ tr(`已保存：${verdictLabel(validationFor(item)?.verdict || "")} / ${severityLabel(effectiveSeverity(item))}`, `Saved: ${verdictLabel(validationFor(item)?.verdict || "")} / ${severityLabel(effectiveSeverity(item))}`) }}</span
            >
          </footer>
          <div
            v-if="validationEditor?.id === item.id"
            class="validation-editor inline-validation-editor"
          >
            <div class="block-title">
              <ClipboardCheck :size="16" />
              <div>
                <strong
                  >{{ tr(`人工验证：${item.title || item.recordKey}`, `Manual validation: ${item.title || item.recordKey}`) }}</strong
                ><small>{{ tr('保存后立即更新当前卡片与统计', 'Saving updates this card and the stats right away') }}</small>
              </div>
              <button
                class="icon-button"
                @click="emit('close-validation')"
              >
                <X :size="15" />
              </button>
            </div>
            <div class="verdict-picker">
              <button
                v-for="choice in [
                  { v: 'true_positive', l: tr('真实漏洞', 'True positive') },
                  { v: 'false_positive', l: tr('误报', 'False positive') },
                  { v: 'needs_more', l: tr('需要补证', 'Needs more evidence') },
                ]"
                :key="choice.v"
                :class="{
                  active: validationForm.verdict === choice.v,
                }"
                @click="validationForm.verdict = choice.v"
              >
                {{ choice.l }}
              </button>
            </div>
            <div class="validation-form-grid">
              <label class="field"
                ><span>{{ tr('确认后严重度', 'Confirmed severity') }}</span
                ><select v-model="validationForm.severity">
                  <option value="critical">{{ tr('严重', 'Critical') }}</option>
                  <option value="high">{{ tr('高危', 'High') }}</option>
                  <option value="medium">{{ tr('中危', 'Medium') }}</option>
                  <option value="low">{{ tr('低危', 'Low') }}</option>
                  <option value="info">{{ tr('信息', 'Info') }}</option>
                </select></label
              ><label class="field"
                ><span>{{ tr('验证备注', 'Validation note') }}</span
                ><textarea
                  v-model="validationForm.note"
                  rows="3"
                  :placeholder="tr('复现过程、判断理由、限制条件', 'Reproduction steps, reasoning, limitations')"
                ></textarea></label
              ><label class="field span-two"
                ><span>{{ tr('证据 / 请求响应 / 截图路径', 'Evidence / requests and responses / screenshot paths') }}</span
                ><textarea
                  v-model="validationForm.evidence"
                  rows="5"
                  :placeholder="tr('粘贴关键请求响应，或填写本地证据文件路径', 'Paste key requests and responses, or enter a local evidence file path')"
                ></textarea>
              </label>
            </div>
            <footer>
              <button
                class="button ghost"
                @click="emit('close-validation')"
              >
                {{ tr('取消', 'Cancel') }}</button
              ><button
                class="button primary"
                @click="saveValidation"
              >
                <Save :size="14" />{{ tr('保存并更新风险', 'Save and update risk') }}
              </button>
            </footer>
          </div>
          <section class="deep-dive-report">
            <div class="deep-dive-head">
              <strong><Search :size="14" />{{ tr('深度利用', 'Deep exploitation') }}</strong>
              <span v-if="exploitFor(item.id)" :class="`deep-status ${exploitFor(item.id)?.deepStatus}`">{{ deepStatusLabel(exploitFor(item.id)?.deepStatus ?? '') }}</span>
              <button
                class="button ghost compact deep-dive-run"
                :disabled="deepDiveBusyId===item.id||!!deepDiveDisabledReason"
                :title="deepDiveDisabledReason"
                @click="emit('deep-dive', item.id)"
              >
                <Activity :size="13" />{{ deepDiveBusyId===item.id ? tr('深挖中…', 'Deep-diving…') : tr('再深挖一次', 'Re-deep-dive') }}
              </button>
            </div>
            <p v-if="deepDiveDisabledReason" class="deep-dive-note">{{ deepDiveDisabledReason }}</p>
            <template v-if="exploitFor(item.id)">
              <div v-if="exploitFor(item.id)?.reproductionRequest" class="deep-field">
                <span>{{ tr('复现请求', 'Reproduction request') }}<button class="button ghost compact" @click="copyText(exploitFor(item.id)?.reproductionRequest ?? '')"><Copy :size="12" />{{ tr('复制', 'Copy') }}</button></span>
                <pre>{{ exploitFor(item.id)?.reproductionRequest }}</pre>
              </div>
              <div v-if="exploitFor(item.id)?.pocScript" class="deep-field">
                <span>{{ tr('PoC 脚本', 'PoC script') }}<button class="button ghost compact" @click="copyText(exploitFor(item.id)?.pocScript ?? '')"><Copy :size="12" />{{ tr('复制', 'Copy') }}</button></span>
                <pre>{{ exploitFor(item.id)?.pocScript }}</pre>
              </div>
              <div v-if="exploitFor(item.id)?.pocOutput" class="deep-field">
                <span>{{ tr('PoC 输出', 'PoC output') }}</span>
                <pre>{{ exploitFor(item.id)?.pocOutput }}</pre>
              </div>
              <div v-if="exploitFor(item.id)?.impact" class="deep-field">
                <span>{{ tr('影响', 'Impact') }}</span>
                <p>{{ exploitFor(item.id)?.impact }}</p>
              </div>
              <small class="deep-dive-note">{{ tr(`迭代 ${exploitFor(item.id)?.iterationsUsed ?? 0} 次 · 请求 ${exploitFor(item.id)?.requestsUsed ?? 0} 次`, `${exploitFor(item.id)?.iterationsUsed ?? 0} iterations · ${exploitFor(item.id)?.requestsUsed ?? 0} requests`) }}</small>
            </template>
            <p v-else class="deep-dive-note">{{ tr('尚无深挖记录。点「再深挖一次」为这条漏洞生成完整复现与 PoC（需已授权 target_write）。', 'No deep-dive yet. Click Re-deep-dive to produce a full reproduction and PoC for this finding (requires target_write authorization).') }}</p>
          </section>
        </article>
        <div v-if="!vulnerabilityRows.length" class="empty-inline">
          {{ tr('当前 URL 没有漏洞记录', 'No vulnerability records for this URL') }}
        </div>
      </div>
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Activity :size="16" />
        <div><strong>{{ tr('PoC 测试记录', 'PoC test records') }}</strong></div>
      </div>
      <div class="poc-list">
        <article v-for="item in pocRows" :key="item.id">
          <strong>{{
            json(item.recordJson).name || item.title
          }}</strong
          ><span>{{
            json(item.recordJson).result || "unknown"
          }}</span
          ><code
            >{{ json(item.recordJson).method || "GET" }}
            {{
              endpointUrl(selectedUrl, json(item.recordJson).url)
            }}</code
          >
          <p>
            {{
              json(item.recordJson).note ||
              json(item.recordJson).responseSnippet ||
              "—"
            }}
          </p>
        </article>
        <div v-if="!pocRows.length" class="empty-inline">
          {{ tr('没有 PoC 测试记录', 'No PoC test records') }}
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.deep-dive-report { margin-top: 12px; padding-top: 12px; border-top: 1px dashed var(--ui-line, #ddd); display: flex; flex-direction: column; gap: 8px; }
.deep-dive-head { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
.deep-dive-head > strong { display: inline-flex; align-items: center; gap: 6px; }
.deep-dive-run { margin-left: auto; }
.deep-status { padding: 1px 9px; border-radius: 999px; font-size: 11px; border: 1px solid var(--ui-line, #ddd); }
.deep-status.exploited { color: #b04432; border-color: color-mix(in srgb, #b04432 40%, var(--ui-line, #ddd)); font-weight: 600; }
.deep-status.waf_blocked, .deep-status.error { color: #b07b32; }
.deep-status.not_exploitable { color: var(--ui-muted, #777); }
.deep-status.in_progress, .deep-status.pending { color: var(--ui-muted, #777); }
.deep-field { display: flex; flex-direction: column; gap: 4px; }
.deep-field > span { display: flex; align-items: center; gap: 8px; font-size: 12px; font-weight: 600; color: var(--ui-muted, #777); }
.deep-field > span > button { margin-left: 0; }
.deep-field > pre { margin: 0; padding: 8px 10px; border: 1px solid var(--ui-line, #ddd); border-radius: 8px; background: var(--ui-surface-raised, #f7f7f7); overflow-x: auto; white-space: pre-wrap; word-break: break-word; font-size: 12px; line-height: 1.5; }
.deep-dive-note { font-size: 11px; color: var(--ui-muted, #777); margin: 0; }
</style>
