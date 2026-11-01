<script setup lang="ts">
// §E3 — the exposure surface's cluster view. It reads clusters page by page, expands one
// cluster's members on demand, and always separates a direct conclusion from an inherited one
// and from a member that lost admission. Nothing here can start, resume or cancel a round.
import { onMounted, ref, watch } from "vue";
import { Search } from "@lucide/vue";
import { exposureApi } from "./api";
import { useExposureClusters } from "./useExposureClusters";
import { useI18n } from "../uiPorts";

const { tr } = useI18n();
const props = defineProps<{ projectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();

const clusters = useExposureClusters({
  projectId: () => props.projectId,
  api: exposureApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  clusters: rows, members, expanded, total, hasMore, loading, loadingMore, stale, available,
  readError, kindFilter, searchFilter, notice, summary,
  load, loadMore, loadMembers, toggle, cacheLabel, evidenceLabel, boundaryText,
} = clusters;

const showProvenance = ref<number | null>(null);

onMounted(() => {
  void load();
});
watch(() => props.projectId, () => void load());
</script>

<template>
  <section class="panel exposure-clusters">
    <header>
      <div>
        <strong>{{ tr("内容簇与域名簇", "Content and domain clusters") }}</strong>
        <small>{{ notice }}</small>
      </div>
      <div class="exposure-clusters-tools">
        <label><span>{{ tr("类型", "Type") }}</span>
          <select v-model="kindFilter" @change="load">
            <option value="">{{ tr("全部", "All") }}</option>
            <option value="content">{{ tr("同字节内容簇", "Byte-identical content clusters") }}</option>
            <option value="domain">{{ tr("同注册域簇", "Same registered-domain clusters") }}</option>
          </select>
        </label>
        <label><Search :size="14" /><input v-model="searchFilter" :placeholder="tr('搜索簇键或成员资产', 'Search cluster keys or member assets')" @keyup.enter="load" /></label>
        <button class="button ghost" :disabled="!projectId || loading" @click="load">{{ tr("重新读取", "Reload") }}</button>
      </div>
    </header>

    <p v-if="!available" class="exposure-error">
      <strong>{{ tr("簇读取接口尚未注册", "Cluster read API is not registered yet") }}</strong>
      <span>{{ tr("页面不会把“读不到”显示成“没有聚类结果”；等主控注册 page_exposure_clusters 与 page_exposure_cluster_members 后才能给出真实簇。", 'This page never shows "unreadable" as "no clusters"; real clusters appear once the main app registers page_exposure_clusters and page_exposure_cluster_members.') }}</span>
    </p>
    <p v-else-if="readError && !rows.length" class="exposure-error">
      <strong>{{ tr("簇读取失败", "Failed to read clusters") }}</strong><span>{{ readError }}</span>
    </p>
    <p v-else-if="stale" class="exposure-stale"><strong>{{ tr("显示的是上一次成功读取的簇快照", "Showing the last successfully read cluster snapshot") }}</strong><span>{{ readError }}</span></p>

    <p class="exposure-clusters-bounds">{{ tr("簇只是分组：既不授予归属，也不授予授权；继承的成员没有自己的验证时间。", "Clusters are only groupings: they grant neither ownership nor authorization, and inherited members have no verification time of their own.") }}</p>

    <p v-if="rows.length" class="exposure-clusters-count">
      {{ tr(`本页 ${rows.length} / 共 ${total} 个簇 · 直接依据 ${summary.direct} · 继承 ${summary.inherited} · 带缺口 ${summary.gaps}`, `${rows.length} on this page / ${total} clusters total · direct basis ${summary.direct} · inherited ${summary.inherited} · with gaps ${summary.gaps}`) }}
    </p>

    <article v-for="row in rows" :key="row.id" class="exposure-cluster">
      <div class="exposure-cluster-head">
        <span class="exposure-cluster-kind" :class="row.kind">{{ row.kind === "content" ? tr("同字节", "Same bytes") : row.boundaryBasis === "exact_host" ? tr("同主机", "Same host") : tr("同注册域", "Same registered domain") }}</span>
        <strong>{{ row.representativeLabel || tr(`资产 ${row.representativeAssetId ?? "?"}`, `Asset ${row.representativeAssetId ?? "?"}`) }}</strong>
        <em>{{ tr(`${row.memberCount} 个成员（直接 ${row.directCount} / 继承 ${row.inheritedCount}）`, `${row.memberCount} members (direct ${row.directCount} / inherited ${row.inheritedCount})`) }}</em>
      </div>
      <small>
        {{ tr(
          `代表来自第 ${row.representativeRunId} 轮 · 观察 #${row.representativeObservationId ?? "无"} · 规则 ${row.ruleVersion || "未记录"} · 配置 v${row.settingsVersion} · 分组于 ${row.updatedAt}`,
          `Representative from round ${row.representativeRunId} · observation #${row.representativeObservationId ?? "none"} · rule ${row.ruleVersion || "not recorded"} · settings v${row.settingsVersion} · grouped at ${row.updatedAt}`,
        ) }}
      </small>
      <small class="exposure-cluster-boundary">{{ tr(`分组边界：${boundaryText(row)}`, `Grouping boundary: ${boundaryText(row)}`) }}</small>
      <small class="exposure-cluster-cache">{{ cacheLabel(row) }}</small>
      <p v-if="row.gapNote" class="exposure-gap">{{ row.gapNote }}</p>
      <div class="exposure-cluster-actions">
        <button @click="toggle(row.id)">{{ expanded.includes(row.id) ? tr("收起成员", "Hide members") : tr("展开成员", "Show members") }}</button>
        <button @click="showProvenance = showProvenance === row.id ? null : row.id">{{ tr("依据", "Basis") }}</button>
      </div>
      <div v-if="showProvenance === row.id" class="exposure-cluster-provenance">
        <span>{{ tr(`簇键 ${row.clusterKey}`, `Cluster key ${row.clusterKey}`) }}</span>
        <span>{{ tr("代表观察必须存在才写 direct；其余成员显示“继承自…”，并可指回被继承的资产与轮次。", 'A member is written as direct only when the representative observation exists; other members show "inherited from…" and link back to the inherited asset and round.') }}</span>
      </div>
      <div v-if="expanded.includes(row.id)" class="exposure-cluster-members">
        <p v-if="members[row.id]?.error" class="exposure-error"><strong>{{ tr("成员读取失败", "Failed to read members") }}</strong><span>{{ members[row.id]?.error }}</span></p>
        <p v-if="members[row.id]?.loading" class="exposure-empty">{{ tr("正在加载成员…", "Loading members…") }}</p>
        <ul v-else-if="members[row.id]?.items.length">
          <li v-for="member in members[row.id].items" :key="member.assetId" :class="{ revoked: !member.admittedNow }">
            <strong>{{ member.label || tr(`资产 ${member.assetId}`, `Asset ${member.assetId}`) }}</strong>
            <span>{{ evidenceLabel(member) }}</span>
            <small>
              {{ member.membership === "representative" ? tr("代表", "Representative") : tr("成员", "Member") }} ·
              {{ tr(`观察 #${member.observationId ?? "无"}`, `Observation #${member.observationId ?? "none"}`) }} ·
              {{ member.verifiedAt ? tr(`自带结论于 ${member.verifiedAt}`, `Own conclusion at ${member.verifiedAt}`) : tr("没有自己的验证时间", "No verification time of its own") }}
              <template v-if="member.inheritedRunId">{{ tr(` · 继承轮次 ${member.inheritedRunId}`, ` · inherited round ${member.inheritedRunId}`) }}</template>
            </small>
          </li>
        </ul>
        <p v-else-if="members[row.id] && !members[row.id].loading" class="exposure-empty">{{ tr("这个簇还没有可读的成员行", "This cluster has no readable member rows yet") }}</p>
        <button
          v-if="members[row.id]?.hasMore"
          class="button ghost"
          :disabled="members[row.id]?.loading"
          @click="loadMembers(row.id, true)"
        >
          {{ members[row.id]?.loading ? tr("正在加载…", "Loading…") : tr(`加载更多成员（已读 ${members[row.id]?.items.length ?? 0} / ${members[row.id]?.total ?? 0}）`, `Load more members (${members[row.id]?.items.length ?? 0} / ${members[row.id]?.total ?? 0} read)`) }}
        </button>
      </div>
    </article>

    <div v-if="loading && !rows.length" class="exposure-empty">{{ tr("正在加载簇…", "Loading clusters…") }}</div>
    <footer v-if="hasMore" class="exposure-more">
      <button class="button ghost" :disabled="loadingMore" @click="loadMore">
        {{ loadingMore ? tr("正在加载…", "Loading…") : tr(`加载更多簇（已读 ${rows.length} / ${total}）`, `Load more clusters (${rows.length} / ${total} read)`) }}
      </button>
    </footer>
  </section>
</template>
