import { computed, ref } from "vue";

export type Locale = "zh" | "en";
if (localStorage.getItem("oviraptor-locale") === null && localStorage.getItem("asset-atlas-locale") !== null) {
  localStorage.setItem("oviraptor-locale", localStorage.getItem("asset-atlas-locale")!);
}
localStorage.removeItem("asset-atlas-locale");
export const locale = ref<Locale>((localStorage.getItem("oviraptor-locale") as Locale) || "zh");

const messages = {
  zh: {
    appName: "Oviraptor", dashboard: "仪表盘", projects: "项目", query: "新建查询", assets: "资产",
    quarantine: "隔离区", changes: "变化对比", tasks: "任务中心", logs: "操作日志", settings: "配置中心", allProjects: "全部项目",
    search: "搜索资产、域名、IP、标题…", newProject: "新建项目", newQuery: "新建查询", export: "导出",
    refresh: "刷新", save: "保存", cancel: "取消", confirm: "确认", close: "关闭", empty: "暂无数据",
  },
  en: {
    appName: "Oviraptor", dashboard: "Dashboard", projects: "Projects", query: "New Query", assets: "Assets",
    quarantine: "Quarantine", changes: "Changes", tasks: "Tasks", logs: "Logs", settings: "Profiles", allProjects: "All projects",
    search: "Search assets, domains, IPs and titles…", newProject: "New project", newQuery: "New query", export: "Export",
    refresh: "Refresh", save: "Save", cancel: "Cancel", confirm: "Confirm", close: "Close", empty: "No data",
  },
} as const;

export function useI18n() {
  const t = computed(() => messages[locale.value]);
  const tr = (zh: string, en: string) => locale.value === "zh" ? zh : en;
  const setLocale = (value: Locale) => {
    locale.value = value;
    localStorage.setItem("oviraptor-locale", value);
  };
  return { locale, t, tr, setLocale };
}
