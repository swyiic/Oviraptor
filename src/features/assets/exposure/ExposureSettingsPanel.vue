<script setup lang="ts">
// §E2 — the exposure surface's own collection settings panel. Read, edit, save as a new
// version; nothing here starts a round, and a refused save stays visibly refused.
import { computed, onMounted, ref, watch } from "vue";
import { Download, Save } from "@lucide/vue";
import { exposureApi } from "./api";
import { useExposureSettings, type SettingsNumberKey } from "./useExposureSettings";
import { buildImportRequest, capabilityLine, capabilityTone, exposureErrorText, sourceLabel } from "./contracts";
import { newOperationId } from "./incidents";
import { submitAssetsFormOnEnter } from "../formInteraction";
import { useI18n } from "../uiPorts";

const props = defineProps<{ projectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();

const settings = useExposureSettings({
  projectId: () => props.projectId,
  api: exposureApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  current, draft, history, loading, saving, error, conflict, available,
  bounds, versionLabel, allowedSources, sources, notice, isDefault, concurrencyNotice,
  capabilities, ownerText, ownerInvalid, documentText, documentInvalid,
  credentialRef, setCredentialRef,
  load, loadHistory, save, setNumber, setPrivateEgress, setSameHostHttpRedirects, setOfficialOwners, setDocumentUrls, toggleSource,
} = settings;

const numbers = computed<{ key: SettingsNumberKey; label: string; unit: string }[]>(() => [
  { key: "exposureMaxItems", label: tr("每轮工作项上限", "Max work items per run"), unit: tr("项：网页按资产、证书/归档按域名", "items; web pages count per asset, certificates/archives per domain") },
  { key: "exposureMaxRequests", label: tr("每轮请求上限", "Max requests per run"), unit: tr("次（含失败尝试）", "requests, including failed attempts") },
  { key: "exposureMaxResponseBytes", label: tr("单响应保留上限", "Max bytes kept per response"), unit: tr("字节", "bytes") },
  { key: "exposureMaxTotalBytes", label: tr("每轮字节上限", "Max bytes per run"), unit: tr("字节", "bytes") },
  { key: "exposureTimeoutSeconds", label: tr("请求超时", "Request timeout"), unit: tr("秒", "seconds") },
  { key: "exposureCtPageSize", label: tr("证书透明度每域名名称上限", "Max certificate transparency names per domain"), unit: tr("个，超出部分不记录", "names; extras are not recorded") },
  { key: "exposureArchivePageSize", label: tr("归档索引每域名行数", "Archive index rows per domain"), unit: tr("行，只读一页", "rows; only one page is read") },
  { key: "exposureAuthorizationDays", label: tr("授权有效期", "Authorization validity"), unit: tr("天（0=不设期限）", "days; 0 = no expiry") },
]);

onMounted(() => {
  void load();
});
watch(() => props.projectId, () => void load());

const importChannel = ref("document_import");
const importOperator = ref("");
const importDeclaration = ref("");
const importDeclaredUrl = ref("");
const importBatchLabel = ref("");
const importPublic = ref(false);
const importPublicNote = ref("");
const importExtractedFrom = ref("");
const importFormat = ref("text");
const importFiles = ref<{ name: string; format: string; contentBase64: string }[]>([]);
const importBusy = ref(false);
const importNotice = ref("");
const importResult = ref("");

watch(importChannel, (channel) => {
  if (channel === "public_intel_import" && (importFormat.value === "text" || importFormat.value === "extracted_text")) {
    importFormat.value = "json";
  }
});

async function onImportFiles(event: Event): Promise<void> {
  const list = (event.target as HTMLInputElement).files;
  if (!list) return;
  const next: { name: string; format: string; contentBase64: string }[] = [];
  for (const file of Array.from(list)) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let binary = "";
    for (let index = 0; index < bytes.length; index += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    }
    next.push({ name: file.name, format: importFormat.value, contentBase64: btoa(binary) });
  }
  importFiles.value = next;
  importNotice.value = tr(`已在本地读取 ${next.length} 个文件；声明的地址只记录，不会被访问`, `Read ${next.length} file(s) locally; the declared URL is only recorded and never visited`);
}

async function submitImport(): Promise<void> {
  if (!props.projectId) {
    importNotice.value = tr("请先选择工作空间", "Select a workspace first");
    return;
  }
  const built = buildImportRequest({
    channel: importChannel.value,
    operator: importOperator.value,
    sourceDeclaration: importDeclaration.value,
    declaredUrl: importDeclaredUrl.value,
    batchLabel: importBatchLabel.value,
    publicMaterial: importPublic.value,
    publicMaterialNote: importPublicNote.value,
    extractedFrom: importExtractedFrom.value,
    files: importFiles.value,
  }, newOperationId());
  if (!built.ok) {
    importNotice.value = built.error;
    importResult.value = "";
    return;
  }
  importBusy.value = true;
  try {
    const result = await exposureApi.importExposureDocuments(props.projectId, built.payload);
    importResult.value = JSON.stringify(result).slice(0, 4000);
    importNotice.value = tr("导入已写入", "Import saved");
  } catch (reason) {
    importResult.value = "";
    const code=(reason as {code?:unknown})?.code;
    importNotice.value = code==="assets_exposure_upload_outcome_unknown"||code==="assets_exposure_upload_review_required" ? tr(`上传或导入结果待核实，未自动重试；请读取原生上传状态：${exposureErrorText(reason)}`, `The upload or import outcome needs verification and was not retried automatically; check the native upload status: ${exposureErrorText(reason)}`) : tr(`导入未写入：${exposureErrorText(reason)}`, `Import not saved: ${exposureErrorText(reason)}`);
  } finally {
    importBusy.value = false;
  }
}
</script>

<template>
  <section class="panel exposure-settings">
    <header>
      <div>
        <strong>{{ tr('暴露面采集配置', 'Exposure collection settings') }}</strong>
        <small>{{ versionLabel }}{{ isDefault ? tr(" · 尚未保存", " · not saved yet") : "" }}</small>
      </div>
      <div>
        <button class="button ghost" :disabled="!projectId" @click="loadHistory"><Download :size="14" />{{ tr('查看版本历史', 'View version history') }}</button>
        <button class="button primary" :disabled="!projectId || saving || !available" @click="save">
          <Save :size="14" />{{ saving ? tr("正在保存…", "Saving…") : tr("保存为新版本", "Save as new version") }}
        </button>
      </div>
    </header>
    <p class="exposure-gap">{{ notice }}</p>
    <p class="exposure-gap">{{ tr(`${concurrencyNotice}；网页逐 URL 保存，并自动分批接续。每轮请求与字节上限是累计硬上限，暂停和恢复不会重置用量。`, `${concurrencyNotice}. Web pages are saved per URL and continue automatically in batches. The per-run request and byte caps are cumulative hard limits; pausing and resuming does not reset usage.`) }}</p>
      <p class="exposure-gap">{{ tr('并行度已从采集合同移除，执行器串行。', 'Parallelism has been removed from the collection contract; the executor runs serially.') }}</p>
    <p v-if="conflict" class="exposure-error"><strong>{{ tr('版本冲突', 'Version conflict') }}</strong><span>{{ error }}</span></p>
    <p v-if="loading" class="exposure-empty">{{ tr('正在读取配置…', 'Loading settings…') }}</p>
    <div v-else-if="current" class="exposure-settings-grid">
      <div class="exposure-settings-sources">
        <span>{{ tr('启用来源', 'Enabled sources') }}</span>
        <label v-for="name in (allowedSources.length ? allowedSources : sources)" :key="name">
          <input type="checkbox" :checked="sources.includes(name)" @change="toggleSource(name)" />
          {{ sourceLabel(name) }}
        </label>
        <small>{{ tr('勾选来源只是选择数据源，不构成跨范围授权；新发现的名字仍要人工复核。GitHub 与指定文档 URL 默认不启用。', 'Checking a source only selects a data source; it does not authorize anything beyond scope. Newly discovered names still need manual review. GitHub and specified document URLs are off by default.') }}</small>
      </div>
      <div class="exposure-settings-capabilities">
        <span>{{ tr('来源能力', 'Source capabilities') }}</span>
        <p v-if="capabilities === null" class="exposure-integration-pending">{{ tr('integration_pending：后端未报告 GitHub 能力与文档来源能力，不能假定可用', 'integration_pending: the backend has not reported GitHub or document source capabilities; do not assume they are available') }}</p>
        <ul v-else>
          <li
            v-for="cap in capabilities"
            :key="`${cap.sourceKey}:${cap.capability}`"
            :class="{ ready: capabilityTone(cap) === 'ok', unconfigured: capabilityTone(cap) === 'gap', 'exposure-integration-pending': capabilityTone(cap) === 'pending', unsupported: capabilityTone(cap) === 'unsupported' }"
          >
            {{ sourceLabel(cap.sourceKey) }} · {{ capabilityLine(cap) }}
          </li>
        </ul>
        <small>{{ tr('仓库元数据可用不代表代码搜索可用；没有凭据时代码搜索不发任何请求。文件导入与情报导入命令已注册（IR-14 已落地）。下面的表单只提交本机选择的文件。', 'Repository metadata being available does not mean code search is; without credentials, code search sends no requests. File import and intel import commands are registered (IR-14 shipped). The form below only submits files chosen on this machine.') }}</small>
      </div>
      <div role="form" class="exposure-import" @keydown="submitAssetsFormOnEnter($event, submitImport, importBusy)">
        <strong>{{ tr('导入文件或情报报告', 'Import files or intel reports') }}</strong>
        <small>{{ tr('只接收本机选择的文件。声明的 URL 只作为来源记录保存，应用不会打开它。PDF、Office、图片不能导入。', 'Only files chosen on this machine are accepted. A declared URL is saved only as a source record; the app never opens it. PDF, Office and image files cannot be imported.') }}</small>
        <label><span>{{ tr('渠道', 'Channel') }}</span>
          <select v-model="importChannel">
            <option value="document_import">{{ tr('文件导入（文本 / JSON / CSV / 已提取文本）', 'File import (text / JSON / CSV / extracted text)') }}</option>
            <option value="public_intel_import">{{ tr('公开情报报告（仅 JSON / CSV）', 'Public intel report (JSON / CSV only)') }}</option>
          </select>
        </label>
        <label><span>{{ tr('操作人', 'Operator') }}</span><input v-model="importOperator" maxlength="80" /></label>
        <label class="wide"><span>{{ tr('来源说明', 'Source description') }}</span><input v-model="importDeclaration" maxlength="500" :placeholder="tr('这份材料从哪来', 'Where this material came from')" /></label>
        <label class="wide"><span>{{ tr('声明的 URL（可选，只记录）', 'Declared URL (optional, recorded only)') }}</span><input v-model="importDeclaredUrl" :placeholder="tr('不会被访问', 'Never visited')" /></label>
        <label><span>{{ tr('批次名（可选）', 'Batch name (optional)') }}</span><input v-model="importBatchLabel" /></label>
        <label class="exposure-check"><input v-model="importPublic" type="checkbox" />{{ tr('声明为公开材料', 'Declare as public material') }}</label>
        <label v-if="importPublic"><span>{{ tr('公开理由', 'Why it is public') }}</span><input v-model="importPublicNote" maxlength="200" /></label>
        <label v-if="importFormat === 'extracted_text'"><span>{{ tr('提取前的原格式', 'Original format before extraction') }}</span><input v-model="importExtractedFrom" /></label>
        <label><span>{{ tr('格式', 'Format') }}</span>
          <select v-model="importFormat">
            <option v-if="importChannel === 'document_import'" value="text">text</option>
            <option value="json">json</option>
            <option value="csv">csv</option>
            <option v-if="importChannel === 'document_import'" value="extracted_text">extracted_text</option>
          </select>
        </label>
        <label><span>{{ tr('文件', 'Files') }}</span><input type="file" multiple @change="onImportFiles" /></label>
        <button type="button" :disabled="importBusy" @click="submitImport">{{ tr('写入导入', 'Save import') }}</button>
        <small v-if="importNotice">{{ importNotice }}</small>
        <pre v-if="importResult">{{ importResult }}</pre>
      </div>
      <label class="exposure-settings-owners">
        <span>{{ tr('GitHub 正式开源账号（人工维护，每行一个）', 'Official GitHub open-source accounts (maintained manually, one per line)') }}</span>
        <textarea :value="ownerText" rows="3" :placeholder="tr('例如 acme-official', 'e.g. acme-official')" @input="setOfficialOwners(($event.target as HTMLTextAreaElement).value)" />
        <small v-if="ownerInvalid.length" class="exposure-error">{{ tr(`不合规：${ownerInvalid.join("、")}（不会保存）`, `Invalid: ${ownerInvalid.join(", ")} (will not be saved)`) }}</small>
        <small v-else>{{ tr('这些账号下的仓库归为“公开材料”排除；名字像集团的其它账号不会因此被排除或认定为集团所有。', 'Repositories under these accounts are excluded as “public material”. Other accounts with group-like names are not excluded or treated as group-owned because of this.') }}</small>
      </label>
      <label class="exposure-settings-owners">
        <span>{{ tr('GitHub 凭据引用（钥匙串账户名，不是 token）', 'GitHub credential reference (Keychain account name, not a token)') }}</span>
        <input :value="credentialRef" :placeholder="tr('例如 exposure-github', 'e.g. exposure-github')" @input="setCredentialRef(($event.target as HTMLInputElement).value)" />
        <small>{{ tr('token 只放在系统钥匙串，服务名 oviraptor.exposure.github。这里只保存引用名，不保存明文，也不会发到日志里。', 'The token is stored only in the system Keychain under the service name oviraptor.exposure.github. Only the reference name is saved here; the plaintext is never saved or written to logs.') }}</small>
      </label>
      <label class="exposure-settings-documents">
        <span>{{ tr('指定公开文档 URL（每行一个，最多 20 个；“URL | 理由”表示人工声明为公开材料）', 'Specified public document URLs (one per line, up to 20; “URL | reason” manually declares it public material)') }}</span>
        <textarea :value="documentText" rows="3" placeholder="https://www.example.org/standard.txt" @input="setDocumentUrls(($event.target as HTMLTextAreaElement).value)" />
        <small v-if="documentInvalid.length" class="exposure-error">{{ tr(`不合规：${documentInvalid.join("、")}（不会保存）`, `Invalid: ${documentInvalid.join(", ")} (will not be saved)`) }}</small>
        <small v-else>{{ tr('只在启用“指定公开文档 URL”来源时读取这些精确地址，每个一次、跳转不跟随、只限 80/443；PDF/Office/图片只记元数据。公开声明只排除“提及”线索，凭据形态值仍按风险处理。', 'These exact URLs are read only when the “Specified public document URLs” source is enabled: once each, without following redirects, on ports 80/443 only. PDF/Office/image files record metadata only. A public declaration excludes only “mention” leads; credential-like values are still treated as risks.') }}</small>
      </label>
      <label v-for="row in numbers" :key="row.key">
        <span>{{ tr(`${row.label}（${row.unit}）`, `${row.label} (${row.unit})`) }}</span>
        <input
          type="number"
          :value="draft[row.key]"
          :min="row.key === 'exposureAuthorizationDays' ? 0 : 1"
          @input="setNumber(row.key, ($event.target as HTMLInputElement).value)"
        />
      </label>
      <label class="exposure-settings-flag exposure-http-option">
        <input type="checkbox" :checked="draft.exposureAllowSameHostHttpRedirects === true"
          @change="setSameHostHttpRedirects(($event.target as HTMLInputElement).checked)" />
        <span>{{ tr('跟随已授权 HTTPS 入口的同域 HTTP 跳转', 'Follow same-host HTTP redirects from authorized HTTPS entries') }}</span>
        <small>{{ tr('默认关闭。仅允许已确认 HTTPS 的同一完整主机名在 443 与 80 端口之间继续公开 GET；不携带 Cookie 或授权凭据，不扩大到兄弟子域或其他端口。HTTP 传输未加密，结果保留警告。保存后仅用于新建轮次。', 'Off by default. Only the exact same hostname with confirmed HTTPS may continue public GETs between ports 443 and 80; no cookies or authorization credentials are sent, and it never extends to sibling subdomains or other ports. HTTP is unencrypted, so results keep a warning. After saving, it applies only to newly created runs.') }}</small>
      </label>
      <label class="exposure-settings-flag">
        <input
          type="checkbox"
          :checked="Boolean(draft.exposureAllowPrivateDestinations)"
          @change="setPrivateEgress(($event.target as HTMLInputElement).checked)"
        />
        <span>{{ tr('允许内网/回环地址出口', 'Allow egress to private/loopback addresses') }}</span>
        <small>{{ tr('默认关闭。开启只对之后的轮次生效；测试镜像必须在配置里显式批准，不能因为“选了来源”就访问。', 'Off by default. Turning it on affects only later runs; test mirrors must be explicitly approved in the settings and are never accessed just because a source is selected.') }}</small>
      </label>
      <p v-if="bounds.units" class="exposure-settings-bounds">
        {{ tr(`上界：请求 ${(bounds.maxRequests as number)} 次/轮 · 单响应 ${(bounds.maxResponseBytes as number)} 字节 · 工作项 ${(bounds.maxItems as number)} 个/轮 · 超时 ${(bounds.timeoutSeconds as number)} 秒`, `Upper bounds: ${(bounds.maxRequests as number)} requests/run · ${(bounds.maxResponseBytes as number)} bytes per response · ${(bounds.maxItems as number)} work items/run · ${(bounds.timeoutSeconds as number)} s timeout`) }}
      </p>
    </div>
    <div v-if="history?.items.length" class="exposure-settings-history">
      <strong>{{ tr('版本历史', 'Version history') }}</strong>
      <div v-for="row in history.items" :key="row.version">
        <span>v{{ row.version }} · {{ row.createdAt }}</span>
        <small v-if="row.corrupt || !row.settings">{{ tr('该版本内容无法解析，原字节保留在历史中', 'This version could not be parsed; its original bytes are kept in the history') }}</small>
        <small v-else>
          {{ tr(`工作项 ${(row.settings.exposureMaxItems as number) ?? "?"} · 请求 ${(row.settings.exposureMaxRequests as number) ?? "?"} · 来源 ${(row.settings.exposureSources as string[])?.join("/") ?? "?"}`, `Work items ${(row.settings.exposureMaxItems as number) ?? "?"} · requests ${(row.settings.exposureMaxRequests as number) ?? "?"} · sources ${(row.settings.exposureSources as string[])?.join("/") ?? "?"}`) }}
        </small>
      </div>
      <p v-if="history.truncated" class="exposure-gap">{{ tr(`只显示最近 ${history.items.length} 个版本，更早的版本未列出（不是完整历史）。`, `Showing only the latest ${history.items.length} versions; earlier versions are not listed (this is not the full history).`) }}</p>
    </div>
  </section>
</template>
