# Changelog

## 1.1.4 - 2026-08-09

- 修复 Strix 1.5 将 `strix-evidence-input` 本地隔离目录误当成 Web URL 的问题。Web 结果入库只接受 `http://` / `https://` 目标，关联任务优先复用原始 URL；本地目录不再生成“未提供公司”，代码审计的源码路径不受影响。
- 启动迁移与结果同步会自动清理历史伪目标。单 URL 任务下误绑到本地证据目录的发现会回归真实 URL，多 URL 无法确定归属时保留在任务总览，不再制造虚假公司；结果页还有同口径防御过滤。
- 任务删除确认明确标注为“整任务删除”，会删除任务下全部 URL、公司归属、证据和验证记录，避免把伪目标分组误解为可独立删除的公司。
- Web 任务“继续当前任务”新增会话预检：先主动校验所有绑定身份，识别 8 小时过期、远端跳回登录页或本地失效状态；单个 401/403 仍只记权限边界，明确 WAF/挑战仍交由目标熔断。
- 会话失效时在当前页面直接展开恢复卡：重新打开同一 WebView 登录入口，人工完成验证码/SSO 后保存，绿灯恢复即可在原任务、原证据和原累计 Token 上续跑，不创建新扫描任务。
- 仍被扫描任务引用的登录会话禁止直接删除，避免任务保留悬空会话 ID 后再也找不到重新登录入口；任务与成本页为失败/完成任务补齐“继续当前任务”操作。
- 新增 Web 本地目标污染隔离、历史修复、代码路径保留和扫描策略会话 ID 去重回归测试；Rust 测试增至 79 项，前端类型检查与生产构建通过。

## 1.1.3 - 2026-08-09

- 修复 Strix 1.5.x 启动健康检查被误计为正式扫描轮次的问题。`Reply with just 'OK'.` 现在单独标记为模型维护请求：Token 仍完整计入成本，但不会增加扫描轮次、失败次数或“无工具进展”计数。
- 修复首个真实 Agent 响应尚在写入工具调用时触发无进展熔断的连锁故障。当前失败任务的 45,088 Token 来自一次健康检查和一次真实根 Agent 调用，并非 Django/指纹识别失败；新版不会在第一轮工具执行前终止 Strix。
- 调查图谱改为容器响应式布局。页面状态、动作、请求/API、假设会按结果区实际可用宽度切换为四列、两列或单列，不再因应用窗口宽但结果内容区窄而裁掉右侧节点。
- 验证契约默认只展示价值最高的 12 条，其余显式展开；长端点、标题和证据文本增加省略与最小宽度保护，避免 90 条候选同时渲染形成超长、横向溢出的页面。
- “任务与成本”取消常驻左右双栏，改为全宽任务列表与下方紧凑详情；成本卡降低高度，任务目标、尝试信息和操作按钮在详情区按容器宽度重新排版。
- 运行轨迹会明确区分“模型健康检查”“上下文压缩”和正式模型请求，便于解释请求数量与成本，不再把维护流量伪装为 Agent 扫描。

## 1.1.2 - 2026-08-09

- Strix 命令构造从版本号判断改为启动时能力探测：解析当前可执行文件真实支持的 `--target`、`--target-list`、`--mount`、`--instruction-file`、`--max-turns`、预算、范围和 diff 参数，并按二进制路径、大小和修改时间缓存；Strix 原地升级后自动重新探测。
- 同时兼容 Strix 1.3.x 与 1.5.x：支持 `--mount` 时继续使用只读挂载；新版移除该参数时自动使用本地目录 `--target`。`--max-turns` 和预算参数仅在 CLI 声明确认支持时传入，不再因新增、移除或别名变化在参数解析阶段失败。
- Web 扫描不再把整个任务目录交给可写挂载。前端证据、代码切片、路由、运行时结果和认证文档先复制到隔离输入目录，提示词使用确定的工作区路径；运行前后比较 SHA-256 清单，副本被修改时拒绝采信结果，原始 Oviraptor 证据保持不变。
- 源码工作台在 Strix 不提供只读挂载时自动创建隔离源码快照，排除 `.git`、依赖、构建产物、缓存和覆盖率目录，并设置 20 万文件/2GB 上限；Strix 可写操作只影响快照，不会修改用户项目。支持只读挂载的版本继续直接挂载，避免无意义复制。
- 运行日志新增 Strix 版本、实际本地输入参数、范围/diff/max-turns/预算能力，未来升级失败时可直接看到缺失能力。必要核心参数缺失会在模型启动前返回明确兼容性错误，不消耗 Token。
- 新增 Strix 1.3/1.5 能力矩阵、未来不兼容 CLI、参数选择、Web 证据隔离和输入篡改回归测试；Rust 测试增至 76 项。

## 1.1.1 - 2026-08-09

- 修复 Strix Web 任务拿不到 `frontend-evidence.json` 的关键链路：每个 URL 的证据目录现在通过只读 `--mount` 进入沙箱，目标级指令携带确定的 `/workspace/<target>/frontend-evidence.json` 路径；证据缺失或挂载不可读时立即停止，不再用模型回合搜索空工作区。
- 修复失败任务被空 SARIF 伪装成“扫描完成”的状态错误。只有 `run.json` 明确进入成功终态且进程正常退出才计为完成；`failed`、`interrupted`、非零退出和仅存在空结果文件都会保留真实失败/部分状态，也不会继续触发自动学习。
- 解决 Oviraptor 禁止子 Agent 与 Strix 根 Agent 只能调度的内在冲突：Web 任务固定为一个根协调器加一个定向验证 Agent，禁止额外 Agent；为调度开销保留最小请求预算，同时继续执行 Token、无进展、目录发现和 WAF 熔断。
- 前端运行时在入队前过滤语言/i18n/locale 噪声，并按已解析 URL、表单结构、控件结构和 DOM 规模去重页面状态，避免登录页多语言跳转耗尽全部状态预算。
- 保存、提交、创建、上传、导入、确认等高价值控件新增“只捕获不发送”动作：浏览器真实执行点击并在 CDP Fetch 层中止产生的请求，仍保存 method、URL、参数、请求体字段、有效请求头和动作因果；删除、支付、退出、重置等高风险控件继续不触发。
- 统一 401/403、WAF 与兜底发现规则：孤立权限拒绝只记录边界并继续其它功能，只有明确登录失效、持续同质拦截、WAF/人机挑战或持续 429 才停止；首次非静态目标允许一次基于已观察业务词的有界目录/API 兜底，不再依赖 Django、Spring Boot、若依或前端框架标签是否准确。
- 调查假设新增端点级状态变更授权：默认只读/只捕获，人工可为具体 method + endpoint 授权一次、30 分钟后自动过期并可立即撤销；授权随紧凑证据包传给验证 Agent，不能扩展到其它目标、接口或尝试次数。
- 新增空 SARIF 状态、框架兜底、授权过期、i18n 去重和浏览器 Mutation 拦截回归测试；浏览器集成测试确认 POST 未到达测试服务器，而请求结构和隐藏客户端头仍被完整记录。

## 1.1.0 - 2026-08-09

- 新增第一等调查图谱：页面状态、自动动作、请求/API、参数、登录身份和安全假设分别保存为节点，点击触发请求、接口包含参数、身份观察和证据支持假设保存为可查询关系；旧版被压扁的前端结果不再需要人工跨页拼接。
- 新增结构化动作协议和 API 参数模型。每次安全交互都保存前置页面、控件、结果、状态变化、请求数量和停止规则；API 模型统一保存请求/响应结构、来源、状态/动作因果、身份范围及新增/变化/未变化状态。
- 新增增量基线和本地信息增益门禁：按工作空间、URL、身份比较 API/参数签名，显示新增、变化、消失和重复证据；复杂前端或重复任务没有新高价值假设时不启动模型，普通服务端 Web 仅在首次基线保留一次有界目录/API 兜底。
- 浏览器会话中心支持同时选择最多五个绿色身份。同一 URL 使用同一动作计划逐身份渲染，形成 API 可达性、状态码和响应结构差异矩阵；单个 401/403 只记权限边界，明确跳回登录页才熄灭对应身份，确认 WAF/挑战则立即停止目标。
- 每个安全假设自动生成验证契约，明确前置条件、必须证据、最大尝试次数、变更策略、成功标准和 WAF/限速/无增益停止条件。发送给 Strix 的紧凑证据包包含图谱决策、API 模型、动作因果、身份差异和验证契约，Agent 不再接收开放式全站探索任务。
- 知识沉淀拆为事实、策略和结果三层：事实只来自运行时/AST/请求证据；策略需至少两个独立任务支持或一次已确认验证才晋升；结果同时记录验证、排除、耗尽和停止原因，避免更换模型后重复学习或反复走无效路径。
- Strix 结果新增“调查图谱”界面，集中展示信息增益、模型门禁、页面→动作→请求→假设因果链、API/参数增量、身份差异和可操作验证契约；概览新增平均信息增益、允许模型目标、确定性事实和已晋升策略统计。
- 数据库新增调查节点、关系、动作、API 模型、假设、身份差异、指标、基线及三层知识表；迁移对旧数据库幂等执行。新增 Rust 图谱/增量/知识回归测试和 Python 多身份矩阵测试，前端类型检查与生产构建同步覆盖。

## 1.0.20 - 2026-08-09

- 将 19,359 行的 Rust `commands.rs` 重构为 68 行稳定入口和 17 个按业务域组织的实现文件，覆盖工作空间、资产、HackerOne、环境、扫描生命周期、知识学习、规则包、前端侦察、运行环境、执行器、代码分析、控制面、AppSec、结果入库、导出与测试。
- 后端采用同模块 `include!` 边界，保持所有 Tauri 命令名、私有辅助函数关系和调用方不变；浏览器登录会话继续作为独立 Rust 模块，71 个 Rust 测试全部通过。
- 前端 Tauri 调用层按工作空间、资产、HackerOne、Strix 和运行环境拆为五个 Feature API；原 `api` 保留为 17 行兼容门面，现有组件无需迁移即可继续使用。
- 将 Sentinel 的状态、风险、指纹、敏感信息降噪、Token 格式与学习门禁解释抽到独立展示规则模块，避免扫描页面继续堆积跨场景判断。
- 新增架构边界文档，明确新命令、新 API、展示规则和业务组件的归属，避免重新长回单文件；本版本不新增数据库表或字段。

## 1.0.19 - 2026-08-09

- Web 与灰盒扫描新增真正的浏览器登录会话：打开独立 WebView 后可人工处理动态验证码、扫码、SSO 和多步骤登录；进入任意后台功能后完成捕获，应用会保存同源 Cookie（含 HttpOnly/Secure）、Local/Session Storage、认证头、请求证据和会话作用域。
- 登录会话中心新增绿/黄/红状态灯、Cookie/认证头/Storage/请求数量、作用域、最近校验、8 小时安全期限、重新登录、主动校验和删除。会话绑定公共工作空间，项目删除影响分析也会统计浏览器会话。
- Web 前端运行时会在页面探索前自动注入有效会话，并由 Chromium 重新生成 Host、Content-Length、Sec-* 等浏览器头；CDP 继续保存实际生效请求头、Cookie 名称、请求发起位置和响应证据。带登录态的任务不再复用匿名前端检查点。
- 调整认证与熔断语义：单个 401/403 作为登录/角色权限边界保留并继续其他同源功能，不再误判会话失效；只有明确跳回登录页且没有成功业务请求才熄灭会话灯。确认 WAF、机器人挑战、验证码或持续限流时立即停止当前目标并进入熔断区。
- 新建工作空间改为顶部文字主按钮、侧栏固定入口和扫描表单空状态强入口，降低“先去资产页建项目”的操作成本；登录窗口关闭不再被主窗口的后台隐藏逻辑拦截。
- 浏览器认证从超长 `commands.rs` 中拆分为独立模块，并补齐会话表迁移、项目级外键、敏感文件 0600 权限、作用域校验、过期门禁、重扫复用和 WAF/权限边界回归测试。

## 1.0.18 - 2026-08-09

- 将“项目”提升为 Asset 与 Strix 共用的顶层工作空间：顶部提供唯一的新建入口，项目页统一展示资产、Strix 任务、漏洞结论、停止记录和最近活动，并可直接进入对应模块。
- Asset 查询与 Strix 工作台都支持在表单内原地创建工作空间；保存后自动选中新项目，同时保留已经填写的 URL、源码、认证、预算和扫描策略，不再要求先跳到 Asset 创建空项目。
- 项目删除门禁覆盖 Asset 资产/目标/运行/事件/保存视图，以及 Strix 扫描、目标、证据、机会、验证、停止记录、漏洞结论、知识与学习候选。只有真正空的工作空间可以物理删除；其他项目只能归档，避免产生失去项目归属的任务和证据。
- 已归档工作空间统一为历史只读作用域：仍可查询 Asset 与 Strix 历史、编辑人工结论并恢复项目，但不能新建、确认、重扫、复测或从熔断区自动重试；前后端入口采用相同门禁。
- 修复编辑/归档不存在项目仍返回成功的问题；项目选择器明确标出归档状态，Asset 复测入口在归档作用域下改为“仅查看”，概览提供直接恢复操作。

## 1.0.17 - 2026-08-09

- 移除 Strix 各页面重复且互相独立的“所有项目”下拉；任务、证据、漏洞、验证与停止队列统一跟随应用顶部的唯一项目作用域，修复当前项目与列表/统计不一致的问题。
- “任务与成本”新增缓存命中率、每个漏洞 Token、零漏洞产出任务与最高成本任务四类决策指标；任务预览默认选中首项，减少空白面板和只展示累计数字却无法判断成本去向的问题。
- 漏洞结论改为索引 + 当前发现详情的主从布局，代码审计与 Web 漏洞不再一次展开所有长证据；选择发现后集中查看描述、技术分析、证据、影响、修复和验证结论。
- 新增漏洞验证工作台查询：直接联结扫描发现与人工结论，同时展示待验证、需补证、真实漏洞和误报，不再只有保存过记录后才出现内容；支持在工作台内查看证据摘要、保存结论和跳转完整证据。
- “停止与熔断”改为 URL 处置队列，按成本/无进展、缺少访问条件、遭到拦截、网络异常和价值不足分类，并给出下一步建议；“编辑验证记录”改为“记录处置决定”，与漏洞真假验证彻底分离。
- 恢复熔断 URL 的文案与行为统一为“复用前端证据、保留历史和累计成本、在原工作流中续跑”，完成处置则可独立归档。

## 1.0.16 - 2026-08-09

- 行动中心“正在调查”和“可验证机会”新增红色数字角标与非零高亮，机会筛选按钮同步显示角标；概览统计改为与当前项目一致的查询口径，修复跨项目累计数量与当前列表不一致导致的可验证区域空白。
- 可验证门禁下没有候选时不再只显示空白：界面会保留并列出最高分的有效线索，明确提示尚缺什么证据，同时避免把普通指纹或低分路径误标成可验证结果；每张机会卡新增证据数量。
- 浏览器侦察接入 CDP `requestWillBeSentExtraInfo` / `responseReceivedExtraInfo`，合并浏览器网络栈实际补充的请求头、Cookie 名称与阻止原因、响应头、协议和 Service Worker 来源，并记录请求发起脚本、函数和行号。
- AST 分析新增 axios/fetch/XHR 的 Header 契约提取，支持客户端默认 Header、单次请求配置、`setRequestHeader` 和动态占位值；API 页面分层展示“运行时真实生效”“JS 明确声明”“浏览器可能管理但尚未观察”三类信息。
- 自动前端探索会在每次点击后重新读取新出现的菜单、标签和弹窗控件，并覆盖开放 Shadow DOM 与同源 iframe；新增 WebSocket/EventSource 握手清单，提升复杂前端的状态、实时接口和二次请求发现率。
- 浏览器运行时已经真实观察到的 API 不再被无登录态的安全 GET 二次探测降级或丢弃；模型证据包只携带请求头名称与来源，不携带 Cookie、Token、Authorization 等实际值。

## 1.0.15 - 2026-08-09

- 新增可直接阅读和编辑理解的内置 Markdown Skill“业务前端深度分析”，按看功能、自动触发安全控件、关联 HTTP 请求、还原接口参数、深挖业务 JS、粗粒度指纹/本地知识匹配、一次性目录/API 保底和明确停止条件复刻日常测试习惯。
- 合并 AntiDebug_Breaker 的接口拆分思路：AST 新增数组 `join` 与字符串证据提取；运行时请求拆成 `origin + apiPrefix + businessEndpoint`；只使用显式 baseURL/apiPrefix 或至少两条真实请求支持的公共前缀重组业务路径，并保存置信度与完整证据来源。
- 前端侦察新增 `apiIntelligence`，将真实请求、AST 直出和证据重组接口统一去重、排序和安全 GET 验证。重组但尚未验证的候选会在 API 列表明确标记，不再伪装成真实接口；修正 client baseURL 与已有前缀重复拼接的问题。
- Asset Web 扫描默认只注入内置业务前端流程，不再随着本地学习 Skill 增多而把所有启用 Skill 塞入每次提示词。专项 Skill 仍可在工作台显式选择，相关知识继续通过当前机会证据匹配。
- 学习候选升级为规范化事实层：保存模型、部署、提示词哈希、规范化版本、canonical key、确定性工具/发现事实和学习策略。相同扫描换模型不会增加知识支持数，单任务知识不会自动影响后续精炼。
- 接受候选和“知识转 Skill”改为确定性的 Markdown 章节合并，不再额外调用当前模型二次改写，因此更换模型接口不会在应用阶段悄悄产生另一份结果或消耗额外 Token；显式“用最新知识精炼”仍作为独立操作保留。
- LLM 分析界面新增候选生产模型、部署、规范化版本、canonical key 和知识独立任务/模型支持数；补充 4 个接口智能单元测试和 2 个 Rust 学习/default Skill 回归测试。

## 1.0.14 - 2026-08-08

- 重构 Asset 工作区的信息层级：增加未审核、待补证据、已确认有效和已送 Strix 的队列概览，拆分探测队列、人工结论、Strix 状态、回收范围和排序控件，并重新适配窄屏布局。
- 修正批量选择只按全局 `asset_id` 导致跨项目碰撞的问题。所有批量结论和回收操作现在使用 `project_id + asset_id`，选择可安全跨页、跨项目保留，发送 Strix 时会按项目拆分任务。
- 明确人工结论语义：“确认有效”保存为 `confirmed` 并移出待复核队列；“保留待核”保存为 `uncertain` 并继续留在队列等待证据；回收站改为正常、仅回收站、全部三种真实数据范围。
- Asset 高级查询、排序、表格字段和 CSV 导出与 `assets`、`project_assets` 及 Strix 关联字段对齐，补充项目名、资产键、备注、最后存活、项目发现时间、任务 ID、回收时间、送扫时间和扫描次数。
- Strix 重扫继续使用当前任务 ID，并新增 `sentinel_scan_attempts` 尝试账本。每次执行独立保存阶段、检查点、停止原因、运行目录、开始/结束时间及本次新增请求和 Token，任务级累计成本保持不变。
- Strix 结果概要新增接口证据链，把静态/运行时接口、参数、HTTP 验证、机会评分和漏洞关联合并到同一视图；扫描后的学习质量门禁不再被解释为扫描本体失败。

## 1.0.13 - 2026-08-08

- 普通服务端 Web（含 401/403 的受保护页面）默认至少进入 `standard`，把主要 Token 留给可复现漏洞验证；强证据仍可升级 `deep`。
- 增加目录发现运行时熔断：普通 Web 最多允许一次有界词表工具调用；第二次检测到 `ffuf`、`dirsearch`、`gobuster`、`feroxbuster` 或 `wfuzz` 时立即停止当前 URL，禁止反复目录爆破。
- 复杂前端继续保持 `manual_review`，不会触发目录发现或任何 Strix Agent 调用；内置前端 Skill 与运行时提示同步更新。
- 为大型 `sec_skills` 方法包增加单次任务注入预算：完整原文继续保存在本地 Skill 中，每次 Agent 请求只注入有界章节摘要，避免约 752KB 方法包显著增加云端或本地模型 Token，并提示按任务显式选择更具体的 Skill。
- 新增 401 受保护传统页面、目录发现工具识别和大型 Skill 上下文压缩测试；未新增数据库表、字段或参数实例结构。

## 1.0.12 - 2026-08-08

- 调整 Strix Web 自适应路由：普通服务端 Web 默认至少使用 standard，保留主要 Token 做有界漏洞挖掘，只有强证据才升级 deep。
- 复杂前端框架改为 `manual_review`：完成本地 API、SourceMap、鉴权、业务入口和敏感线索提取后，不再启动 Strix Agent，避免在 SPA/前后端分离应用上反复消耗云端 Token。
- 扫描目标列表新增“复杂前端·人工复核”状态和高亮样式；重扫、恢复和状态修复逻辑会把该状态视为已处理目标。
- 不新增数据库表，也不修改 `config_profiles.settings_json`、`sentinel_scans` 或参数实例结构；仅复用已有文本状态字段记录 `manual_review`。

## 1.0.11 - 2026-08-08

- 沉淀候选和知识转换现在会同时对比已有 Skill 与高质量知识目录：重复内容合并到已有 Skill，只有具备独立复用价值时才新建，保留本地规范化重复检查作为最后一道门。
- 新增“用最新知识精炼”入口。用户可在 Skill 卡片上受控触发云端或本地 OpenAI-compatible 模型精炼；内置 Skill 不会被覆盖，而是生成增强副本。
- LLM 分析页将聚合、强制刷新、导入/导出知识等低频操作收进“更多工具”，默认只保留来源提炼和刷新，减少顶部拥挤。
- Strix 顶部不再常驻“同步结果 / 导出项目 / 导入项目”；后台同步继续工作，项目导入导出保留在有明确上下文的项目操作区。
- 外部 HTML 导入会保留常见 `<pre><code>` 代码块和图片 `alt`/说明文本，避免正文抽取把关键审计线索压成一行；动态脚本仍按静态来源处理，不伪造浏览器执行结果。
- 修正发布说明日期为 2026-08-08；Strix 模块页面改用当前子菜单标题，移除内容区重复的品牌标题与扫描记录前缀。
- 重做 LLM 分析顶部来源工具栏：输入框、缓存提炼、重新分析和更多工具改为独立控件与分层布局，统一高度、圆角、间距和焦点状态。
- 本版本未新增数据库表，也未修改 `config_profiles.settings_json`、`sentinel_scans` 或参数实例结构；继续复用现有 `strix_skills`、`strix_knowledge_entries` 和 `strix_learning_candidates`。

## 1.0.10 - 2026-08-08

- 将 Strix Web、代码审计、灰盒联测和 CI/CD 合并为统一的 Strix 扫描入口，并在页面内提供扫描类型切换；移除会压制正文展示的超大 `AI SECURITY OPERATIONS` 标题。
- 将本地 `sec_skills` 目录完整导入为内部 Skill 包，不再按关键词截断文本；Skill 保存在 Oviraptor 内并自动启用，供公司授权资产自查使用。
- 支持导入任意公开安全文章，以及 Safari/Chrome 保存的 HTML 文件。HTML 会先抽取可读正文再交给 LLM；原先 2MB 限制改为 16MB 原始文件限制，并在送入模型前做上下文截断。
- 增加生成阶段的质量门禁：`needs_verification`、`no_learning_value` 及其他未达到可复用标准的候选不会落库；LLM 分析页支持删除历史候选。
- 优化 LLM 分析页面布局，并修复自定义 Skill 删除按钮样式。

## 1.0.9 - 2026-08-08

- Added a learning quality gate for Strix post-scan refinement: ordinary probes, banner/version-only CVEs, duplicate tool loops, and findings without reproducible evidence are classified as `no_learning_value` or `needs_verification` and cannot be applied as Skills.
- Added CVE signal classification (`confirmed`, `needs_verification`, `dependency_signal`, `info`) and exposed the gate result in the LLM Analysis UI.
- Added audited `sec_skills` import that keeps methodology, fingerprint, evidence, and stop-condition guidance while filtering credential access, reverse shells, encoded execution, exfiltration, metadata, and bypass instructions. Imported content is disabled until review.
- Added public-source knowledge ingestion for HackerOne, Medium, security advisories, and local Markdown. Sources are content-hash cached in the existing knowledge store; later scans use cached method-card indexes instead of fetching each time.
- Added UI controls for source caching/distillation and force refresh.

## 1.0.8 - 2026-08-08

- Added a scan-to-learning loop for Web, code, grey-box, and CI/CD Strix scans. When a scan finishes, the active cloud or local OpenAI-compatible model now produces a reviewable candidate containing new ideas, redundant or weak steps, external knowledge requests, and a proposed Skill patch. Applying an accepted candidate performs a second model refinement against the selected Skill before the final Markdown merge.
- Added explicit `pending` / `accepted` / `rejected` / `applied` candidate lifecycle. Rejected candidates are excluded from future scans; nothing is persisted as a Skill until the user accepts and applies it.
- Added automatic inheritance of every enabled Skill for Asset/Workbench scans when no explicit Skill list is supplied. The exact injected Skill names remain recorded on the scan for reproducibility.
- Added Markdown-aware Skill patch merging. Replacements and removals operate on named sections, additions are deduplicated, and built-in Skills are cloned into an enhanced copy instead of being overwritten.
- Added local-model compatibility retry: if an OpenAI-compatible endpoint rejects `response_format` or returns non-JSON content, Oviraptor retries once without that optional parameter before falling back to a safe manual-review candidate.
- Added formatted Skill preview and copy-to-edit workflow in the Workbench, plus learning-candidate review controls in LLM Analysis.
- Added the `strix_learning_candidates` table and indexes. Existing `config_profiles.settings_json` and `sentinel_scans` columns are unchanged.

## 1.0.7 - 2026-08-08

- Hardened Strix result synchronization across the 1.3.x–1.5.x artifact formats: JSON envelopes (`vulnerabilities`, `findings`, `results`, `items`), SARIF, CSV, and per-vulnerability Markdown are now supported with deterministic fallback ordering.
- Preserved Strix-native fields while normalizing newer aliases for targets, remediation, PoC, titles, severities, rule IDs, and evidence locations; SARIF fallback findings are stored as `strix` findings instead of being misclassified as `local-sast`.
- Added compatibility for `succeeded`/`success`/`done` run states, `.state/events.jsonl`, `events.ndjson`, event directories, and camelCase or numeric-string token usage fields including cached input tokens.
- Added regression tests for JSON envelopes, SARIF/CSV/Markdown fallbacks, new completion states, and token aliases. No database schema or persisted parameter structure changed.

## 1.0.6 - 2026-08-03

- Replaced framework-count scoring with a hard evidence gate: Vue/React/Angular and bundle/route volume no longer trigger Strix by themselves. Modern frontend scans now require a verified or robust high-confidence API, registration/business form, SourceMap, or high-severity sensitive clue.
- Added a compact `verificationPlan` to every frontend evidence packet. Framework applications permit only a few prioritized API/business candidates, require a fresh request against the primary candidate first, limit each candidate to two attempts, and explicitly forbid broad crawling, bundle enumeration, and duplicate framework discovery.
- Added strict modern-frontend time, request, uncached-token, and cumulative-context ceilings. Local full-power mode can still expand ordinary Web scans but can no longer bypass targeted frontend limits; repeated tools and two no-progress model calls stop the current URL.
- Preserved bounded Strix-native discovery for ordinary server-rendered Web pages and kept Strix's vulnerability verification, evidence, PoC, CVSS/CWE, and remediation workflow unchanged.

## 1.0.5 - 2026-08-03

- Fixed jQuery `$.ajax({ type: "POST" })` method extraction and retained high-confidence same-origin state-changing API candidates when a deliberately safe GET probe is inconclusive.
- Prevented the static-framework guard from overriding standard/deep routing, included unresolved API candidates and login/auth flows in scoring, and passed those candidates into the compact Strix evidence packet.
- Added an explicit recon-only task completion state so a pipeline that never launched Strix no longer appears as a normal completed Strix scan; existing all-recon-only tasks are repaired on startup.
- Tightened IP detection with semantic network context, dependency/version-banner filtering, and matching UI-side filtering for old records. Added high-confidence GitHub, GitLab, Slack, Stripe, SendGrid, npm, bearer-token, and database-password rules.
- Moved remote Worker health, environment, task, control, and synchronization network I/O off the Tauri event thread. Worker imports now run sequentially to avoid SQLite and memory bursts.
- Unmounted the large Strix result tree whenever it is not visible, reducing WebKit restore/repaint pressure after macOS desktop switching and preventing hidden scan data from accumulating in the active DOM.

## 1.0.4 - 2026-08-03

- Removed Oviraptor's per-local-model context-window and maximum-output overrides. MLX now remains the sole owner of context size and inference limits.
- Removed the `STRIX_CONTEXT_*` and `STRIX_TOOL_OUTPUT_MAX_*` environment overrides and stopped rewriting local model request token limits.
- Migrates existing model profiles by deleting the retired `contextWindow` and `maxOutputTokens` fields without changing model names, endpoints, credentials, or active-profile selection.
- Prevented local Strix inference from overlapping with Chrome/Node reconnaissance for the next URL, while retaining adaptive budgets and no-progress fuses.
- Restored automatic Strix task-list refresh when returning from Asset after creating a scan; result artifact synchronization is no longer required to see new drafts.
- Changed the Asset probe-result advanced filter to selectable localized statuses instead of requiring internal enum values.

## 0.9.8 - 2026-07-25

- Added a configurable Strix frontend packet budget. Evidence JSON and supplementary code slices now share one limit, while URLs, status, API methods and parameters, routes, sensitive clues, and runtime signals are retained by priority.
- Added compact and custom packet strategies, with an 8K-context recommendation of 4-6 KB and a 32K/40K recommendation of 12-24 KB. Full reconnaissance remains stored locally.
- Added per-local-model context-window and maximum-output settings. The local LLM hook now clamps outgoing `max_tokens`/`max_completion_tokens`; the configured context window also caps the frontend packet budget and records a diagnostic policy.
- Simplified the in-app release information to four plain-text entries without decorative icons.

## 0.9.7 - 2026-07-25

- Made pause immediate across frontend reconnaissance, browser descendants, Strix, and Docker on Windows, macOS, and Linux, while preserving queued URLs for resume.
- Local full-power mode now runs without token, request-count, active-duration, repeated-tool, or no-progress limits; startup, model-interface, and context-window failures remain protected.
- Removing a URL from the fuse zone now starts a targeted retry that reuses its saved frontend reconnaissance instead of repeating completed work.
- Added red fuse and vulnerability counters, vulnerable-URL filtering, high-value URL markers, and sensitive-information counts beside JS/API summaries.

## 0.9.6 - 2026-07-25

- Made pause immediate and cooperative across frontend reconnaissance, Strix, Docker, and browser child processes on Windows, macOS, and Linux; queued URLs remain untouched for resume.
- Local full-power mode now removes token, request-count, active-duration, repeated-tool, and no-progress limits while retaining startup, model-interface, and context-window protections.
- Raised the governed cloud no-progress default from two to four consecutive model turns.
- Context-window failures now enter the fuse zone with an explicit reason; other local full-power progress limits no longer create fuse entries.
- Removing a URL from the fuse zone now creates and starts a targeted retry task that reuses the saved frontend reconnaissance checkpoint.
- Added red pending counters for the fuse and vulnerability zones. Vulnerability-zone navigation filters tasks and URLs to those with vulnerability findings, and each completed review decrements the counter.
- Added high-value URL markers and sensitive-information counts beside JS/API and URL summaries.

## 0.9.5 - 2026-07-25

- Added a live Strix runner log panel to the Sentinel operation log page, with two-second polling, scan selection, tail limiting, ANSI removal, and automatic scrolling.
- Added a Tauri command for reading sanitized runner log tails from the selected scan.
- Pausing a Sentinel scan now sends stop signals to registered URL, Strix, and Docker processes and records the action in the runner log.
- Added per-URL pipeline and pause checkpoint messages so a stalled target can be diagnosed without guessing.
- Prevented failed or context-overflow LLM requests from being counted as successful requests or estimated tokens.
- Updated local and cloud scan diagnostics for cross-platform startup, timeout, and model-interface failures.
