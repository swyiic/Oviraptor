# Nest 聊天异步状态、任务切换与时间线审核

日期：2026-09-26。范围：Master Plan §13.3 的人工指令、真实聊天投影、事件补齐与状态恢复相关要求。

## 结论与边界

### 人工操作失败提示隐私续核

- 六个失败入口原先可能回显异常原文：发送/确认的未知错误、取消、当前回执核对、历史回执核对和补充预览。使用含虚构路径/Token/Authorization 的 Error 与字符串异常复现，12 条泄露断言失败；另四条已知路由/接收方/租约拒绝提示回归原本即通过。不涉及真实凭据或目标。
- 未引入全局错误框架：原指令业务模块内部分类仅返回固定提示，未知结果按动作提供刷新核对建议；取消与核对不回显异常，预览说明未创建新任务。模糊写入结果不宣称“一定没提交”，不新增自动重试；最后已核验状态、输入和独立角色权限保持不变。本次不证明后端幂等、阻止人工重复提交或任意消息正文均已脱敏。
- 两个业务模块 **277/42 行**；新增 `tools/agent_dialog/action_privacy.cjs` **61 行 / 16 项**，复用既有指令入口、SFC 夹具与模板渲染，没有新依赖。覆盖六种动作的 Error/字符串异常、输入/快照保留、无自动重放/预览事件，以及四类已知错误附加敏感原文不泄露。
- 中间专项仍有两条旧文案断言失败，其中预览用例因断言提前抛出而未执行末尾 unmount、留下轮询。确认进程属于本次测试后仅终止其子进程，补上 `t.after(unmount)`；更新两处固定提示断言，不删业务断言或用例。原始中间失败日志保留。
- 最终完整 UI **711/711**、类型检查/生产构建、全目标/全特性严格 Clippy、fmt、差异检查 exit 0；聊天 JS **70.86 kB**。日志 `/tmp/oviraptor-action-privacy-{red,green,ui-final,build,clippy}.log`，最终 UI 结果是 `ui-final`，不是名为 `green` 的中间日志。所有本次进程已收终态。
- 未修改 Rust、执行器或门禁，未重跑 Rust 全量、真实 IPC/桌面、包安装或外部目标。当前安装态不一致和 §18 发布缺口依然存在。

### 2026-09-30：消息身份在缓存、阅读与模板间保持一致

- 展示合同原本允许任意非空事件类型和 ID，并用 JSON 二元组排重；缓存/阅读却用冒号、模板用连字符拼接。合成输入 `("type:a", "b")` 与 `("type", "a:b")` 先红测证明增量覆盖和阅读锚点误跳；不是宣称已观察到生产数据损坏。
- 在原业务合同新增 `timelineIdentity`，校验、状态合并、阅读定位/已显示集合和 Vue 键统一复用。真同一身份的更新仍替换旧记录；不同身份同时保留；锚点消息移出缓存时保留当前阅读页，禁止该页推进服务端已读状态。不修改消息来源身份、角色授权、持久化格式或后端执行。
- 新增 `tools/agent_dialog/timeline_identity.cjs` **61 行 / 3 项**，复用原测试入口及真实 SFC 夹具；另覆盖实际编译模板的连字符/冒号键。合同 **37 行**、状态 **232 行**、阅读 **227 行**、聊天组件 **400 行**、夹具 **293 行**；没有引入全局工具类或新增依赖。
- 红测 2/2 失败符合预期；首次时间线/阅读专项 **67/67**；全量首次 **694/695**，一项历史测试硬编码旧锚点，改用统一身份函数且保留原业务断言后最终 **695/695**。类型检查/生产构建 exit 0，聊天产物 **69.94 kB**。原始日志 `/tmp/oviraptor-timeline-identity-{red,green,ui,ui-final,build}.log`。
- 没有真实桌面/浏览器交互验收、Rust 全量、打包/安装、扫描或目标访问。真实模板与合成传输测试不等于真实 IPC/独立 Agent 协作验收；Master Plan 与安装态仍未完成。

### 2026-09-29：任务导航、历史与阅读偏好错误展示

- 将任务选择读取/保存、单任务读取、任务列表/更早分页、历史消息、阅读偏好读取/保存共八个错误入口改为固定中英文提示，不回显异常中的路径、凭据或内部错误码。分别留在原业务 composable，没有建立全局异常框架；阅读模块向历史模块传递既有翻译函数。
- 保留选择保存 CAS 冲突后的显式重读、失败偏好保存禁止自动重放、畸形回包拒绝、项目/attempt/请求代次隔离、分页游标及只读历史语义。没有更改执行权限、门禁或消息正文。
- 任务/阅读/历史模块分别 **230/227/104 行**，聊天组件 **399 行**且未增长；新增测试 `tools/agent_dialog/navigation_privacy.cjs` **53 行 / 16 项**，复用原入口和真实 SFC 渲染夹具，无新依赖。覆盖 Error 与字符串异常，不删除独有回归；旧测试仅更新错误文案断言。
- 首轮聊天 **297/299**：两项新用例将有历史缺口的窗口误作可标读窗口，保存被正常保护阻断；随后仅修正夹具，完整 UI **639/639**、类型检查/生产构建与空白检查通过。日志 `/tmp/oviraptor-navigation-privacy-{chat,ui,build}.log`；首次聊天日志保留失败，不宣称先红后绿。主 JS **309.67 kB**、聊天 **69.99 kB**。
- 本轮没有真实浏览器/桌面、打包、安装或外部扫描。证明范围是上述异常展示入口，不包括人工指令错误、任意消息正文或嵌套回执的全站脱敏；大型 Board **4,094 行**的结构债仍在，不能据此宣称项目结构或 Master Plan 全部完成。
- 同时收取前序 Rust 全目标/全特性会话 **42557，exit 0**：主库 **1437/1437**、历史导入器 **30/30**，没有重复启动 Cargo；主库耗时 **1606.96 秒**。本次仅是完成已有回归，不代表 CPU 问题、打包/安装态或完整产品验收已完成。

### 2026-09-29：状态与事件监听错误不再回显底层原文

- 状态读取与事件监听异常原先直接写入可渲染错误字段，合成的文件路径、查询凭据及 Authorization 文本能够进入聊天提示。新增 Error 对象/字符串两种错误形式的 10 项回归，修复前状态专项 **59 通过 / 10 失败**；这是本地模拟传输证明的展示风险，不是发现真实凭据泄露。
- `useAgentDialogStatus.ts` 内改为固定中英文提示，不输出或新增记录底层错误原文。仍在内部按既有三类回执错误前缀判断失效：`request_review_`、`administrative_closure_`、`closure_handoff_`；清空缓存/水位、失效旧请求、恢复时强制全量读取的行为不变。普通失败保留上次快照，事件监听失败继续数据库轮询；本次未放宽读取合同或执行权限。
- 新增 `tools/agent_dialog/status_privacy.cjs` **61 行 / 10 项**，通过既有状态测试入口加载，复用真实 SFC/渲染夹具，无新增依赖、npm 入口或公共层。状态模块 **227→233 行**；状态/事件/时间线测试 **353/298/346 行**，原回归均保留。更新旧错误文本预期，没有删除水位、缓存隔离、重试或恢复断言。
- 第一次完整聊天复跑 **254/283**：29 项旧时间线测试仍期待内部错误码，两处参数化断言改为固定用户提示后，最终聊天 **283/283**、完整 UI **623/623**、类型检查/生产构建与空白检查通过。主 JS **309.67 kB**、聊天 **69.22 kB**。日志 `/tmp/oviraptor-status-privacy-{red,green,green-final,ui,build}.log` 保留各轮结果。
- 范围仅为状态与事件监听两个错误入口，不是通用文本脱敏器或整站 Secret 验收。任务列表/选择、历史页、阅读偏好等独立 composable 仍有 `String(cause)` 的可展示错误赋值，需继续分别审计；不凭本次修改宣布这些入口已安全。嵌套回执、消息正文和人工操作错误也不在此增量的证明范围。
- 没有修改 Rust、派发能力、安全门禁或历史格式，没有外部扫描、真实浏览器/桌面验证、打包或安装。此前启动的 Rust 全量会话 **42557** 仍在运行，终态以实施状态文档后续收取为准；本次前端通过不替代完整 Master Plan 或安装态验收。

### 2026-09-29：实时消息窗口与本地缓存分离校验

- 依据现有 Rust `status_timeline_window.rs` 的只读合同，状态接口一次最多投影 100 条事件；前端合并缓存上限仍为 300 条。这两个容量不是一回事。新增业务内 `timeline/statusContract.ts` **31 行**，将游标检查从状态 composable 搬出，并在发布/合并前拒绝超过 100 条、消息超出水位或低于窗口起点、非空窗口起点为 0、增量重放请求游标以前消息等回包；非零增量窗口起点也必须在请求游标之后。
- 拒绝时保留先前有效视图与水位，不写已读，按已有 3 秒错误重试；首包异常不恢复阅读偏好。允许可选起点字段缺省、空/稀疏投影、非序号时间戳排序、任务 attempt 改变后的全量回包；不将首条可见消息当作首条持久事件，不要求每个事件都有气泡。仍不核验全部嵌套回执、授权或脱敏内容。
- 新回归收纳到 `tools/agent_dialog/status_window.cjs` **137 行**，由已有状态测试入口加载，共 **20 项**；没有增加 npm 命令、依赖、重复夹具或通用公共层。状态模块从 **243 → 227 行**，共用夹具 **284 行**，相关现有测试均低于 400 行。大型 Board 的结构债并未因此解决。
- 首批 17 项修复前 **6 通过 / 11 失败**，修复后 **17/17**。首次全聊天接入后 **226/270**：44 项旧用例使用了“有消息但游标为 0”或“单次回包携带几百条缓存”的夹具。逐处补齐正向元数据，将阅读/渲染压力测试的缓存显式注入本地状态；没有全局规范化模拟回包、放宽校验或删除原断言。浏览器模拟传输也改为真实的 100 条分窗追读，400 条突发由四次增量读取覆盖。另补首包恢复与可选字段兼容 3 项。
- 最终完整 UI **546/546**，`npm run build`（含 vue-tsc）与 `git diff --check` 通过。主 JS **309.67 kB**、聊天 **69.21 kB**。日志 `/tmp/oviraptor-status-window-{red,green,chat-initial,chat-final,ui,build}.log`；red/green 对应最初 17 项，ui 包含全部新增 20 项。
- 本轮未改 Rust、执行能力或安全门禁，未删独有回归、访问外部目标、部署或打包。未重跑真实浏览器 DOM；之前的 DOM/安装包证据不能代替本次工作树验收，Master Plan 整体仍未完成。

### 2026-09-29：实时与历史消息共用展示合同

- 当前状态入口此前只检查游标元数据，畸形 timeline 仍可能进入状态或在合并/渲染时抛错；历史入口虽检查 ID/序号，却未检查摘要、时间戳、线程和原因列表。新增 31 行业务合同 `src/features/sentinel/timeline/projectionContract.ts`，由状态与历史两个真实入口共用，删除历史入口中的重复身份检查。
- 校验消息数组、非空 ID/事件类型、正安全整数序号、字符串摘要/时间戳、可选文本及字符串列表；同一批消息不能重复序号或事件类型+ID。增量与旧缓存合并后也校验，不能借另一个实体复用旧序号。失败保留先前有效状态与水位，首包失败不恢复已读偏好；合法重试正常推进。
- 保留同 ID 不同事件类型、实体的新版本替换、时间戳排序、可选 null、稀疏/空历史页，不强制时间戳为 ISO。该合同只检查基础消息展示字段，不核验嵌套回执、全部状态字段或执行权限；不能替代后端证据审计、Reviewer 或 Secret 文本脱敏。
- 首批真实 SFC 红测 **34/67**，33 项失败。接入校验后的聊天回归暴露旧夹具缺失摘要/序号/事件类型，以及一次测试局部 event 名称遮蔽，均补齐/修正，未放宽生产合同或删除原断言。补充缓存冲突、首包恢复与兼容性共 3 项；最终聊天 **253/253**、完整 UI **488/488**，类型检查、生产构建与差异空白检查通过。
- 日志 `/tmp/oviraptor-timeline-projection-{red,chat,chat-corrected,chat-final,ui,build}.log` 保留全部轮次。状态/历史模块 243/103 行、共用夹具 274 行、时间线/历史测试 332/285 行；未新增测试入口、依赖或修改 Rust 代码。
- 未重跑真实浏览器 DOM、Rust 全量或打包安装；当前证据为生产 Vue setup/渲染配模拟传输。前序 `.app` 不包含本次修改，Master Plan 完整角色/恢复/桌面验收仍未完成。

### 2026-09-29：确认/取消入口绑定当前草案版本

- 审核发现：前端原先只检查任务、attempt 与 rejected 状态，未确认传入卡片仍存在于当前草案列表，也未排除已确认/已取消状态。同任务旧卡片、重复 ID 或矛盾投影仍能触发 IPC。Rust 确认事务已有 scope、revision/hash、完整性与状态核验，取消使用 pending 状态及版本条件更新；本问题是前端动作边界缺口，不能据此宣称后端授权被绕过。
- 在原指令 composable 中核对当前快照归属、唯一草案 ID、安全整数版本、非空 hash 和 pending 状态；确认额外复用现有草案合同检查，并核对线程/接收方绑定。过期或矛盾卡片在 IPC 前拒绝，保留输入、不刷新或重试。模板确认/取消按钮同步采用相同判定。正常轮询返回同版本的新对象仍可操作，不使用对象引用相等作为版本依据。
- 取消只要求当前待处理草案身份，不要求可执行的接收方绑定；历史未绑定草案仍能取消，但不能确认。没有修改后端执行链、授权范围、租约或安全门禁。
- 现有动作隔离测试中新增 5 项回归，首次 **9 通过 / 4 失败**，复现了无当前草案仍提交、矛盾确认和未绑定确认问题；正向同版本轮询用例原先即通过。原异步测试补齐当前草案夹具并明确断言旧操作实际在途，避免新拦截使原测试空跑。修复后两份指令测试 **62/62**、完整 UI **443/443**、类型检查及前端构建通过，两个完整命令均 exit 0。
- 本轮只修改现有 4 个代码文件：指令 composable **273 行**、聊天组件 **399 行**、动作隔离测试 **215 行**、指令测试 **362 行**。不新增文件、依赖或测试入口，不删除独有回归；证据为 `/tmp/oviraptor-directive-binding-{red,green,ui,build}.log`。
- 以下 DOM 续核发生在此次确认边界修改之前；本批模板已由真实 SFC 渲染回归覆盖，但未重跑人工浏览器/桌面验收、Rust 全量或 macOS 打包。已有 `.app` 不包含本批修改；不能据此勾选整个指令闭环、执行链或 Master Plan 完成。

### 2026-09-29：当前聊天组件的切换与重挂载 DOM 续核

- 本地夹具加载生产聊天组件及 composable，仅替换传输。实际制造 A/B 同时在途，B 先返回、旧 A 后返回，界面保持 B；切回 A 恢复本次组件生命周期内的未发送草稿。后续旧 B 轮询回包同样未覆盖当前 A。
- 新 attempt 清除旧轮次消息；团队频道显式标读后卸载/重挂载，模拟存储恢复频道与未读 0。未发送草稿在卸载后清空，不宣称草稿持久化；夹具显式指定初始 A，不证明最后任务选择恢复。
- 卸载后释放实际在途请求，组件未重新出现、在途归零且浏览器错误为无。过程与请求编号记录在 `../tools/chat_dom/README.md`；临时标签及回环服务已关闭。
- 本轮没有生产修改、新增依赖、测试入口或全量测试重跑。不是 Tauri IPC、真实数据库重启、真实多智能体执行或最终安装包验收，整体 Master Plan 仍未完成。

### 2026-09-29：任务详情独立读取与故障隔离

- 真实业务 composable 原先等待五项 `Promise.allSettled` 全部完成才一次发布详情。运行状态等任一读取长期未返回，会连带隐藏已经读取的执行轮次、历史证据、学习候选与知识；调用数组构造时的同步异常也会中断整批处理并产生未处理拒绝。
- 新增 4 项回归先得到 **3 通过 / 4 失败**（原 3 项展示回归仍通过），复现慢状态阻塞、失败提示延迟、任务往返时当前独立结果不可用及同步适配器异常。现在各项读取独立捕获异常并按视图代次发布；慢状态不阻止查看已取得的执行历史和日志/通信/工具记录，错误保持固定提示，不展示原始异常。
- 保留同一任务的轮次选择、任务切换/卸载隔离及最后完成时的代次复核。所有读取完成前仍如实保持整体读取提示，不宣称后端请求已取消或超时；草稿/非 Web 任务可无 Native 状态，沿用原有缺失状态语义。这里没有增加自动重试、任务启动、执行能力或权限变更。
- 任务详情模块 **191 → 195 行**；复用现有 `tools/test_execution_details.cjs`，现 **163 行**。测试运行真实业务模块与 Vue 挂载/卸载及 watcher，仅替换 IPC 和宿主；既有 SFC 展示及执行历史测试一并保留，没有新增文件、依赖、测试入口或删除用例。
- 联合专项 **37/37**，完整 UI **434/434**，类型检查/生产构建均 exit 0；任务中心产物 **38.79 kB**、主 JS **309.57 kB**。日志 `/tmp/oviraptor-detail-independent-{red,green,ui,build}.log`。没有修改 Rust，未重跑完整 Rust 测试。
- 此批源码更新在先前记录式 macOS 冷构建之后，**该旧回执和 `.app` 不覆盖本次修复**。本轮是前端构建与本地自动化验证，未安装/启动桌面应用或访问外部目标，不能据此勾选真实桌面验收、§18 全量矩阵或整个 Master Plan 完成。

### 2026-09-29：消除测试渲染上下文警告

- 既有 `withDirectives can only be used inside render functions` 警告来自执行历史夹具在 Vue render 之外直接调用编译模板，不是产品运行时结论。现在在 SSR 组件的真实 render 内取得模板节点，显式断言 Vue warning 列表为空，再返回原模板的事件回调供测试同步触发；没有屏蔽控制台警告或复制业务逻辑。
- 初次调整把事件触发一并变成 async，导致 watcher 在原“立即清空旧详情”断言前重新加载，专项与全量各出现同一项失败。最终拆开异步渲染准备与同步事件触发，保留原即时清空断言及后续加载校验；未删除测试、放宽断言或改动生产代码。
- 最终专项 **18/18**、完整 UI **430/430**（exit 0）；最终完整日志未出现 Vue warning。测试文件 **400 行**，无新增文件/依赖。日志：`/tmp/oviraptor-render-context-test.log`、`/tmp/oviraptor-render-context-ui-final.log`；中间失败全量保留在 `/tmp/oviraptor-render-context-ui.log`。本轮未改产品代码，不重复生产构建或 Rust 全量；构建仍引用归档增量的成功证据，不声称本轮重新构建。

### 2026-09-29：任务归档的项目隔离与回执检查

- 归档回调原本没有项目代次或卸载隔离；父组件接到 `archived` 会合并任务并发出成功通知。新增 3 项回归先得到 **14 通过/3 失败**，复现项目往返后旧回包发布、原始异常泄露、其他项目任务仍能提交。测试夹具首次因等待未受保护的请求而停滞，已终止该测试进程并修正夹具清理；红测数字来自随后正常退出的重跑，不把终止算作验证通过。
- 组件现在以项目代次隔离归档请求，卸载作废回调，旧 finally 不能解锁新操作。入口拒绝不属于当前选定项目的任务；未选择项目的全局视图仍按任务自身项目调用。返回值必须匹配提交时捕获的任务与项目，才可更新列表或通知父组件。后端已提交的操作不被声称取消，返回该项目时仍以数据库读取为准。
- 保留单请求防重复提交；错误显示固定提示。搜索条件变化后旧操作不覆盖新搜索错误或列表；仍在同一项目的有效成功回执可以通知父组件。第四项回归核对恢复归档传入 `false` 及搜索错误隔离。测试运行实际 SFC、Vue 生命周期和父事件回调探针，IPC 为模拟；不是后端事务或真实桌面验收。
- 初次联合专项 **29/29**；补齐第四项后完整 UI **430/430**、类型检查与生产构建 exit 0。任务中心 **354 行**、复用的执行历史测试 **395 行**；无新增文件、依赖或测试入口，未修改 Rust、执行权限或历史 JSON 格式，未重跑 Rust 全量。既有 Vue `withDirectives` 测试宿主警告仍存在，与前一轮日志一致，不声称零警告。
- 日志：`/tmp/oviraptor-archive-{red,green,ui,build}.log`。本增量不证明安装包源码一致性、§18 完整矩阵或整个 Master Plan 完成。

### 2026-09-29：任务搜索卸载与分页续核

- 复用原任务中心和执行历史测试入口，新增 4 项真实 SFC 回归；修复前 **11 通过/3 失败**，分别复现卸载未释放搜索定时器、迟到结果回写、整条任务对象作为分页游标。项目/查询/视图切换隔离已通过，未替换既有有效机制。
- 卸载现在清除防抖定时器并作废搜索序号，拒绝卸载后的新读取；成功、错误和 finally 均不能再改已销毁视图。分页只传 `id/updatedAt`，保留单请求、失败保留已有页/原游标、显式重试和末页停止。搜索失败不再展示原始后端异常文本；不声称其他入口的错误已全部脱敏。
- 初次专项 **20/20** 通过，补充重试成功与末页验证后完整 UI **420/420**、类型检查、生产构建及空白检查通过。任务中心 **396 行**、现有执行历史测试 **325 行**，无新增文件/依赖/测试入口，也未删除独有回归。构建主 JS **309.52 kB**、任务中心 **37.95 kB**；日志为 `/tmp/oviraptor-task-search-{red,green,ui,build}.log`。
- 测试仅将计时器、IPC 和宿主替换为可控夹具，运行实际组件逻辑；不证明真实桌面行为。未修改 Rust 或执行权限，未重跑 Rust 全量、部署或访问外部目标，整体目标仍未完成。

### 2026-09-29：任务详情业务拆分与只读请求生命周期

- 将任务中心的单任务详情读取移入 `execution/useTaskExecutionDetails.ts`，组件从 **540 行降至 389 行**，业务模块 **191 行**。保留现有模板、搜索、历史导入和事件接口，不建立无实际复用的公共类；没有新增依赖或独立测试入口。
- 三项新增回归先全部失败：邮箱/日志接收了不属于所选任务或轮次的返回，关闭详情没有清除加载态，组件卸载后迟到的详情/工具/邮箱/日志响应仍回写状态。现在验证邮箱的 `scanId/attemptNumber`、日志的 `scanId/attempt`；关闭预览清除加载态，卸载作废全部详情请求，卸载后的主动读取也不再发起 IPC。作废界面请求不等于取消后端已开始的读取。
- 修正原日志夹具错误使用 `attemptNumber` 的字段，采用实际类型的 `attempt`。两组现有组件测试均加载真实抽出的业务模块，仅替换 IPC/宿主；没有复制生产逻辑或删除既有回归。
- 专项 **16/16**、完整 UI **416/416**、类型检查和生产构建通过；主 JS **309.52 kB**、任务中心 **37.83 kB**。日志为 `/tmp/oviraptor-task-detail-{red,green,ui,build}.log`；修复前执行历史专项 **7 通过/3 失败**，修复后全部通过。测试文件分别 **236/151 行**，未新增测试文件。
- 本批仅修改前端、既有测试和文档；未重跑 Rust 全量，未改变执行器、权限或预算门禁，未部署或访问外部目标。此证据不替代真实 Tauri IPC、桌面视觉或全量 Master Plan 验收。下文旧数字属于历史批次，当前总览以实施状态表为准。

最新续核：工具轮次关注点增量的完整 Rust `4007` 已收取 **1342 项库测试／30 项导入测试，exit 0**。随后前端补齐事件突发合并与长时间线有界展示；最终聊天 **99**、完整 UI **248**、构建、fmt 和空白检查通过。已在本地 Chrome 对生产聊天组件做部分真实 DOM 验收，传输与数据为显式模拟，不是 Tauri IPC 或全应用验收。详细证据见本文末尾“事件突发与长时间线 DOM 续核”；下文各批数字是历史快照，整体目标仍未完成。

本轮修复了真实 `AgentDialog.vue` 中可复现的异步竞态，并增加 29 项组件逻辑回归。它不是整个 Master Plan 的完成证明，也不是完整桌面 UI 视觉验收。没有修改目标测试权限、增加主机执行器、部署应用或访问外部授权 URL。

测试运行实际 Vue SFC 的编译后 setup、Vue watcher 与挂载/卸载生命周期；替换的是 API、事件传输及 renderer 的宿主节点。测试不复制一份生产业务逻辑作为“参考实现”。该测试不渲染模板/真实 DOM，不启动 Tauri IPC，也不替代 Rust 事务、真实浏览器和打包验收。

## 先失败再修复的证据

| 批次 | 修复前结果 | 实际问题 |
| --- | --- | --- |
| 首批 9 项 | 8 失败、1 通过 | 旧任务回包清空新任务输入、旧错误污染新任务、旧 finally 解除新请求等待状态、跨任务草案被提交、编辑中的新文字丢失、卸载后回写错误 |
| 扩至 19 项 | 8 失败、11 通过 | 请求次序与 DB 快照次序混用、旧 attempt 覆盖新 attempt、状态错误不能恢复、分页错误跨项目、刷新丢弃历史页、耗尽的分页被重开、列表/监听失败未捕获 |
| 扩至 23 项 | 3 失败、20 通过 | 先发后回的更新快照被忽略、同 scan 新 attempt 未隔离人工动作、旧 attempt 草案仍可提交 |
| 最终 29 项 | 全部通过 | 加入正常确认/取消参数、事件去重与增量投影、新 attempt 时间线切换、监听失败后的实际轮询和卸载清理 |

修复前日志：

- `/tmp/oviraptor-20260926-dialog-red.log`
- `/tmp/oviraptor-20260926-dialog-async-red.log`
- `/tmp/oviraptor-20260926-dialog-attempt-red.log`

最终组件日志：`/tmp/oviraptor-20260926-dialog-green.log`。

## 已落实的行为

### 人工指令

- 每次解析、确认、取消捕获项目、任务视图代次和操作序号。A → B → A 也不能恢复旧响应对新视图的写入权限。
- 切换任务后可操作新任务；旧请求的 catch/finally 不会改变新任务的错误或等待状态。
- 后端已经发起的请求不因切换界面被声称“取消”。返回原任务时以数据库投影为准，不伪造撤回或成功消息。
- 确认/取消须匹配当前 scan 与 attempt，仍传递后端规定的草案 ID、revision、hash；前端校验不是授权机制，也不取代后端校验。
- 同一 scan 的新 attempt 到来时，旧人工操作回调及暂存拒绝卡失效。
- 输入修改使用同步修订号追踪。解析期间继续输入，不会被前一次成功回包清掉。
- 拒绝保留可修改原文与原因。DB commit 失败保留输入，不产生乐观“已发送”消息。
- 未发送文字按项目/任务/线程在本次页面实例的内存中隔离保存；不写 localStorage、磁盘或后端。**这不等于跨刷新恢复草稿。**

### 时间线

- 当前视图与卸载状态对成功、异常、定时器都生效。
- 同任务先比较 attempt，再比较数据库 `latestSequence`；只有同一版本时才用请求序号解决并发回包。较旧快照不能回滚游标，较新的快照也不能因为较早发起请求而丢失。
- 增量按 `eventType:id` 合并，保留同 ID 不同类型的事件，更新已有实体，不重复插入。
- 新 attempt 替换旧时间线，不把旧执行轮次的状态拼入新轮次。
- 无关或已经读取的事件不发起重复读取。
- 事件监听失败有明确提示，使用 DB 状态轮询；测试真实触发定时回调并验证漏掉的消息被补齐。卸载清理轮询；晚到的监听注册立即注销。
- 动作错误、状态读取错误、列表/分页错误、监听错误分开保存；正常轮询不能擦掉人工指令被拒绝的原因。

### 历史任务列表

- 最近任务刷新与历史分页使用独立请求序号，项目切换使用共同的项目代次隔离。
- 旧项目分页回包不能改变新项目的结果、错误、loading 状态。
- 正常刷新不抛弃并行加载的历史页，也不把已经耗尽的历史分页重新打开。
- 合并列表按 `updatedAt` 拒绝更旧的任务行覆盖新行。
- 列表读取错误被捕获，界面提供重新读取入口；分页错误保留通过原加载按钮重试的路径。

## 复验命令与结果

```sh
npm run test:agent-dialog
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
node tools/test_native_runtime.cjs
git diff --check
```

- 组件逻辑：29 项通过；入口为 `tools/test_agent_dialog.cjs`，已接入 npm script。
- Rust 主库：655 项通过；历史导入工具：30 项通过。
- Clippy `-D warnings`、fmt、前端 build、空白检查通过。
- 本地浏览器回环：8 次请求，匿名采集与身份隔离通过。
- 前端主 JS chunk 为 **769.64 kB**，仍有拆包警告，不应提高阈值来掩盖它。

日志分别为 `/tmp/oviraptor-20260926-dialog-{rust,clippy,build,native}.log`。日志是本地临时文件，源码测试与本审核文档才是可随项目交接的复验材料。

## 2026-09-28 增量：补充任务与回执失效后的人工动作隔离

复核真实组件时发现两个未被旧测试覆盖的入口：

1. `prepareFollowup` 只检查任务视图、不检查轮次，且无条件释放全局等待标志。同任务新 attempt 可接收旧预览；跨任务时旧请求占住新任务按钮，旧 finally 也可能解除新请求的等待状态。
2. 人工指令的轮次 watcher 只比较两个非空状态。回执完整性错误清空缓存后，再恢复同一或新的 attempt 时，旧发送/确认/取消/回执核对请求仍保有修改 UI 的权限。

先添加失败测试：补充预览专项 **4 通过 / 2 失败**（`/tmp/oviraptor-followup-view-red.log`），回执失效专项 **1 通过 / 1 失败**（`/tmp/oviraptor-dialog-invalidation-red.log`）。分别复现旧轮次草稿被打开、新任务按钮被旧请求锁定，以及快照失效后旧人工动作锁未失效；不是编译失败。

修复位于 `src/features/sentinel/components/AgentDialog.vue`：补充预览使用独立操作序号，项目/任务/轮次或已核验快照丢失时失效；成功、异常、finally 都核对所有权。人工指令在轮次变更及快照从有到无/从无到有时使旧操作失效。仅轮次标量变化触发，不因普通轮询替换对象而打断当前操作。

这些变化不撤销已发送后端请求、不声称执行取消、不重新派发、不修改后端授权或预算。恢复后以数据库为准，前端只阻止旧回调发布消息/错误或清空输入。

新增 5 项测试：旧轮次预览成功/失败、4 种视图变化及新旧 finally 交错、同轮次预览保留、4 种人工动作 × 2 种恢复轮次 × 成功/失败共 16 种组合、同轮次人工操作防重复。完整聊天组件 **57 passed / 0 failed，exit 0**（`/tmp/oviraptor-dialog-view-isolation-green.log`）。完整 UI/构建/本机浏览器仍待本轮通用门禁；不是 Tauri IPC/视觉/安装包验收。线程选择与未读状态持久化没有在本增量实现。

## 2026-09-28 增量：线程选择与已读游标的数据库闭环

本增量接通 `agent_dialog_views` → 两个 Tauri command → 类型/API → 真实 `AgentDialog.vue`。以当前 scan/attempt 隔离本地操作者的线程选择与阅读游标，不保存消息正文、草稿、凭据，也不参与 mailbox ACK、审批、预算或执行授权。新表是 UI 状态，不是执行账本。

后端 `commands/agent_dialog_view.rs`：无记录读取返回 revision 0，但不补写记录；保存只接受真实已脱敏时间线中的线程与不超过真实事件序列的游标。IMMEDIATE 事务中重查当前 attempt/删除标记，以 expected revision 比较交换阻止旧窗口覆盖，游标单调增加，全部已读可压缩被覆盖的线程游标。写后重新核验，触发器损坏回滚；无有效提交回执不声称保存成功。记录、线程数、字符串长度和 JS 安全整数均有边界；损坏记录只报错，不偷偷修复。

前端自动恢复/保存线程选择，**只有显式点击“标记当前线程已加载消息为已读”才写阅读游标**；点击选线程不标已读，提交成功前不减少未读数。计数针对已加载时间线条目的最新事件序列，不是无限历史消息总数。晚到新消息保留未读，缺失线程显示不可用选项而不覆盖旧偏好。读写失败独立展示，不隐藏扫描结果，也不停止任务；版本冲突必须显式重新加载，没有后台覆盖重试。项目/任务/轮次/快照失效以及卸载均隔离旧回包；同轮次正常轮询不重置偏好或发起写入。

同时移除“无漏洞候选 · 免审”字样，明确没有候选不代表总体覆盖审查完成。

已收取证据（均为本地测试，不调用外部模型/用户目标）：

- `cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -j 1 dialog_view_ -- --test-threads=1`：**7 passed / 0 failed，exit 0**。包含全库前后对比、双连接同时保存只有一个成功、旧窗口拒绝、单调游标、未来游标/损坏记录、换轮/删除标记、写后损坏回滚、重开数据库、重复初始化和任务删除级联。日志 `/tmp/oviraptor-dialog-persistence-rust-final.log`。
- `node tools/test_agent_dialog.cjs`：**66 passed / 0 failed，exit 0**。新增阅读状态 8 项及零候选文案 1 项，包含读/写 × 四种视图边界 × 成功/失败共 16 种组合、旧 finally 不解锁新请求、卸载后异常、显式已读和无写入轮询。日志 `/tmp/oviraptor-dialog-persistence-ui.log`。
- 最新严格 all-targets/all-features Clippy：**exit 0**，7.02 秒；日志 `/tmp/oviraptor-dialog-persistence-clippy.log`。
- `npm run build`：**exit 0**；主 JS chunk **854.70 kB**，仍有拆包警告，未修改阈值掩盖。日志 `/tmp/oviraptor-dialog-persistence-build.log`。
- `npm run test:ui`：**215 passed / 0 failed，exit 0**；日志 `/tmp/oviraptor-dialog-persistence-full-ui.log`。
- `node tools/test_native_runtime.cjs`：**exit 0**，本机浏览器匿名/对照身份采集完成，隔离检查通过；日志 `/tmp/oviraptor-dialog-persistence-native-runtime.log`。这是已有浏览器 worker 回归，不是聊天桌面 IPC 验收。
- 独立 `cargo fmt -- --check` 和 `git diff --check`：**exit 0**；新增后端文件另做 no-index 空白检查通过。fmt 日志 `/tmp/oviraptor-dialog-persistence-fmt.log`。
- 此线程/已读增量的完整 all-targets/all-features Rust 回归 session `37381` 已收取 **主库 1302／导入器 30，exit 0**；主库耗时 1170.13 秒，日志 `/tmp/oviraptor-dialog-persistence-all-targets-all-features.log`。它编译于下节任务选择增量之前，不能替代新代码回归。单 Cargo、`nice -n 15`、`-j 1`、`--test-threads=1`，不宣称 CPU 硬限额。

首轮新增组件测试的三个渲染失败来自测试 `status()` 缺少真实 API 必填的 `stopDiagnostic`；已补齐夹具，不通过弱化生产校验处理。首轮 61/64 不作为通过证据。初版后端 5 项和 UI 213 项为中间快照；以上 7/66 与完整 UI 215 是补齐并发/迁移/迟到回包后的证据。完整 Rust 1302/30 是独立收取的新结果，不复用默认 v4 的 1295/30。

该快照范围限制：当时尚未恢复“项目上次选中的任务”（包括不在前 300 条列表的旧任务）；下一节为后续实现与独立验证状态，不能将上一快照的门禁移用于新代码。真实桌面重启/IPC/安装包仍未验证。当前按 scan 的现行 attempt 恢复，不提供任意历史轮次的阅读编辑界面。不支持多用户独立阅读身份，当前为本地操作者。§13.3 不能因这项增量整体勾选完成。

## 2026-09-28 项目上次任务恢复增量（该快照全量已收取）

新增 `agent_dialog_selections`：每项目及全项目视图分开存储最后选择，使用 revision CAS；任务删除 `SET NULL` 保留版本，避免旧窗口以 revision 0 覆盖删除后的偏好，项目删除级联清理。SQL 约束显式拒绝空 scope key 和 NULL project 绕过 CHECK 的情况。偏好不包含输入草稿、授权、凭据或阅读游标。

新增 get/save selection 与 scoped task read 三个异步 Tauri command，通过阻塞工作线程访问数据库。恢复直接按 ID 读取当前项目内、非删除标记的任务，不翻完所有历史分页。纯读不初始化偏好行、不修改执行状态；保存以 IMMEDIATE 事务重验范围、比较版本及核验写后结果。损坏、跨项目、已删除、越界版本及提交失败均不得产生成功回执。

真实聊天组件优先处理显式传入任务，其次恢复偏好，再使用现有默认选择。恢复旧任务不移动第一页分页游标，不调用 resume/审批/工具，也不标记消息已读。快速本地切换串行合并最后一次待存选择，返回值不倒退当前所选任务；跨窗口冲突须手动重读并重新选择，不自动覆盖。首次偏好读取失败同样清空待存选择，防止后续重读误提交失败前的旧选择。删除/移出项目提示及读取错误在空任务列表上仍可见。项目往返、卸载后旧读写回包被隔离。

当前证据与待收取项：

- 聊天实际 SFC 组件 **76 项通过**；包含之前 66 项、新增任务恢复 8 项，以及异常回包与首次读取失败排队写入 2 项。日志 `/tmp/oviraptor-dialog-selection-ui.log`。
- 新增任务恢复后端专项 **7 passed / 0 failed，exit 0**：旧于 300 条首屏、只改偏好表、项目/全局隔离、删除标记、并发 CAS、删除后版本保留、重复初始化、损坏只读拒绝、插入/更新写后损坏整体回滚。命令 `nice -n 15 cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -j 1 dialog_selection_ -- --test-threads=1`，日志 `/tmp/oviraptor-dialog-selection-rust.log`。
- 完整 UI 在补强返回数据校验后复跑 **225 项通过，exit 0**。日志 `/tmp/oviraptor-dialog-selection-full-ui.log`。
- 严格 all-targets/all-features Clippy **exit 0**（24.19 秒），fmt **exit 0**；日志 `/tmp/oviraptor-dialog-selection-clippy.log`、`/tmp/oviraptor-dialog-selection-fmt.log`。
- 构建 **exit 0**；主 JS **858.26 kB**，拆包告警保留。日志 `/tmp/oviraptor-dialog-selection-build.log`。
- 本机浏览器 worker 回归 **exit 0**，匿名和对照采集完成，8 次观测请求，身份隔离通过；日志 `/tmp/oviraptor-dialog-selection-native-runtime.log`。这不是聊天 IPC/桌面重启验证。
- `git diff --check` 及新增后端两文件 no-index 空白检查通过。新增文件必须随提交保留，不能只收集 tracked diff。
- 修改后完整 Rust 回归 **session `96706` 已收取 exit 0：主库 1309、导入器 30**，主库耗时 1184.48 秒；日志 `/tmp/oviraptor-dialog-selection-all-targets-all-features.log`。仍为单 Cargo、低优先级、单编译 job 和单测试线程；不是 CPU 硬限制。其编译后新增下节路由修改，不能证明后续快照全量。
- 之前的完整 Rust session `37381` 已收取 **1302/30，exit 0**；它编译于此任务选择增量之前，只能证明线程/已读状态那一快照。

新增异常回包用例首轮失败是夹具默认按更新时间选 B，而断言预期 A；改为明确只有 A 的任务列表后通过，没有放宽生产校验。所有测试仅针对本地数据库/替代传输；组件卸载重挂与 DB reopen 不等于安装版桌面重启验收。

## 2026-09-28 多目标人工指令路由增量（专项通过，全量待收取）

代码审核确认两个真实缺口：聊天草案按 `updated_at DESC LIMIT 1` 猜接收方，不使用选择的线程；确认重新选择最近更新的 Coordinator，导致另一个目标更新后，原草案可能无法确认。此外旧版未绑定根/目标的指令允许由首个 Coordinator 接走，存在多目标误投路径。

本轮调整：

- 草案仅从当前 scan/attempt 的有效根匹配目标、独立 `coordinator:<root-id>` 线程键、assignment 或消息 correlation；团队频道/重复 correlation 匹配多个根时明确拒绝歧义，不默认广播、不猜测最近目标。接收方解析和草案插入位于同一个 IMMEDIATE 事务，事务提交前回读并验证摘要与绑定。
- 确认命令在 IMMEDIATE 事务内读取当前 attempt、草案不可变接收方、版本和摘要；继续检查根状态、fencing 与线程归属。其他目标更新时间不改变绑定。已确认重复请求只回读同一指令，不再派发。
- 未绑定根/目标/fencing 的旧草案不可新确认；已存在未绑定 pending 历史指令保留并隔离，不再被第一个目标收养。历史正文不会被删除或转化为新授权。
- 状态只读投影新增 `directiveRecipients`，使没有消息的活跃目标也有可选线程；阅读偏好接受这些服务端投影的空线程，不接受用户凭空构造的频道。路由键与展示目标分离，避免长 URL 或查询参数脱敏破坏路由；草案返回值同样脱敏。根结束后可从既有时间线恢复可读标签。
- UI 对歧义、无接收方、未绑定旧草案提供下一步提示；发送失败保留当前输入，不产生假消息。

当前已收取：

- 最终路由 Rust 专项 **主库 92／导入器 1**，exit 0；包括新增路由回归 **11** 项。日志 `/tmp/oviraptor-directive-routing-opaque-regression.log`。新增用例还覆盖真实 SQLite 写锁竞争、插入后触发器篡改导致整笔回滚、超长含敏感参数 URL 的准确投递和回包脱敏。
- 最终完整 UI **228**，exit 0；其中聊天测试 **79** 项。日志 `/tmp/oviraptor-directive-routing-full-ui.log`。单独聊天日志 `/tmp/oviraptor-directive-routing-ui.log` 的 79 项结果早于最终已结束线程标签补充，最终快照由完整 UI 覆盖。
- 最终严格 Clippy session `14176` 已收取 exit 0，日志 `/tmp/oviraptor-directive-routing-clippy.log`；最终 `cargo fmt --all -- --check` 通过。
- 最终前端构建 session `52882` 已收取 exit 0，日志 `/tmp/oviraptor-directive-routing-final-build.log`；主 JS **859.78 kB** 大包警告仍在，未通过提高警告阈值隐藏。
- 本机浏览器 worker 回环结果 `passed=true`，8 次观察请求、匿名/对照采集 complete、身份隔离 true；日志 `/tmp/oviraptor-directive-routing-native-runtime.log`。该结果早于最终路由增量，worker 未修改；它不能替代当前聊天 IPC 或真实外部目标验收。
- 路由快照完整 Rust session **`34604`** 已收取 **主库 1320／导入器 30，exit 0**，日志 `/tmp/oviraptor-directive-routing-all-targets-all-features.log`。该编译快照早于下面的源码阶段引导增量，不覆盖新代码。保持单 Cargo、`nice -n 15`、`-j 1` 和 `--test-threads=1`；这不是 CPU 硬限制。

本增量不表示源码流程已消费运行中的全部人工提案、所有专家均已接入，或桌面 IPC/安装包已经验收。路由快照当时的源码入口尚无阶段引导；其后实现及证据见下节，不能用当时的缺口描述覆盖后续代码。团队广播若后续需要，必须先明确逐目标草案/审批合同，不能恢复首个接收者猜测行为。

## 源码阶段人工建议：冻结输入、真实送达与归档增量

本工作包接入源码初评与工具阶段首轮，未引入新的模型调用、工具、目标权限或主机执行。源码工具/辅助/深度模式依然是单独的待实现合同。

### 实现与安全边界

- `agent_source_guidance` 按 root/phase 保存不可变快照，空快照同样冻结。派发前在同一事务中领取、确认、选择建议并保存快照；插入失败或写后状态被改变时整笔回滚。已存在 assignment 的旧阶段保留原输入，不能追溯加入新消息。根删除可级联清理，单独修改/删除快照被拒绝。
- `native_source_coordinator.rs` 的 RepoMapper/SourceAnalyst 初评及 `native_source_tools.rs` 的每个工具阶段首次派发前执行冻结；真实模型请求带入 `operatorGuidance`。后发消息只进入后续尚未冻结的合法阶段，不改写已发请求，不复制 Web URL 队列动作。
- 仅普通、已确认的根线程只读分析偏好可进入该路径；暂停、指定其他角色、预算/范围变化等需专门动作的请求明确 deferred，并说明下一步，不降格为普通建议后声称已执行。
- 模型响应、完成事件、checkpoint、建议送达回执同事务提交；`deliveredDirectiveIds` 记录实际 ID，回执绑定实际 assignment/child/event 与快照摘要。回执失败不保留成功事件；恢复核验原请求、响应与事件，不重复联网调用。
- 聊天投影不直接信任 `payload_json.sourceGuidance`：重新关联冻结草案、快照、assignment、模型请求/响应摘要、完成事件和 checkpoint。证据不匹配时显示 `receipt_unverified`，不显示送达成功。
- 任务收口对已核验送达的偏好记 `analysis_guidance_delivered`／completed，**仅表示建议送达完成**，不表示建议采纳、工具执行、漏洞确认或覆盖通过。其余未落实指令保持原有 deferred/对账规则；收口新增写后校验。原快照、模型证据和归档回执保留。

### 已收取证据与正在运行的门禁

- 源码建议专项 **10 passed，exit 0**，日志 `/tmp/oviraptor-source-guidance-closure-rust.log`。涵盖阶段/晚到消息、旧任务、unsupported 动作、写后故障回滚、fencing、响应原子性、回放篡改、快照级联删除，以及真实生产入口的 localhost 模型请求；不是只调用纯函数的模拟。
- 生产入口测试实际完成 7 次本机模型请求。在第一、第二次请求进行中各提交一条消息，分别进入下一 SourceAnalyst 初评和 RepoMapper 工具阶段；旧请求不含晚到消息，实际事件 ID、聊天投影与归档回执一致。修改 child 回执后投影降为未核验，不再显示成功。
- 指令回归 **主库 92／导入器 1，exit 0**，日志 `/tmp/oviraptor-source-guidance-directive-regression.log`；包含旧 Web 优先级/提案/终态收口路径。
- 聊天组件 **83**、完整 UI **232** 通过，日志 `/tmp/oviraptor-source-guidance-closure-ui.log`、`/tmp/oviraptor-source-guidance-full-ui.log`。
- 前端构建通过，日志 `/tmp/oviraptor-source-guidance-build.log`；主 JS **861.70 kB**，大包警告仍在。
- 本机浏览器 worker **passed=true**，8 次观察请求、匿名/对照 complete、身份隔离 true，日志 `/tmp/oviraptor-source-guidance-native-runtime.log`。没有访问用户外部 URL，不代表真实聊天桌面 IPC 验收。
- 源码范围 Rust 回归 session **`60591` 已正常退出，主库 274／导入器 2 通过**，日志 `/tmp/oviraptor-source-guidance-source-regression.log`；此编译不含下方回执缺失修复，不能用于证明后续修改。

### 仍需继续

本段为初评/工具增量时的缺口快照；Reviewer 受控引导已在下方续核接入，状态以下方证据为准。工具阶段内部晚到消息的后续轮次策略、源码专属角色提案/动作与完整恢复仍需继续，不能宣称覆盖全部自然语言意图。已结束阶段不吸收新建议，扩大范围仍走新合同/新任务。桌面重启/IPC、真实沙箱/平台矩阵和整个 Master Plan 未完成。

### 送达回执缺失与旧库升级续核

发现并修复一处投影遗漏：此前仅对非空 `sourceGuidance` 核验，删除该字段后仍可能沿用通用 `modelDelivery` 的送达状态。当前投影对指令逐条核验；若冻结阶段已有真实响应或归档声明，而源码回执缺失/置空，则显示 `receipt_unverified`。归档前即使两个送达字段均被移除，也从已有阶段响应识别缺口；只冻结、尚未收到响应的合法等待状态不误报。归档回执本身保持不可变，删除被拒绝。读取不补造回执、不改状态、不重发模型请求；结案同样不能把损坏回执降格为普通未执行建议。

新增旧库升级/再次打开测试：仅在临时夹具中移除新增空表，使用真实 `db::initialize` 恢复 schema；核对原草案、指令、不可变快照、租约及无新增模型调用。生产入口回归扩展至初评/工具两类阶段、错误 child、删除/置空回执、同时删除展示字段，并校验读取不修复损坏数据。

当前聊天 **84**、完整 UI **233** 通过，日志 `/tmp/oviraptor-source-guidance-integrity-ui.log`、`/tmp/oviraptor-source-guidance-integrity-full-ui.log`。第一轮 Rust 专项 **10 通过／1 失败**，日志 `/tmp/oviraptor-source-guidance-integrity-rust.log`：测试试图移除不可变 `taskClosure`，被 `directive_task_closure_immutable` 拒绝。没有关闭保护；测试已改为明确断言归档删除被拒绝，并在归档前删除两个送达字段验证实际响应兜底识别。修正后专项 **11 passed，exit 0**（session `12892`），日志 `/tmp/oviraptor-source-guidance-integrity-rust-final.log`；包含旧库升级与重开验证。严格 all-targets/all-features Clippy **exit 0**，日志 `/tmp/oviraptor-source-guidance-integrity-clippy.log`；最终 fmt 与 `git diff --check` 通过。

完整 Rust 回归 session **`44173` 已收取 exit 0：主库 1331／历史导入器 30 全通过**，日志 `/tmp/oviraptor-source-guidance-integrity-all-targets-all-features.log`，主库耗时 1201.90s。命令为 `nice -n 15 cargo test --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -j 1 -- --test-threads=1`；只运行一个 Cargo，`nice` 不是硬 CPU 上限。该编译覆盖回执缺失/旧库升级修复，不包含下方本轮 Reviewer 引导，不能据此标记新代码或整个 Master Plan 完成。

### 候选与覆盖 Reviewer 的阶段引导增量（专项及全量通过）

源码建议的阶段键新增 `review:source_candidates` 与 `review:source_coverage`。两个 Reviewer 虽然共享角色名，但通过冻结的 source 根、实际 assignment trigger、review lane、child 和租约绑定区分；Web Reviewer 不采用源码建议。保持旧 system prompt 与已派发任务原输入，避免历史重放因提示词改变失效。

- 在两个生产审查入口的 NotStarted 事务中，先验证前置证据，再冻结合法建议。无候选、没有实际候选审查时不领取建议，由下一个真实适用阶段处理。已派发/received/delivered 不重新冻结；legacy assignment 无快照时保留旧输入。
- 建议仅作为关注点；新增约束明确不得指定裁决、替代证据、隐藏覆盖缺口或扩大权限。两个 Reviewer 仍分别执行，只读冻结证据，不增加模型调用阶段、目标请求或主机动作。
- 复用实际模型响应、事件和建议回执的原子保存；投影按阶段检查实际请求、assignment、响应和 checkpoint，缺失/串阶段/伪造 child 不能显示送达。归档仅代表建议送达，不代表采用建议或确认漏洞。
- 新测试覆盖生产 localhost 请求期间提交建议、候选/覆盖分流、无候选跳转、最终冻结后晚到消息、缺口保留、回执损坏不修复、冻结事务回滚、旧 assignment 及 received 重入不重发。候选与覆盖两阶段均验收；含候选流程仍为 8 次模型请求，无候选流程仍为 7 次。

完整 UI session `59378` 已收取 **235 passed，exit 0**，日志 `/tmp/oviraptor-source-review-guidance-ui.log`，其中包含两个 Reviewer 阶段各自的真实 child/阶段提示与不冒充裁决的回归。仅增加组件回归，未改前端产品组件，不宣称完成桌面视觉/IPC 验收。此前 session `44173` 的编译不包含本 Reviewer 增量，不能作为其全量证明。

首轮专项 session `97112` 编译失败（exit 101）：新测试用 `usize` 读取 SQLite 计数，但该类型没有 `FromSql` 实现；已改为 `i64`，未改生产检查。日志 `/tmp/oviraptor-source-review-guidance-rust.log` 保留。修正后 session **`93797` 已收取 14 passed、exit 0，耗时 108.24s**，日志 `/tmp/oviraptor-source-review-guidance-rust-retry.log`。严格 all-targets/all-features Clippy session **`53337` exit 0**，日志 `/tmp/oviraptor-source-review-guidance-clippy.log`；最终 `cargo fmt --all -- --check` 和 `git diff --check` 均 exit 0。

包含 Reviewer 增量的完整 Rust 回归 **session `6253` 已收取 exit 0：主库 1334／历史导入器 30 全通过**，主库耗时 1327.42s，日志 `/tmp/oviraptor-source-review-guidance-all-targets-all-features.log`。包含 14 项源码引导及 9 项来源 gap 再审，并覆盖共享 specialist/Web Reviewer 路径。使用 `nice -n 15`、离线、all-targets/all-features、`-j 1`、`--test-threads=1`，仅一个 Cargo。此编译不含下方指定角色关注点新代码；不是新关注点或整体目标的通过证明。

此快照尚未扩展源码专属动作/指定角色指令、工具阶段轮内动态改策、源码三档、独立主机模块、Stage 9/10 或真实平台验收；后续指定角色关注点见下一节。严格授权与资源上限继续保留；没有访问外部目标。

### 指定源码角色关注点（2026-09-28，专项通过，新全量运行中）

本轮检查发现 `@source_analyst` / `@repo_mapper` 未被解析为指定角色，可能退化为协调者普通建议，由其他源码阶段先消费。现增加两类源码角色别名，并且仅在数据库确认的源码根任务及根级线程中，将明确的 `@角色 请关注…` / `请解释…` / `请分析…`（英文 `focus on` / `explain`）解析为 `source_analysis_focus`。支持 RepoMapper、SourceAnalyst 和共享 Reviewer；后者仍由真实候选/覆盖阶段分别接收。

- 新草案显示接收角色、需要确认、不新增模型请求及“仅下一未冻结阶段”语义；没有剩余阶段时不改投、不补开 Agent，不显示送达。私有 assignment/message 线程不静默转投其他阶段；多角色/非明确关注点请求保留原提案或专用动作语义。
- 冻结阶段遇到其他角色的关注点继续等待，不拒绝也不消费；真正冻结时纳入原实际请求、回执、事件和归档链。读取不可变快照再次核对指定角色。已派发阶段和旧草案不被重新解释。
- 暂停、扩权、增加预算、写入、跳过复核、直接指定裁决不降格为关注点。Web 根任务不启用源码关注点；仅伪造 `source:` 目标也不能选中源码模式。
- 增加草案动作分类、私有线程、不同角色等待、已冻结/耗尽不改投与 Web 隔离测试；原 localhost 生产测试扩展为普通建议/指定角色两种模式，验证实际请求、回执损坏和 Reviewer 无候选跳过，保持原调用数量。

本轮聊天 **87**、完整 UI **236 passed / exit 0**，最终日志 `/tmp/oviraptor-source-addressed-focus-ui-final.log`（含被拒绝草案不再显示送达提示、不将关注点展示为调度动作）；首次 UI 日志亦保留。最终构建 **exit 0**，日志 `/tmp/oviraptor-source-addressed-focus-build-final.log`；主 JS **862.49 kB** 告警保留。增加重算摘要后复制到另一角色的快照拒绝测试，不关闭不可变触发器，读取不修复损坏行。没有真实桌面/IPC/视觉通过声明。

本轮 Rust 专项 **session `62122` 已收取 18 passed、exit 0，128.78s**，日志 `/tmp/oviraptor-source-addressed-focus-rust.log`；其中 4 项新增、原生产回归扩展普通/指定角色两种模式。导入器因过滤为 0 项，不作为导入验收。严格 all-targets/all-features Clippy **session `87338` exit 0**，日志 `/tmp/oviraptor-source-addressed-focus-clippy.log`；`cargo fmt --all -- --check` 与 `git diff --check` 均 exit 0。

包含本轮最终代码的新全量 **session `88757` 正在运行**，日志 `/tmp/oviraptor-source-addressed-focus-all-targets-all-features.log`，终态尚未收取。前序 `6253` 的 **1334／30** 不覆盖本轮指定角色改动。继续保持离线、单 Cargo、`nice -n 15`、`-j 1`、`--test-threads=1`；没有访问外部目标，也没有新增主机能力。

此增量是对既有源码角色的阶段关注点路由，不是任意源码角色动作、动态工具轮次改策、额外 Agent 派发或完整自然语言闭环。整体目标继续进行中。

## 后续不能跳过的工作

1. 按 Master Plan 对 assignment → child-run → mailbox → 独立 Reviewer → 人工指令实际执行与结果投影逐项建立验收矩阵，不能以 UI 测试替代执行链验收。
2. §13.3 的线程选择和已读游标已有上述数据库增量；项目最后任务恢复在本节增量中验证，真实重启/IPC、未处理审批/完整协作恢复仍需逐项验收。内存草稿隔离不能当作持久化恢复证据。
3. 本轮读取的 scheduler 仅接受 Mapper、Identity、Deep Investigator、WebExecutor、Authorization、Reviewer 等已列角色，其他角色明确拒绝；不能宣称 Stage 10 全部专家已实现。
4. Stage 9 的来源关联补证已存在实际新 Broker HTTP 事实、独立 Reviewer 和来源 gap receipt 链，不能再当作只有 deferred 提案：当前 `agent_gap_review.rs` 与 `agent_tests_gap_review.rs` 已核对，前序全量 `44173` 包含 9 项相关通过记录。后续应验证本轮共享 specialist 改动的兼容，并继续补非 HTTP 证据来源、其他专家、完整授权/跨 fencing 与桌面恢复边界；这些未完成项不能被已有链路替代。
5. 真正的隔离沙箱、固定工具镜像、Linux 运行、安装包及完整场景验收仍需独立证据。
6. 增补真实 DOM/桌面交互检查，尤其是长时间线、输入草稿、错误提示和事件突发时的可用性/性能。此处没有截图或视觉通过声明。

当前依旧 `web_only`。主机扩展申请与执行授权分离；没有新增可执行主机功能。

### 源码工具轮次内的人工关注点（本轮实现，Rust 验证待收取）

生产 `native_source_tools.rs` 已在第二、三轮预算准入前接入 `source_rounds::prepare_next_authorized`。这不是增加模型轮数、另开专家或执行任意角色动作；关注点只进入现有工具 assignment 的下一未冻结轮次，工具、目标、轮数和总预算不变。

- 新确认的根级普通分析建议与明确指定源码角色关注点在草案 hash 内带 `source_guidance_tool_round_eligible` 标记，预览解释接收边界。旧 phase-only 草案不添加标记、不重算 hash、不自动提前到工具轮次；旧 assignment 没有首轮引导快照时也不补建轮次引导。
- 按 `tools:<role>:round:<n>` 冻结不可变快照，包括空快照。先验证真实运行 child、租约、前一轮已完成的工具回执与精确 transcript，再冻结。后到消息不能改写已有请求；另一角色的关注点不被消费，没有后续轮次/阶段时归档为未作用。
- 下一轮只追加一条包含冻结 `operatorGuidance` 的用户输入，之前所有消息、工具定义与模型配置保持原样。实际完整请求参与 token 准入，预算不足不签发请求；准备快照不代表送达。
- 每个工具轮次的模型事件、送达回执、历史投影按实际轮号校验。模型响应持久化后才记录送达；未知结果不自动重发，received 恢复不重复调用。缺失回执或输入不一致不修复成成功，也不冒充建议采纳或任务完成。
- 专项增加轮次冻结写后故障/撤权回滚、预算不足、空快照后晚到建议进入第三轮、旧 assignment 和有效旧草案不升级测试；扩展原工具回执测试覆盖第二轮、数据库重开、未知结果、损坏回执、跨角色隔离。真实 localhost 生产测试在工具请求进行中确认建议，检查下一轮实际请求及最终收口，无候选流程仍为原 7 次模型请求。

当前完整 UI **238 passed / exit 0**（session `29574`，`/tmp/oviraptor-source-round-focus-ui-final.log`）。最终构建 session `87121` 已收取 **exit 0**，主 JS **863.46 kB** 大包告警保留。前序指定角色全量 `88757` 已收取 **1338 项库测试／30 项导入测试，exit 0**，但不覆盖本节工具轮次增量。

本节新 Rust 补验：首次编译发现测试中 `(bool, usize)` 匹配不穷尽，已补上仅用于守卫外不可达情况的分支。其后专项 `59028` 为 **21 passed / 1 failed**，失败位于生产回归的测试预期：新增第四条未送达建议不应被要求拥有送达回执。现将三条真实送达建议的损坏回执检查保留，并另验第四条建议的未作用状态、归档不可删除、伪造回执拒绝及恢复；未放宽生产校验。修正后专项 `14418` 运行中，日志 `/tmp/oviraptor-source-round-focus-guidance-final.log`。当前不能宣布最新 Rust 门禁全绿。

此改动补的是源码工具轮内只读分析关注点，不是任意策略/动作变更、额外专家派发、完整指令闭环或全 Master Plan 验收；真实桌面 IPC/视觉、其余专家、恢复与平台隔离等未完成项仍保留。

#### 本节补验终态

- 修正后的源码引导专项 `14418`：**22 passed / exit 0**，日志 `/tmp/oviraptor-source-round-focus-guidance-final.log`。
- 共用工具轮次回归 `7798`：**9 passed / exit 0**，日志 `/tmp/oviraptor-source-round-focus-rounds.log`。
- 严格 `cargo clippy --offline --all-targets --all-features -j 1 -- -D warnings`（指定 `src-tauri/Cargo.toml`，`nice -n 15`）`3796`：**exit 0**，日志 `/tmp/oviraptor-source-round-focus-clippy.log`。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` 与 `git diff --check` 无报错；完整 UI **238** 和最终前端构建结果见上文。

上述测试保持单 Cargo、单构建 job、单测试线程；只使用本地材料与 localhost 测试端点。此次补验未启动新全量测试，旧全量 **1338／30** 不覆盖轮次关注点增量，不合并统计为最新全量通过。没有部署、外部目标探测或主机执行。产品文档首页补充了“工具执行节点不等于主机评估目标”的边界；独立主机评估、源码按需三档仍未实现，整体目标保持未完成。

### 事件突发与长时间线 DOM 续核

本节覆盖前序源码轮次全量收取，以及后续聊天组件增量。未改 Rust 生产代码、目标授权、审批或扫描执行权限。

#### 生产修复

- 原组件 199 条通知立即触发 199 次 IPC 读取；重复突发产生 101 次读取，空 payload 抛异常。五项红测失败证据：`/tmp/oviraptor-chat-event-batch-red.log`。
- 现以 50ms 窗口合并提示，同一当前视图的事件驱动读取单飞，只保存最高 attempt/sequence 和关联任务标志；必要时尾随追读，已有快照覆盖的提示不再读取。先比较 attempt，再比较 sequence；畸形通知丢弃。旧视图仍在途的 IPC 不能被伪称取消，但不能发布到新任务或释放新任务的队列。
- 终态任务的事件读取失败也保留 3 秒数据库重读；完整性错误清空缓存后，追读必须恢复完整历史，不以增量假装全量。7 项新增回归覆盖突发、覆盖去重、新 attempt、任务切换/卸载、错误重读、新旧视图队列隔离和完整性恢复。该阶段聊天 **96**、UI **245** 与构建 exit 0 已收取。
- 实际浏览器中发现长列表全部展开、输入区随消息数下移。新增展示层每页最多 100 条气泡及独立滚动区；默认最新页，可向前/后翻页。旧页以事件类型和 ID 组合锚定，新消息不抢走阅读位置；换线程/任务/attempt 回到最新页。不裁剪已加载证据，不自动写已读或通信回执。3 项新红测原先失败，其中模板实际渲染 **450** 条而非 **100**；日志 `/tmp/oviraptor-chat-dom-window-red.log`。修复后全部通过。

#### 本地浏览器证据及限制

新增 `tools/serve_chat_dom.mjs` 与 `tools/chat_dom/`，通过 `npm run test:chat-dom` 启动回环地址夹具。真实生产 SFC 与样式，显式模拟 API/事件；动作接口拒绝，内存数据不含真实任务或凭据。生产入口不导入测试替身。复现步骤与已执行范围见 `tools/chat_dom/README.md`。

原生 Chrome 实际结果：初始读取 2 次；第一次 200 事件后为 3 次、在途 1；再次 200 事件和畸形通知不增加读取；释放后为 4 次、消息 401。分页修复后显示 302–401；向前翻为 202–301，再添加 200 条仍保持该页、未读变为 601，输入保留。A 请求延迟时切 B，先返回 B 后返回旧 A，仍显示 B；回 A 后显示 702–801 / 801 并恢复 A 的未发送文字，无浏览器错误。跨视图总在途峰值 2 合法，不宣称所有状态读取全局单飞。

已检查真实渲染及截图，但没有记录严格 FPS/时延/内存基准，也没有逐项完成新 attempt、失败重读、卸载的浏览器操作验收；这些只有组件回归。当前分页控制 DOM 数量，不解决全量历史内存增长。真实桌面 IPC、重启、完整 UI/平台与其余 Master Plan 条目仍待验收，模拟消息不是执行证据。

#### 最终收取结果

- Rust 全量 `4007`：**1342／30，exit 0**，`/tmp/oviraptor-source-round-focus-all-targets-all-features.log`；包含最终工具轮次关注点 Rust 改动。本节没有再改 Rust。
- 聊天 `32713`：**99 passed / exit 0**，`/tmp/oviraptor-chat-dom-window-dialog.log`。
- 完整 UI `30468`：**248 passed / exit 0**，`/tmp/oviraptor-chat-dom-window-ui.log`。
- 构建 `20788`：**exit 0**，`/tmp/oviraptor-chat-dom-window-build.log`；主 JS **866.27 kB**，大包告警未消除。
- 严格全目标/全特性 Clippy 的前序 `3796` exit 0，日志已核对；之后无 Rust 变更。本轮重新运行 fmt、`git diff --check` 和夹具脚本语法检查通过。

全量进程已正常退出；未启动第二个 Cargo。上述结果不代表 Strix 剥离、历史 JSON、完整指令与专家矩阵、真实沙箱及安装包已完成逐项最终审计。整体目标保持未完成。
