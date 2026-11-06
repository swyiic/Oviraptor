# 扫描重构:agent 拥有分析/漏洞/报告/链路层 + UI 瘦身 + 知识库迁移(设计)

状态:设计稿(2026-11-06,Opus)。全程授权渗透语境、守授权与 B4 审批墙,不规避任何安全判断。后端由本设计驱动,UI 交 Sonnet 5.5(§8)。

## 1. 目标(用户)
- agent 接管扫描任务"全流程"。
- 扫描界面只留 **总览 / 任务 / 结果**(总览=目标,任务=统计,结果=展示字段)。
- **知识库迁到 agent 主界面**。
- 设计:测试过程怎么产生实时展示信息;漏洞 + 漏洞细节(含 HTTP/WS 请求响应)怎么记录;测后汇总进 **MD 报告**怎么展示;相关漏洞脚本/前端解密脚本怎么归置与展示;结果页细节怎么呈现。

## 2. 现状(探明,决定能做什么)
- **扫描执行 = 原生 Rust 多代理流水线,不是 agent-host 聊天 agent。** 聊天 agent 只是"scan bridge"(`scanning-tasks.mjs` 的 `scan_start/scan_results/scan_command/scan_review/scan_verify…`):**触发+读取**扫描,真正扫描脑在 Rust(`agent_runtime/multi_agent/**` coordinator+子执行器)。两个 agent 是分开的。
- **单主脑接管(B3)= 脚手架,未上线**:`scheduler/controller_phase_lease.rs`(全 `#[allow(dead_code)]`)、`child_contract.rs`、`agent_native/executor.rs` 的 B3.3c "one brain" 仅 env `OVIRAPTOR_CONTROLLER_EXECUTION` 开(默认关、inert)。即 Stage 4 真·接管是后话。
- **扫描 UI**:`SentinelBoard.vue`(4033 行)一个组件扛所有页,tab = `overview|queue|results|fuse|validations|workbench|knowledge|dialog|help`;结果子页 `ResultTab = summary|fingerprint|api|endpoints|vulnerabilities`(`results/*.vue`)。导航 `shell/moduleCatalog.ts`。
- **漏洞/证据模型**:`sentinel_findings` 表(id,scan_id,target_url,stage,kind,record_key,title,severity,**record_json**),漏洞=`kind='vulnerability'`,富字段(cvss/confidence/cwe-cve/evidence/business_impact/chain_context/poc…)都在 record_json;`SentinelVulnerabilitiesPane.vue` 逐字段渲染 + deep-exploit(reproductionRequest/pocScript/pocOutput)。**完整 HTTP 请求/响应**在 `InvestigationValidation`(method/requestUrl/headers/body/responseStatus/headers/body/decodedBody);agent 的 HTTP 请求复核台 = `agent_request_reviews`(snapshot + 操作员 reviews)。**WS**:仅 `kind='realtime_endpoint'` finding,**没有独立 WS 请求/响应存储**(缺口)。
- **知识库**:`KnowledgeDistillation.vue` 挂在 sentinel `knowledge` tab;蒸馏库(贴文章/md→AI 蒸出 method cards)+ `aggregate_agent_knowledge` 从历史扫描挖。
- **脚本/产物**:聊天 agent 的 `workspace_run` 脚本在每对话私有工作区;扫描 agent 的 `run_analysis_script`(python 沙箱)记进 `tool_invocations`;`host_task` artifacts `{id,label,location}`。**缺口:没有任何 UI 把 workspace 脚本 / host_task 产物列给用户。**
- **报告**:导出只有 **JSON/SARIF/Bundle**(`export_native_source_findings`、`export_sentinel_results`)。**没有任何 Markdown 报告。**

## 3. 核心判断(agent 接管全流程现在怎么落)
不做 B3 大改(替换原生引擎,工作量巨大)。近期落地 **"agent 拥有分析/漏洞/报告/链路层"**:
- 原生引擎当**传感器**(抓资产/指纹/端点/原始流量);
- **agent(聊天 agent)当分析员/报告员**:经 scan bridge 读结果 → 用我们建的工具(大文件直读、carver/解密台骨架、工具库召回、jev 决策、深度指引)**深挖、解密、组合漏洞链、扩大危害** → 产结构化漏洞 + 证据 → 汇总 MD 报告。
- 这层**现在就能做**,不碰 B3。真·单主脑替换引擎(Stage 4)作为独立后话,本设计不依赖它。

## 4. 扫描 UI 瘦身 → 总览 / 任务 / 结果(§A)
`SentinelBoard` 的 9 个 tab 收成 3 个 + 迁出 2 个:
- **总览**:现 `overview`(执行总览 `SentinelOverviewSummary/Sidebar/TaskStrip`)+ 目标/范围 + "新建扫描"入口(把 `workbench` 新扫表单折成总览里的动作,不单列 tab)。
- **任务**:现 `queue`(任务列表/统计);`validations`(人工验证队列)折进来做子区;`fuse`(排除目标)折进总览或任务的子区。
- **结果**:现 `results` + 全部 `ResultTab`(summary/fingerprint/api/endpoints/**vulnerabilities**)+ 新增 **HTTP/WS 证据查看器、PoC 脚本/产物、MD 报告、漏洞链视图**(§D–G)。
- **迁出**:`dialog`(agent 对话)→ 并入 agent 主界面;`knowledge`→ agent 主界面库区(§B);`help`→ 结果页/总览的说明。
- 导航:`moduleCatalog.ts` 的 sentinel `pages` 砍成 overview/queue(任务)/results 三项;`SentinelBoard.vue` 删掉迁出 tab 的分支。**这是 5.5 的大刀。**

## 5. 知识库迁移 → agent 主界面(§B)
把 `KnowledgeDistillation.vue` 从 sentinel `knowledge` tab 搬到 agent 主界面的**顶层「库」区**,跟 5.5 已建的 记忆库 / 技法卡 / 工具库 并列(库区已在 `agent-top-ui` 分支)。后端 `ingest_agent_knowledge_source / aggregate_agent_knowledge / listAgentKnowledge` 不变,只换挂载位置。sentinel 的 knowledge 入口删除(或留重定向,同配置中心旧入口的处理)。

## 6. 测试过程实时展示(§C)
- **agent 分析层**:复用我刚做的**工具活动时间线 + `runtime.toolProgress`**(运行中工具实时输出预览 + 已输出行数)——agent 解密/碳刻/找加密点的过程直接在时间线上滚动。
- **原生扫描引擎**:沿用现有 `AgentDialog` 的 process 面板 + `SentinelTaskStrip` + `EXECUTION_STAGE_STEPS`。
- 两者在 UI 上并行:扫描引擎的阶段进度 + agent 分析的工具时间线。

## 7. 数据与新功能(后端我来)
### §D 漏洞 + HTTP/WS 证据记录
- **agent 产结构化漏洞入库**:新增 host 工具 `record_finding`(agent 调用)→ 写 `sentinel_findings`(kind='vulnerability', record_json 含 title/severity/cvss/confidence/cwe-cve/evidence/**chain_context 组合链**/business_impact/poc)。复用现有 `SentinelVulnerabilitiesPane` 渲染。
- **HTTP 证据**:把 agent 的 `workspace_http`/`workspace_browser` 捕获的 req/resp 作为 finding 的证据落库(复用 `InvestigationValidation` 的 method/url/headers/body/responseStatus/headers/body/decodedBody 形状,或 finding record_json 内嵌 requestResponse)。
- **WS 证据(补缺口)**:现仅 realtime_endpoint finding 无 req/resp。新增 WS 帧记录(方向/类型/payload/时间戳)作为 finding 证据。**已落地(增量4)**:host 工具 `workspace_ws`(workspace-tools.mjs)——`materialOpen`(授权)+ `allowedHosts` 锁定到 cited/已授权主机 + `blockedAddress` SSRF 硬墙(复用 saveFetchedUrl 的同一道墙,见 cited-pages.mjs `assertWsTarget`);打开 ws/wss、发可选文本帧、抓收发帧,上限 30s/100 帧/2MB、单帧 64KB;返回帧经 `record_finding` 的新 `wsFrames` 字段入库(findings.mjs `boundWsFrames`)。read-class,与 workspace_http 同类门禁;深度指引已提示 agent 用它。
- **硬墙**:record_finding/证据只是记录,不授予任何权限;实际发请求仍走授权+审批。
### §E MD 报告汇总
- 现在没有 MD 报告。**agent 用 `workspace_write` 把 `report.md` 写进工作区**(深度指引末句已要求它落结构化发现)——零后端即可产出。
- 展示:结果页新增**「报告」子页**,渲染该 md(markdown→HTML);导出按钮(复用导出目录)。Word/xlsx 模版渲染(应用侧套模版)=后话(用户明确先不加)。
- 结构化来源:agent 同时产一份 `findings.json`(供将来模版渲染/机器消费)。
### §F 脚本/产物归置与展示(补缺口)
- 现在没 UI 列 workspace 脚本/产物。新增**「产物与脚本」面板**(结果页子区或 agent 主界面):列当前对话工作区文件 + `host_task` artifacts(id/label/location),可查看脚本正文、下载。
- **可复用的解密/分析脚本 → 沉淀进工具库**(我已建的 `agent_tool_library`,参数化骨架),一键"入库"(operator 审),下次按指纹召回复用。闭环。
### §G 结果页细节
每条漏洞:现有富字段 + **HTTP/WS 请求响应查看器**(可折叠、脱敏)+ **PoC 脚本 + 运行输出** + **组合攻击链视图**(chain_context 可视化)+ **关联脚本链接**(指向 §F 的产物)+ 顶部一键看 **MD 报告**。

## 8. 前后端分工 + 契约
- **后端(我)**:`record_finding` 工具 + HTTP/WS 证据落库 + `workspace_ws`(若定)+ MD/findings 产出引导(提示词)+ 知识库迁移后端不变 + 把这些接进 scan bridge/agent-host。契约(命令/字段)定死给 UI。
- **UI(5.5,大刀)**:SentinelBoard 砍成 总览/任务/结果;knowledge 搬库区;结果页新增 报告子页 / HTTP·WS 查看器 / PoC / 链视图 / 产物脚本面板;导航 moduleCatalog 瘦身。按契约对接。

## 9. 开放决策(请你拍板,其余按推荐)
- **Q1 范围**:近期就做"agent 分析/报告层"(推荐,现在能落),B3 真·接管引擎留作后话?还是你要我现在就上 B3(大改、风险高)?
- **Q2 WS 抓取**:要不要现在加 `workspace_ws`(agent 主动抓 WS 帧作证据)?它是新增外发能力,要走授权+B4 审批;不加的话 WS 只能记 realtime_endpoint + 被动证据。
- **Q3 record_finding**:agent 自动入库漏洞 vs 操作员审一道再入库(推荐后者,防噪声/误报,像技法卡/工具库)?
- **Q4 UI 迁出的 dialog/knowledge**:旧入口删除还是留重定向(推荐留重定向,不炸深链)?

## 10. 守墙(不触发 safeguard)
全程授权渗透:record_finding/证据/报告只是**记录与呈现**,不授予权限、不扩范围;实际发请求/WS/高危动作仍走已确认授权 + B4 单次审批 + scope 冻结;agent 的分析在沙箱内;不规避模型安全判断。知识库/工具库/记忆/jev 决策均 advisory,永不进任何授权/范围/结算/能力闸。

相关记忆:[[scan-agent-redesign-staging]]、[[pentest-harness-design-docs]]、[[agent-host-workspace-file-capability]]、[[external-mcp-connector-jev]]、[[permission-two-axis-model]]。
