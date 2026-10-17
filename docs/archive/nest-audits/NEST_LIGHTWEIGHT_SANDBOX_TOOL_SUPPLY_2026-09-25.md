# Oviraptor 轻量沙箱与 Agent 工具供应实施合同（2026-09-25）

> 状态：实施设计，**不是当前已完成的安全隔离声明**。执行顺序以 `NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md` 为准；本文件细化蓝图 §5.1。Qoder 不得仅添加 Docker/Kali 启动按钮就把本项标完成。

## 1. 明确决策

默认不用 Kali VM，也不在扫描进行中联网拉工具。将工具分成三个层次：

1. **内建 Broker**：现有 `inspect_evidence`、`replay_http`、`compare_identities`、`targeted_discovery`、`browser_action`、`record_hypothesis_result`、`finish_target` 保持受控入口；模型不得得到通用 shell、任意 URL 抓取或容器守护进程权限。
2. **发布时固定的轻量 worker**：仅在内建 API 无法完成解析、浏览器或静态分析时使用。最小工具随应用发布，或由管理员在任务之外导入有签名/摘要的离线包；单个 worker 故障只影响自己的能力。
3. **可选兼容环境**：Kali/其他 VM 仅用于需要特殊工具且明确获批的任务，不是扫描成功的前置条件；仍逐请求经过 Oviraptor Broker。若该工具不能遵守 Broker 合同，则不得接入主动扫描。

### 给实施者的最终选型（不要让每个 Agent 都装一套工具）

Agent 是角色和权限，不是各自一台完整 Kali。Coordinator/Reviewer/Deep Investigator 只需应用内的数据与模型接口；WebExecutor 使用同一受控 HTTP Broker；仅浏览器、AST、特殊解析器等无法在 Rust 内安全完成的能力，才按**能力**启动隔离 worker。多个 Agent 可以复用相同的只读工具包文件，但进程、临时目录、身份、assignment、预算与审计必须逐 run 隔离。共享缓存绝不等于共享会话或权限。

一次任务的能力解析必须是纯本地、可复现的决策：`内建能力 → 本机已批准且 digest/平台/适配器自检匹配的固定包 → 对应支线标缺失`。任务里没有“安装后继续”、`npm install`、`apt`、`brew`、`docker pull` 或跟随网页建议安装工具的分支。Agent 只能申请能力，不能选择下载 URL、签发者、版本、镜像源或解除隔离。任务启动前把实际选中的包版本、摘要、适配器、能力缺口冻结到 attempt；恢复时再次复验，变更版本必须新 attempt 并告知操作者。

落地时由**发布/运维流水线**准备一小套基础能力：Rust Broker 和只读模型/数据库 API 随应用；静态解析器及必要运行时随应用固定版本交付；浏览器是独立的大包，可预置于受控内网或离线介质，但只有对应平台的沙箱和所有子请求 Broker 均通过验收后才显示可用。额外工具只按某条被批准的能力申请在任务外导入；包不是 Agent 私有副本，工具缓存可共享，执行现场不可共享。1 GiB 候选包缓存只解决“完整且未改动地存放”，**不等于安装、批准或可执行**。

例如：普通 HTTP 任务即使完全断网、没有 Kali/浏览器，仍可使用内建 Broker；需要 DOM 的任务在浏览器未获批准/未隔离时只报告 DOM 缺口，不能偷偷调用宿主 Chrome；需要特殊二进制的任务先由人工评估是否值得准备其最小包。若工具原生要求 raw socket、任意外连或自行重放目标请求，不能通过 Oviraptor 的逐请求 Broker，则即使 Kali 能运行它也只允许离线分析已保存的证据，不接入主动目标 lane。

具体供应流程：新建任务只做能力清单预检，Coordinator 可以提出 `capability_request`（工具 ID、用途、预期 Broker 权限、缺失后覆盖损失），**不能触发下载或审批**。管理员在任务外从内部批准仓库或离线介质取固定版本，检查签名、摘要、许可证和平台，审查能力与隔离适配器后批准；发布缓存的身份为 `id + version + platform + digest`。下次任务从本地批准清单选择，不按 Agent 的描述、网页提示或 `latest` 动态解析。已在扫描中的 attempt 不热替换工具；升级产生新版本、新审计和明确的续跑决策。允许管理员准备阶段使用受控内网镜像，但离线环境不以公网为依赖，**任务执行阶段的在线拉取一律拒绝**。

候选缓存当前已按上述四元组生成带域分隔的稳定目录键；路径不直接使用不可信版本/平台字符串。同包摘要的两个不同签名版本不再互相覆盖。旧的 `id/digest` 候选目录不删除，也不自动赋予批准/执行资格；管理员后续需按新身份重新离线导入。这个修复没有实现审批注册表或 worker 隔离。

能力缺失按支线处理：没有浏览器则标注 DOM/真实交互覆盖缺口，保留 Broker HTTP；没有 AST 解析器则保留原始证据并记录静态分析缺口；没有特殊工具则只生成申请，不能退化成 Agent 自行调用宿主 shell。Reviewer 不因缺工具而假称覆盖完整，Coordinator 可暂停受影响 assignment 并允许用户在准备完成后发起新的有界 attempt。硬性授权/预算停止不得因其他 Agent 同意而解除。

### 按角色发能力，而不是给每个 Agent 装系统

| 角色/支线 | 默认可获得的能力 | 可选能力及执行位置 | 绝不直接授予 |
| --- | --- | --- | --- |
| Coordinator | 任务状态、授权范围摘要、能力缺口和 mailbox；不发送目标请求 | 无；只提出有界能力申请并等待新 attempt | shell、安装器、目标网络、审批权 |
| Reviewer / Deep Investigator | 已封存证据、只读差异与受约束的分析消息 | 已保存内容的离线解析 worker；不得自行追加目标请求 | 目标凭据、宿主浏览器、动态下载 |
| WebExecutor | 同一 Rust HTTP Broker 的目标请求租约和证据落盘 | 经批准的浏览器 worker；浏览器**全部**目标请求必须经过 Broker，且另有网络出口阻断 | 原始 socket、直接 `curl`、无隔离 Chrome |
| Code/CI Analyzer | 固定源码 snapshot 的只读输入 | 固定摘要的 AST/规则包/单工具 worker，仅在独立沙箱内执行 | 宿主 repo 写入、任意 `npm`/`pip` 安装、目标网络 |

首发按三档安装包验收：`Core` 只包含应用/Broker/只读分析能力，断网可完成纯 HTTP 任务；`Browser` 增加平台对应、已批准的固定浏览器包和通过出站测试的适配器；`Code` 增加固定规则包与源码分析 worker。它们不是三个权限等级，也不自动授予角色任何额外授权。Kali/其他 VM 是第四种**显式审批**的兼容运行面，不列入默认安装或成功条件。任何档次都不得在扫描启动或扫描恢复时以缺工具为理由拉取网络资源。

运维操作顺序必须是：发布/管理员在**任务外**取得固定版本 → 验来源、签名、摘要、许可、平台和适配器 → 独立审批并可撤销 → 创建 attempt 时本地解析并冻结每个 assignment 的能力、包摘要和缺口 → child run 按租约使用 → 恢复时复验。管理员可以在准备阶段从获准内网仓库取包，也可以从离线介质导入；两者走同一验证与审批过程。没有批准源或断网时可继续运行 `Core`，而不是等待在线安装。用户在聊天中说“装个工具”只产生待审批申请，不等同系统级安装授权。

目前 `WorkerSettingsPanel.vue` 的手动说明仍把 Docker Desktop、Node 等描述为普遍前置条件，`App.vue` 的环境页也仍提供通用依赖安装；这是旧运行环境 UI，不能用它证明上述档次已经上线。迁移时把“全局依赖是否存在”替换为“本任务实际能力是否可用”，将历史 Worker/远程节点配置与 Native 沙箱能力分别展示。过渡期的安装入口必须继续受任务外租约互斥约束，不能由 Agent 或聊天调用，也不能把宿主安装的 Node/Chrome 误标为已批准包。

| 场景 | 推荐执行面 | 工具取得方式 | 缺失时的结果 |
| --- | --- | --- | --- |
| 默认只读 HTTP、证据检索、审查、协作 | 应用内 Rust Broker/数据库 | 随应用发布，不需要 Kali/Node | 内建能力异常就停止相关请求，不能换宿主 curl |
| JS AST/静态源码分析 | 固定摘要的只读 parser/worker | 随发布包；额外语言解析器可在任务外离线导入 | 该分析支线不覆盖；不能在运行中 `npm install` |
| DOM/真实浏览器动作 | 固定浏览器二进制 + 隔离适配器 + 全请求 Broker | 发布时打包或任务外由管理员导入并验证 | `unsupported_sandbox:browser`，HTTP 仍可工作；不使用宿主 Chrome 兜底 |
| 特殊安全工具 | 最小的单工具能力包 | 管理员按平台/版本从批准源**准备阶段**预置，固定 digest | 提交能力申请/人工复核，不临时拉镜像 |
| 某些只能在 Kali 运行的工具 | 显式可选的 Kali 兼容 lane | 任务外预置固定镜像摘要，单独审批 | 不阻塞普通 Native 任务；无法 Broker 化的目标访问工具不接入 |

只有**任务外**且安装租约已独占、没有活跃/排队 attempt 时，管理员才能从批准源下载或选择本地离线包。执行时只能查验本地已批准的固定 digest；无缓存、校验失败或平台不匹配都 fail closed。离线内部环境直接通过离线文件/内部镜像仓库准备，不把外网当运行依赖。导入需要同时保存供应来源、发布者签名或经明确定义的内部签发链、许可证、SBOM/组件清单、平台架构、摘要、批准人、批准/撤销时间；单独的 SHA-256 只证明文件未变，不能证明来源可信。

当前 `src-tauri/src/commands/agent_tools_dispatch.rs` 是 Native 工具分派；`native_helpers.rs` 会选择宿主 Node，`9_frontend_runtime_probe.cjs` 会找宿主浏览器；`environment.rs::install_environment_dependencies` 是 UI 手动调用的联网安装入口。`native_pipeline/analyzer.rs` 已为**配置了固定镜像**的源码分析器提供 `--pull=never`、无网络、只读源码挂载等部分约束，并记录实际执行的标志；宿主二进制模式不声称同样隔离。现有机制还不构成所有 Agent 工具通用的沙箱/锁定能力包，迁移时须保留旧任务结果兼容，不能声称已有的 Node/Chrome 执行符合本合同。

候选缓存复用额外拒绝非 `manifest.json`/`package.blob` 的目录项，以及硬链接包文件，避免把缓存附带物误当未来入口或经外部路径修改。该校验仍不能消除路径祖先/目录交换的 TOCTOU 风险；批准和执行阶段必须用目录句柄锚定、重新验签验摘要，并验证实际启动的不可变文件身份。当前仍无执行入口。

注意 `src-tauri/resources/capability_bundles/*.json` 目前是**角色配置草稿可选择的能力目录**，不是下文所述的可执行工具包、签名清单或沙箱证明。不得把它们的存在当作工具已经安装/隔离的证据；后续供应链清单应与角色能力目录分表、分版本管理。

当前增量：发布版本编译时固定了浏览器 worker、AST worker 和 Babel parser 的字节摘要；资源定位及正式执行前校验随包文件，缺失、篡改或未注册的 worker 拒绝运行。AST 不再回退读取宿主机的 `@babel/parser`。这只证明应用内 JS 资源与编译版本一致；宿主 Node/Chrome 的二进制与运行环境尚未固定，校验和启动之间仍有 TOCTOU 窗口，进程/网络沙箱和外部包签名尚未实施。不得把此增量描述为完整能力包供应链或 Kali 替代沙箱。

2026-09-25 边界增量：多智能体 WebExecutor 的 `browser_action` 不再把目标交给尚未隔离的宿主浏览器；有已定位的只读 URL 时只走单次 Rust HTTP Broker，并标记 `browserDriven=false`、`browserAttempt=unsupported_sandbox:browser_network_broker_unavailable`，不声称执行 DOM 动作或产生真实网络增量。旧的前端预侦察和单 Agent/测试浏览器路径仍使用宿主运行时，**尚不能视为全产品浏览器沙箱已完成**；必须迁移这些路径并验收浏览器的全部子请求之后才能恢复多智能体浏览器动作。

## 2. 工具放哪、何时取得

| 能力 | 默认供应 | 执行地点 | 断网/缺失处理 |
| --- | --- | --- | --- |
| 协调、证据阅读、独立 Reviewer、知识检索 | 应用内 Rust，随版本交付 | 无目标网络能力的数据 API | 数据缺失列 EvidenceGap，不安装新工具 |
| 目标 HTTP 和身份对照 | 内建 HTTP Broker | Rust 进程；每次请求检查授权、归属、速率与次数 | 停止对应调用，保留审计和可恢复条件 |
| 前端 JS/DOM/浏览器行为 | 固定版本的 Node helper + 浏览器能力包 | 独立低权限 worker/隔离的浏览器 profile | 先保留 HTTP/静态证据；标 `unsupported_capability:browser`，不得用宿主 shell 兜底 |
| 源码和依赖静态分析 | 只读快照 + 签名的解析器包 | 无网络 worker；仅写入任务临时输出 | 单独列未覆盖分析项，不执行构建脚本 |
| 额外专家工具 | 管理员审批的特定工具包 | 该工具专属的隔离 lane | 生成能力申请，未获批则不调度 |
| Kali VM | 无默认供应 | 用户显式选择的独立兼容 lane | 不影响其余 Native 任务 |

发布包应附固定 manifest 和必要工具的实际二进制/worker；不能把开发机上的 `/opt/homebrew/bin/node`、系统 Chrome 或 PATH 命中误记为可复现能力。应用升级可带来新的 manifest，但运行中的 attempt 固定原版本。可选能力包只有管理员在**准备阶段**选择导入本地文件或从批准源获取；校验摘要、来源签名、许可证与系统兼容性后进入只读缓存。扫描阶段只查缓存，缺失即拒绝，不触发 `brew`、`winget`、`apt`、`pip`、`npm`、`curl | sh` 或运行时镜像拉取。当前环境页的手动安装命令仍可在准备阶段通过 `brew`/`winget` 联网取包，**不符合最终能力包供应链**；但安装与新任务入队的数据库互斥已落地，仍需将该旧入口替换为受控的任务外包准备流程。

这回答“网上拉还是预装”：**扫描时绝不拉取**。发布的最小集合应包含 Rust Broker 与受控 HTTP 能力；浏览器和重型解析器若因许可、体积或平台原因不能随包交付，则在管理员准备阶段从批准源下载并固定摘要，或者在内网导入同一摘要的离线包。两条路径产出相同的已验证缓存状态；首次离线启动只有已随包或已导入的能力可用。当前工作树没有实现这条完整供应链，不能把现有环境页的一键 `brew`/`winget` 安装视为等价实现。

### 2.1 按任务选择执行 profile，不按 Agent 安装系统

| Profile | 适用任务 | 所需工具 | 不适合时怎么办 |
| --- | --- | --- | --- |
| `core`（默认） | URL 入口、只读 HTTP、身份对照、协作、Reviewer、历史结果 | 应用内 Rust Broker 和数据层；无 Kali/浏览器/容器依赖 | 某个动态页面不可观察就列浏览器覆盖缺口，不伪造“已扫描完整” |
| `browser`（可选） | SPA 的真实 DOM 状态、点击、XHR/fetch、账号隔离 | 固定浏览器包、逐请求 Broker、独立网络出口限制和每 run 私有 profile | 适配器未通过子请求逃逸验收时禁用真实动作；保留 `core` 可做的只读部分 |
| `source`（按需） | 已获授权的源码/CI/Greybox | 冻结只读源码快照、批准的静态解析器/规则包、无网络 worker | 镜像或规则包缺失时只报告该分析支线未覆盖，不改用 PATH 命中的宿主工具 |
| `compat-kali`（例外） | 特定工具只有该环境可运行，且审批明确允许 | 固定摘要的最小 VM/镜像和该单工具；仍需受控目标出口 | 无法把目标访问纳入 Broker、无法隔离或资源成本不合适，就不启用该 lane |

上述 profile 是**工具运行面**而不是目标授权。`compat-kali` 不隐含主机测试权限；目前不存在 `host` profile、SSH 工具、通用命令执行能力或 Windows 主机执行器。Web 测试的默认目标面是 Master Plan §6.4 的 `web_only`，不影响另行授权的 Code/CI/Greybox 任务。通过 Web 参数导致远端 OS 执行也必须被主机边界拒绝，不能借浏览器或 HTTP Broker 走私。日后若单独批准 Linux HostVerifier，它须与 Web worker 隔离目标、凭据、进程、预算和租约；HostVerifier 只能接收白名单的结构化操作及精确主机合同，不能得到任意 shell。当前不要预装这种工具或宣称开关已能启用它。

一个任务可组合多个 profile，但 `core` 不因 `browser`/`source` 缺失而整体变成“失败”或假装完成。Coordinator 只读取能力状态并签发最小 assignment；Mapper、Reviewer、Deep Investigator 不需要 Kali。能力包文件可被多个任务只读复用，**执行实例不可复用**：每个 run 的临时目录、浏览器 profile、身份句柄、令牌、预算、取消信号和审计链必须分别创建并在终态后受控清理。绝不让一个 Agent 自己调用 `npm/pip/apt/brew` 安装工具，也不把用户聊天中的“装个工具”当作管理员批准。

准备阶段实行 `download/import → staging → 校验来源签名与摘要/许可/SBOM/平台 → 管理员批准 → 原子发布只读缓存 → 能力自检`；任何一步失败仅留下可审计的失败记录，不改变正在运行 attempt 固定的工具版本。任务创建时冻结所需 `tool_id@digest`；启动前再次核验，缺失/撤销/不兼容就只关闭相关 lane 并给出准确缺口。扫描期间断网、缓存未命中或镜像不存在均不得联网补装。Kali 只是第四种显式 profile，既不负责默认的 HTTP Broker，也不能成为逃过目标授权检查的后门。

### 2.2 复用边界、生命周期与启动成本（必须按此实现）

“同一次任务的 Agent 共享沙箱”只在**共享已批准的只读包、Broker 服务和封存证据**的意义上成立；不得共享可写文件系统、浏览器 profile、进程、身份 cookie、临时凭据或能力租约。`scan → attempt → assignment → child_run → tool_session` 是所有执行实例的归属链。一个 child run 可以在有效租约内连续使用同一私有 worker，以免每个工具调用都冷启动；另一个 child run，即使属于同一任务或由同一 Agent 执行，也不能直接接管该 worker。Agent 之间通过持久 mailbox/证据引用交接，不传容器文件路径或进程句柄。

| 层 | 可否跨任务复用 | 生命周期及清理 |
| --- | --- | --- |
| 应用/Broker、固定 digest 的包与镜像层、受信规则及只读缓存 | 可以；升级或撤销有独立 epoch | 任务外发布，任务开始/恢复均复验；不能缓存凭据、会话、网页内容 |
| 调度器的能力清单与冷启动模板 | 只可复用**无身份、无目标连接**的不可变模板 | 可预加载包/验证适配器，不能预先打开跨任务共用的浏览器 profile 或带网 worker |
| 每个 child run 的进程/容器/轻量 VM、临时输出、租约和审计 | 不可以跨 child run/任务复用 | 首次需要该能力时惰性创建；同一 child run 多次调用可保持温热；完成、取消、租约失效或超时后终止子进程并验证清理收据 |
| 浏览器实例、profile 和身份状态 | 不可以；不同身份也不能复用 profile | 按 child run 且按身份隔离；同一 run 的连续动作可保留会话。封存必要证据后删除 profile；凭据继续只由身份 Broker 保管 |
| 源码/文件输入与输出 | 输入只读引用可由授权 child run 复用；可写输出不共享 | 从已冻结 snapshot 或已封存 artifact 精确映射到该 worker；输出先限大小/类型与校验，再由 Broker 发布为新 artifact；不挂载宿主项目根 |

暂停/崩溃不靠“保留活容器”恢复：记录 attempt 的固定能力计划、证据、已完成调用与未知结果，回收或隔离遗留进程；恢复时重新自检相同 digest/适配器并创建新的 child run 执行实例。不能无痕升级工具、继续使用旧浏览器 profile 或自动重放结果未知的目标请求。清理失败应显示 `cleanup_failed` 并阻止该执行槽再次复用，不能仅把进程从 UI 隐藏。工具包仍留在只读缓存，所以下次任务只新建轻量执行现场，**不重新下载/安装整套工具**。

浏览器自动化、代码执行与文件处理必须拆开声明能力。浏览器 lane 要有已批准浏览器包、逐请求 Broker 和独立网络出口阻断；缺一个就不做真实 DOM/点击，保留可用的 HTTP/静态证据。固定 AST/规则包的源码分析只读取冻结文件，不运行构建脚本；模型生成的任意代码/命令是**另一个默认禁用的高风险能力**，不得因为 AST worker 已可用就启用，若未来引入必须另有显式授权、无目标网络的隔离执行和输入/输出限制。文件解析只接收 Broker 指定的有界 artifact，不接受模型给出的宿主绝对路径、归档穿越或符号链接；不可信压缩包解包需要单独的限额和格式审查。

新建任务先做纯本地能力预检，展示每条 lane 的 `ready / degraded / missing / incompatible / revoked / unsupported_sandbox` 和具体原因、覆盖损失。必需能力缺失时该 lane 不启动，并允许用户选择等待管理员准备或明确以降级覆盖创建新 attempt；可选能力缺失时其余 lane 照常执行。若能力在运行中撤销/损坏，停止受影响 child，保留证据和缺口；修复后由管理员准备，再以**新 attempt**复核，不在旧执行实例热安装。Reviewer 必须在总结里区别“未观察到问题”和“没有浏览器/代码能力而未覆盖”。

性能策略是 `预检与摘要校验 → 惰性启动所需 worker → child run 内复用 → 终态清理`，不是“一任务启动完整 Kali”，也不是“每个请求建一个容器”。允许在任务外预热**不含身份和网络**的只读镜像/包缓存；首版不设跨任务浏览器进程池。调度按实际内存/CPU限制并发，有资源不足时排队且显示等待原因；排队/冷启动与 Agent 有效工作分别计时，但仍受独立的总等待超时和整体资源预算限制。记录每个能力的排队、适配器自检、冷启动、首个可用动作、温态调用、内存峰值和清理耗时的 P50/P95；基于真实三平台测试再定产品阈值，不能编造统一的“毫秒启动”承诺。UI 以阶段进度说明等待，不把启动慢误判为 Agent 无进展或悄悄延长硬预算。

生命周期验收至少覆盖：两个并行 Agent 同任务互不可见写区/凭据；同一 child run 连续工具调用不反复冷启动；两个任务命中同一个只读 digest 但绝不复用进程/profile；暂停/取消/崩溃后无孤儿进程、未知请求不重放；缺浏览器、缺规则包、包撤销、适配器失败和缓存篡改时准确降级；大型文件/压缩包限额；冷热启动与资源排队均有独立计时和清理收据。未通过这些测试前，不得在 UI 宣称“共享沙箱已安全复用”。

## 3. 能力包清单与调用合同

每一能力包须有机器可验证的 `ToolCapabilityManifest`。下面是**签名信封**格式；`payload` 内的字段（包括 `digest`）必须一起验签，不能只验证包摘要：

```json
{
  "payload": {
    "schemaVersion": 1,
    "id": "browser-runtime",
    "version": "fixed-release-version",
    "platform": "target-os-target-arch",
    "source": "approved-publisher-and-artifact-id",
    "digest": "sha256:<64 lowercase hex digits>",
    "license": "recorded-license-id",
    "entrypoint": "bin/browser",
    "brokerCapabilities": ["browser_action"],
    "networkPolicy": "broker-only",
    "filesystemPolicy": "read-only-input-and-private-temp-output",
    "timeoutSeconds": 150,
    "memoryMiB": 768,
    "cpuLimit": 1,
    "processLimit": 32,
    "outputBytes": 2097152,
    "inputSchemaVersion": 1,
    "outputSchemaVersion": 1
  },
  "signerId": "approved-release-key-id",
  "signatureBase64": "detached-ed25519-signature"
}
```

数值只是格式示例，不能当已批准生产限额。当前 `tool_supply.rs` 已提供**离线候选包**的第一道验证：对 `OVIRAPTOR-TOOL-MANIFEST-V1\0 || serde_json(payload)` 使用外部传入的 Ed25519 公钥验签，检查撤销、平台、入口路径、允许能力/网络策略及 SHA-256。内部 `stage_offline_tool_candidate` 对最多 64 MiB 的内存字节包验签后独占任务外环境准备租约；`stage_offline_tool_candidate_from_file` 对本地文件以 64 KiB 块流式预验及落盘复验，硬上限 1 GiB。两者将不透明包放入私有临时目录，按 digest 原子改名并记录候选审计；缓存命中重新流式验签与摘要，损坏则保留租约待人工核对，活跃任务不得导入。大包文件路径拒绝末级 symlink/硬链接，拷贝过程中被替换会因第二遍摘要不符而失败；这仍**不保证**父目录无 TOCTOU，也不适合将不可信网络文件系统当缓存。**它们没有注册为 UI/Agent 命令**，调用者传入的公钥还没有独立管理员信任库来认证，因此候选缓存不是已批准的工具，也不能启动。尚缺管理员身份/审批/撤销持久化、安全归档解包、跨平台路径/并发 TOCTOU 加固、Node/浏览器二进制锁定与进程/网络沙箱；绝不能因为候选字节入缓存就显示“已安装/安全可执行”。后续导入须验证包内 symlink 不逃逸及 OS/架构；以内容摘要为存储键，拒绝滚动 `latest`。`tool_id + digest + schemaVersion` 写入每次 assignment/run/attempt 和工具审计，不只写 UI 当前版本。冻结 manifest 后跨版本续跑需显式迁移审查；原 attempt 不可无痕换包。缓存只读，证据原件与临时目录分离。

调用采用结构化消息而非 shell 字符串。Coordinator 只能派角色与能力，不能给自己补权限；Broker 检查 scan/attempt/target、scope、身份、方法、DNS 解析/IP、预算、取消 token 和 fencing，再向 worker 发最小输入。worker 只返回有大小上限的结构化结果和 artifact 引用。不能直接给容器目标网卡并声称已经经过 Broker；浏览器的跳转、子资源、XHR/fetch、WebSocket、下载和重定向均需同一策略，做不到时将浏览器能力判 `unsupported_sandbox`，不能暗中直连。

### 3.1 任务外准备与任务内解析的硬边界（交付时逐项验收）

工具供应拆成三种职责分离的接口，而非一个带 `installIfMissing` 参数的接口：

| 接口 | 调用者和时机 | 允许的输入 | 允许的副作用 | 必须拒绝 |
| --- | --- | --- | --- | --- |
| `prepare_tool_package` | 独立的管理员操作；无活跃任务 | 本地文件或管理员配置的固定源、预期 digest、受信签发者、许可审查结果、平台 | 暂存、验签、审查、安全解包、发布新版本及审计 | Agent/聊天发起，`latest`、动态 URL、活动 attempt 热替换 |
| `resolve_tool_plan` | 创建 attempt 前和恢复前 | 已批准 registry、冻结的需求、平台/适配器状态 | 只读决策及缺口，不做下载或执行 | 候选包充当批准包、PATH/宿主命中、缺失时自动安装 |
| `execute_tool` | 持有效 assignment 的 child run | 已冻结 digest、能力租约、最小结构化输入、Broker token | 在隔离实例执行并持久化有限输出/审计 | 包版本变化、租约撤销、任意 shell/网络、绕过 Broker |

注意 `src-tauri/src/agent_runtime/role_config.rs` 的静态 `CapabilityBundle` 只描述角色可申请的权限，**不是**安装状态；`src-tauri/src/tool_supply.rs` 的缓存只存尚未批准的不透明候选，**不是**可执行 registry。两者必须由管理员批准记录和适配器自检关联，不能只根据 `list_capability_bundles` 的返回值启用工具。建议批准记录的主键是 `(tool_id, version, platform, digest)`，保存独立信任根版本、签发者、许可/SBOM 审查、能力列表、适配器 ID/版本、批准人/时间和撤销 epoch；私钥不能随候选包、网页或模型输出提供。实际冻结记录须绑定 `(scan_id, attempt_number, child_run_id, assignment_id, capability_lease_id, tool_digest, adapter_version)`。重启恢复先复验注册表、撤销 epoch、包字节、隔离适配器和已有调用状态；任一变化停止该 lane，不能无痕换包或补发未知结果的目标请求。

解析顺序固定：

```text
for each required capability:
  if built_in_broker_implements(capability): select built_in
  else if approved_registry_has_exact_digest(capability, platform)
       and signed_package_reverified()
       and sandbox_adapter_probe_passes()
       and all_required_broker_paths_pass(): select pinned_worker
  else: record capability_gap(reason_code); disable only dependent lane
freeze(plan, digest, adapter, gaps, attempt) before any child executes
```

不能把“有 Docker/Kali/Node/Chrome”算作上述 `probe_passes`。浏览器 profile 额外要求封闭直连出口并逐类证明导航、重定向、子资源、XHR/fetch、WebSocket、下载、Service Worker 的目标流量均受 Broker 策略控制；只能拦截页面导航时继续 `unsupported_sandbox`。`source` profile 可在完全无网络的 worker 内消费只读源码快照；依赖工具若要执行安装脚本/构建脚本，需另签明确的构建能力和隔离合同，静态解析能力不隐含这些权限。

Qoder 的纵向切片顺序必须是：先做独立管理员信任根/批准记录与候选→已批准状态迁移，并测试伪签名、撤销、缓存篡改和旧库迁移；再做只读解析与冻结及缺口 UI；再做**一个**无网络静态解析 worker 的隔离执行/取消/资源上限；最后才接浏览器和可选 Kali。每个切片须有断网运行、重启、并发任务、失败降级和撤销回归。禁止在缺少可信根或隔离 adapter 时给已有候选包加一个“执行”按钮；不得为了功能演示恢复宿主 Chrome/Node 的多 Agent 目标访问。

## 4. 隔离适配器及失败语义

跨系统定义统一 `SandboxAdapter`（`probe`、`prepare`、`run`、`cancel`、`collect`），但不把任何 Linux-only 标志直接用于 macOS/Windows。Linux 可选择受约束的 rootless 容器或进程隔离；macOS/Windows 使用经验证的本地隔离/轻量 VM 适配器。适配器在实际启动前逐项报告：非 root、只读包、输入只读挂载、输出专用目录、无宿主工作区根挂载、进程/CPU/内存/时长/输出限额、禁止提权及网络阻断/代理。缺任一必需保证，返回 `unsupported_sandbox`，不能悄悄退回宿主 `Command::new`。

隔离并不授予目标授权；HTTP Broker 保留所有原有安全硬边界。软停机（无进展、能力缺失）显示下一步、未完成项和是否可恢复；硬停机（授权超界、拒绝、WAF/429、预算和证据落盘失败）不可由 Agent 互相投票取消。崩溃后未知结果的目标调用标记 `indeterminate`，人工确认前绝不重放。Kali lane 即使启动成功，也不得 privileged、host network、root 工作区挂载或拿到其他角色的身份凭据。

浏览器不能只在页面级拦截导航后就放行：必须验证子资源、重定向、XHR/fetch、WebSocket、下载及 Service Worker 均不会绕开 Broker。若采用 Playwright 路由，应在需要拦截的上下文阻断 Service Worker，并以独立的网络出口限制作为最后一道边界；缺少任一项就继续标记浏览器能力不可用。容器使用本地固定摘要并禁止扫描期拉取；rootless 也要检查平台的实际 cgroup 限额是否生效，不能只看启动参数。

## 5. UI 和实施顺序

“环境/能力”页展示 `内建 / 已验证 / 缺失 / 不兼容 / 待批准 / 已撤销`、版本、摘要、平台、来源、权限和最近自检；提供**任务外**的导入/审批/撤销。新建任务页按实际角色列必需能力及缺口，浏览器缺失不阻止可执行的只读 HTTP 任务，但应明确降低覆盖。任务细节显示每个 run 使用的 digest、隔离适配器和停止原因；聊天只解释状态，不能当授权或审批表单。

任务 UI 固定显示“执行范围：仅 Web”。疑似主机线索显示为待确认的覆盖缺口，附来源证据与明确的“未进行主机验证”，不混入漏洞确认或“扫描完成”覆盖率。“允许申请主机验证”即使日后提供，也默认关闭且只是申请入口：单独的授权表单与后端审批身份、有效期、精确目标、动作白名单及新 attempt 是执行前的必要条件；未实现这些条件时 UI 必须显示不可用，不能靠前端布尔值解锁工具。

Qoder 按如下关口交付，每个关口先写失败回归：

1. 盘点所有 `Command::new`、Node/Chrome、下载器、网络出口和 UI 安装调用；区分管理员安装、应用更新、任务执行。保留旧 JSON/SQLite 读取，不迁移原始证据内容。
2. 实现 manifest schema、签名/摘要验证、只读缓存、安装状态与审计；扫描路径禁止安装函数/网络拉取。现有安装入口已增加跨进程文件锁、SQLite 持久租约和触发器，安装认领/任务进入队列原子互斥；去掉了自动执行 Homebrew 远程脚本。租约若因崩溃遗留，须人工核对安装子进程并输入确认语，持锁期间拒绝恢复，恢复事件写入数据库。**这仍不是**分包供应链、管理员身份认证或 worker 沙箱。
3. 接入 `SandboxAdapter` 的本机能力探测和 fail-closed；优先改造浏览器 helper 和源分析 worker。没有可验证的网络隔离前，**不能**把现有浏览器 helper 称为 broker-only。
4. 将角色权限和每个工具调用绑定到 manifest digest、assignment、attempt 与 Broker 授权；提供 UI 状态与精准缺口，保持现有 Native HTTP 可用。
5. 验收断网首次启动、缓存损坏/摘要不符、签名撤销、二进制缺失、超时/OOM、容器/VM 崩溃、软硬预算、跨项目路径、DNS 重绑定、浏览器子请求逃逸、升级/续跑、人工取消与未知调用不重放。macOS/Linux/Windows 分别记录支持矩阵，不支持的系统拒绝该 lane。
6. 增加 Web→主机拒绝测试：默认任务中 Agent/聊天/恶意网页提出 SSH、OS 命令、主机文件或通过 Web 参数执行命令，只产生 `host_boundary_candidate`，不派发执行；切换前端开关或伪造审批请求不能取得后端租约；授权过期/撤销/目标变化拒绝；没有 Linux HostVerifier 实现时即使已有授权文件也只能保持待实施。Windows 主机能力始终标未支持。

任何一关未实现时，在进度文档继续标“待做”；不能以 Kali 镜像存在、Docker 可用、或 `cargo test` 通过替代真实隔离验收。

### 安装与任务启动的互斥验收细则

已实施的底层互斥：`begin_environment_preparation` 先取得 OS 文件锁，再在 SQLite `BEGIN IMMEDIATE` 中检查 `queued/scanning/pausing` 并认领持久租约。`sentinel_scans` 的 INSERT/UPDATE 触发器阻止租约存在时激活任务；双向并发仅一方成功。只有全部安装步骤成功、显式 `complete()` 才释放持久租约并记录事件；错误、异常 drop/崩溃均继续阻断入队（即使失败发生在安装器启动之前，也须人工恢复）。状态查询区分 `idle/installing/requires_manual_recovery`；恢复须锁空闲、指定当前 owner、无活跃任务及人工确认安装进程已停止，并写恢复事件。确认语只是人工声明，不证明进程确已退出，且当前 UI 无独立管理员身份认证。

后续必须补：能力包专用的 epoch/阶段、固定 digest 的 staging→原子发布、签名/许可/平台校验、安装子进程取消与完成证明、管理员身份/操作人审计和平台沙箱适配器。不能给现有 `brew`/`winget` 安装器套上“离线可复现”标签，也不能把上述互斥等同网络隔离。恢复时不能因 OS 锁空闲就自动删租约；子进程可能存活，人工核对是当前最小安全处理。
