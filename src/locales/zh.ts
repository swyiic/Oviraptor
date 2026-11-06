// Source catalog (Chinese). Every other language provides exactly these keys:
// the Messages type below makes a missing or extra key a TypeScript error.
// To add a language, copy en.ts to <code>.ts, translate the values and
// register it in LOCALES (src/i18n.ts). `{name}` marks a value filled by fmt().
export const zh = {
  app: { name: "Oviraptor" },
  common: {
    refresh: "刷新", export: "导出", save: "保存", cancel: "取消", confirm: "确认", close: "关闭",
    confirmDelete: "确认删除", deleting: "删除中…", empty: "暂无数据", language: "语言",
  },
  // One name per concept across the app (see the glossary in the UI notes).
  shell: {
    workspace: "工作空间", allWorkspaces: "全部工作空间", newWorkspace: "新建工作空间",
    workspacesAndScope: "工作空间与范围", manageWorkspaces: "管理工作空间与范围", chooseWorkspace: "选择工作空间",
    logs: "日志", settings: "配置中心", find: "搜索", appOptions: "应用选项", builtInPlugins: "内置插件",
  },
  modules: { chat: "对话", asset: "资产", sentinel: "扫描", hackerone: "HackerOne" },
  pages: {
    asset: {
      dashboard: "总览", tasks: "采集任务", assets: "资产列表", seeds: "发现种子", ownership: "归属确认",
      exposure: "公开暴露面", changes: "变化对比", quarantine: "隔离区", query: "发现资产", logs: "资产日志",
    },
    sentinel: {
      overview: "总览", queue: "扫描任务", urls: "结果与证据", dialog: "执行过程", fuse: "已停止目标",
      new_scan: "新建扫描", knowledge: "知识库", logs: "扫描日志", validations: "人工验证队列",
    },
  },
  settings: {
    groups: { modelsSkills: "模型与技能", assets: "资产", scanning: "扫描", machineNetwork: "机器与网络", accessPlugins: "权限与插件" },
    items: {
      models: "模型", skills: "技能", mcp: "MCP", agentKnowledge: "记忆与技法", assetApis: "资产 API", collection: "资产采集", probe: "资产探测",
      contentRules: "内容分类", pageRules: "页面与划词", agentPolicy: "扫描 Agent 策略", codeRules: "代码规则库",
      fingerprints: "指纹库", businessNetwork: "业务网络", workers: "Worker 节点", environment: "运行环境",
      dataPermissions: "数据权限", apps: "侧栏与应用",
    },
  },
  scanning: {
    vulnerabilities: "漏洞", targets: "目标", task: "任务", taskList: "扫描任务", runDetails: "执行过程",
    companyAssets: "公司资产", sourceContext: "源码上下文", newScan: "新建扫描",
    overviewIntro: "先看最近任务的漏洞数和可直接验证的机会；扫描在确定性流程形成高价值候选后才调用模型。",
    overviewMore: "统计、成本与流程说明",
    resultsOneTask: "结果与证据一次只看一个任务。从扫描任务列表或下方任务卡片打开一个任务。",
    chooseTask: "选择要查看的任务",
    taskListIntro: "每行最大的数字是该任务已发布的漏洞数。点击任务直接打开它的「结果与证据」；草稿任务打开确认面板。",
    process: {
      title: "调查过程", intro: "这里只看任务是怎么跑的；漏洞、指纹、接口等结论在「结果与证据」。",
      graph: "调查图谱", opportunities: "机会与下一步", trace: "执行轨迹", attempts: "执行与成本", routing: "路由与能力",
      informationGain: "信息增益 {value}/100", loading: "正在读取执行过程…", loadingTask: "正在载入该任务的执行过程…",
      noTrace: "该任务没有已保存的执行轨迹。", noRouting: "该目标暂无分流与能力记录。",
    },
    row: { run: "执行", details: "详情", confirmScan: "确认扫描", stopKeep: "停止并保留", stopping: "正在停止",
      resumePartial: "继续未完成", resume: "继续扫描", archive: "归档", unarchive: "恢复归档", delete: "删除",
      forceDelete: "强制删除", archived: "已归档", high: "高价值 {count}", mode: "任务模式：{mode}" },
    deleteTask: "确认删除任务「{name}」？",
    deleteActive: "当前任务仍在执行或等待退出，后端将拒绝删除。请先暂停并确认执行退出、请求已核清；删除不会强杀进程或清理磁盘文件。",
    deleteRecord: "这是整任务记录删除：删除关联 URL、解析结果、对话和人工验证记录；如有未知请求或未结算执行，将保留任务并拒绝删除。历史源文件与任务产物文件保留，不释放磁盘空间。仅需整理列表时，请使用归档。",
    forceDeleteTitle: "强制删除崩溃任务「{name}」？",
    forceDeleteDetail: "常规删除被拒绝：{reason}。该任务已崩溃但未结清（崩溃残留了未释放的预算预留或未结算分配）。强制删除会跳过结算校验，级联物理删除任务及关联记录，总览的 token 消耗随之扣减；财务审计记录原样归档保留（标记为强制删除），不会凭空消失。仍要求本地进程已退出，不会强杀进程或清理磁盘文件。此操作不可撤销。",
    pauseTitle: "确定请求暂停吗？",
    pauseDetail: "系统会等待执行线程退出后标记“已暂停”；已保存证据、未知目标效果和未确认清理记录会保留。",
    pauseConfirm: "请求暂停",
  },
} as const;

type Catalog<T> = { readonly [K in keyof T]: T[K] extends string ? string : Catalog<T[K]> };
/** The shape every language catalog must match. */
export type Messages = Catalog<typeof zh>;
