import { reactive, type Ref } from "vue";
import type { SentinelFinding, SentinelFuseEntry, SentinelTarget, SentinelValidation } from "../types";
import { useI18n } from "../../../i18n";

const { tr } = useI18n();

export type FuseDetailTab = "summary" | "fingerprint" | "assets" | "endpoints" | "proof";
type FuseDetailState = {
  open: boolean;
  loading: boolean;
  loaded: boolean;
  tab: FuseDetailTab;
  findings: SentinelFinding[];
  validations: SentinelValidation[];
};

// The label slot is a getter so each read follows the current UI language.
const detailTab = (key: FuseDetailTab, label: () => string) =>
  Object.defineProperty([key], 1, { get: label, enumerable: true }) as unknown as [FuseDetailTab, string];

export const fuseDetailTabs: [FuseDetailTab, string][] = [
  detailTab("summary", () => tr("概要", "Summary")),
  detailTab("fingerprint", () => tr("指纹配置", "Fingerprint")),
  ["assets", "JS / API"],
  detailTab("endpoints", () => tr("端点验证", "Endpoint checks")),
  detailTab("proof", () => tr("漏洞证明", "Vulnerability proof")),
];

export function sameTargetUrl(left: string, right: string) {
  const normalize = (value: string) => {
    try {
      const url = new URL(value);
      return `${url.protocol}//${url.host}${url.pathname.replace(/\/$/, "")}${url.search}`;
    } catch {
      return value.replace(/\/$/, "");
    }
  };
  return normalize(left) === normalize(right);
}

export function useFuseDetails(options: {
  targets: Ref<SentinelTarget[]>;
  loadFindings: (scanId: string) => Promise<SentinelFinding[]>;
  loadValidations: (scanId: string) => Promise<SentinelValidation[]>;
  notifyError: (message: string) => void;
}) {
  const states = reactive<Record<number, FuseDetailState>>({});
  const empty: FuseDetailState = {
    open: false, loading: false, loaded: false, tab: "summary", findings: [], validations: [],
  };
  function fuseState(item: SentinelFuseEntry) {
    return states[item.id] || empty;
  }
  function ensureState(item: SentinelFuseEntry) {
    return states[item.id] || (states[item.id] = {
      open: false, loading: false, loaded: false, tab: "summary", findings: [], validations: [],
    });
  }
  function fuseRows(item: SentinelFuseEntry, ...kinds: string[]) {
    return fuseState(item).findings.filter(
      (row) => sameTargetUrl(row.targetUrl, item.url) && kinds.includes(row.kind),
    );
  }
  function fuseTarget(item: SentinelFuseEntry) {
    return options.targets.value.find(
      (target) => target.scanId === item.sourceScanId && sameTargetUrl(target.url, item.url),
    );
  }
  function fuseValidationRows(item: SentinelFuseEntry) {
    return fuseState(item).validations.filter((row) => sameTargetUrl(row.url, item.url));
  }
  async function toggleFuseDetail(item: SentinelFuseEntry) {
    const state = ensureState(item);
    state.open = !state.open;
    if (!state.open || state.loaded || state.loading) return;
    state.loading = true;
    try {
      [state.findings, state.validations] = await Promise.all([
        options.loadFindings(item.sourceScanId),
        options.loadValidations(item.sourceScanId),
      ]);
      state.loaded = true;
    } catch (error) {
      state.open = false;
      options.notifyError(tr(`完整情报加载失败：${String(error)}`, `Failed to load full intelligence: ${String(error)}`));
    } finally {
      state.loading = false;
    }
  }
  return { fuseState, fuseRows, fuseTarget, fuseValidationRows, toggleFuseDetail };
}
