# 源码首轮工具阶段恢复审计（2026-09-28）

## 范围与结论

本次先处理同一有效 Coordinator lease 下，源码初评两阶段和第一轮 `repo_mapper` 工具阶段均已持久化完成、第二轮 `source_analyst` 工具阶段尚未创建时的顶层重入。原路径会重新计算第一轮工具的预留并尝试重建同一 assignment，因 `assignment_dedup_conflict` 中止。现在重入先审计完整的三阶段前缀和已结算账本，再仅执行尚未完成的第二轮工具阶段和后续 Reviewer；已完成模型请求不重发。

后续续核又发现相邻的前缀损坏问题：仅完成第一项或两项初评时，原顶层重入未在取得执行权前审计已有回执。真实红测将首项初评的根预算改坏后观察到业务表写入。现已把一／两阶段初评前缀的 assignment、child、ACK、回执、用量、冻结任务、账本、额外工作和原有效 lease 审计前移；损坏时零模型重发、零业务表写入，正常前缀仍继续执行。

同日新增“初评响应已经持久化、尚未本地交付”的更窄恢复边界。首项 Mapper 或第二项 Analyst 处于 `received` 且原 lease 仍有效时，先只读核对已完成前缀、冻结输入、响应事件与 checkpoint、唯一未结预留、lane/能力和额外工作，再以原 child 原回执直接进行一次事务内本地交付；不调用模型传输、不准备新 assignment、不轮换 fence。响应仍为 `executing`、回执损坏、ACK/账本/权限不符时拒绝重入且逐表无业务写入。`paused` 且回执完整的第二项也可沿原地本地结算，不重新授权。该恢复不扩展到过期 lease 或源码工具未知结果。

随后补上零 child 的首次执行准入：原先 `completed=0/nonreview=0` 直接进入 lease 获取和模型调度，根任务已有异常用量、未决预留或孤立账本时会产生新业务写入。现在先只读检查 root 的零用量／零预留、没有 child、预算账本、旧模型调用／轮次及阶段成果；异常即拒绝首次派发。正常的注册后首次执行仍由原路径处理。这是空前缀完整性检查，不是过期 lease 接管或未知外部结果的自动核对。

顶层续跑原本对一／两项已完成初评虽审计了前缀，但随后仍从第一个初评角色重新经过调度、回执读取和本地交付。现在准入返回明确的 `Initial(next)`、`Tools(next)` 或 `Review` 阶段：仅运行尚未完成的初评／工具阶段，已完成的评估直接使用已核验的前缀投影。源码初评的执行放在独立业务文件 `native_source_initial.rs`，不再向 Coordinator 增加重复循环；这不把 `received` 当成已完成，也不放宽未知工具结果或过期 lease 的拒绝条件。

续核又发现根 run 上的孤立 `tool_invocations` 未被上述准入覆盖：它可能代表未知外部副作用，即使已有部分 child 回执完整也不能接着派发。现将只读检查放在源码顶层重入各分支之前，所有阶段均拒绝这种不属于 fenced child 的工具记录；不删除该记录、不退款、不重发、不终态化。正常的工具回执仍归各 child 审计。此项只处理明确可见的根 invocation，不能证明任意未知模型/目标副作用可自动恢复。

这不是通用崩溃恢复、过期 lease 接管或整个 Master Plan 的完成声明。实际授权 URL、桌面安装包和部署均未验收。

## 恢复准入和不变量

- `source_phases::audit_first_tool_prefix` 重用四阶段完成证明的逐项检查，要求恰好两项初评及第一轮 Mapper 工具阶段；核对冻结任务切片、evidence revision、child/run 绑定、完成状态、实际模型及工具回执、唯一 ACK 消息、能力收口、每个 child 用量。不能把 `completed` 标签当作证明。
- `source_phases::audit_initial_prefix` 对一／两项初评采用同一逐项回执核验，并要求精确角色顺序；只在已有工作全部落盘且原 lease 有效时允许继续。零完成项仍由原无工作启动路径处理，未知在途结果拒绝自动重试。
- 零 assignment 的启动路径额外要求根用量与预留均为零、无孤立 child／账本／阶段回执／决定；它不把用户尚未消费的消息当作模型成果，也不创建虚假的审计完成状态。
- `source_phases::audit_initial_received_prefix` 仅证明已完成的第一项，同时顶层独立核对唯一 `received` 的第二项；两项证明必须与预算、child 数、ACK 数、原始请求和当前授权一同成立。首项尚未交付时采用零已完成前缀，同样只允许唯一 `received` 的 Mapper。使用 `complete_readonly_assessment` 的原子本地交付，不调用失败清理兜底来掩盖损坏准入。
- 顶层在 acquire、fence 轮换以及通用失败终态化之前做只读准入。三阶段前缀还要求 root 预算已结算、无未决预留和活动能力、没有额外 child、消息或模型轮次。准入失败不能产生业务表写入。
- 首轮工具调用已经派发但响应未持久化时，其结果未知；拒绝自动重发、退款、换代或终态化主任务。此状态需要专门的核对流程，不由本次修复猜测结果。
- 第二轮工具阶段仍按原调度路径执行，保留原 lease、权限、预算和 Reviewer 的独立裁决。该恢复路径不跨已过期 lease，也不恢复任意其他部分阶段。

## 真实回归

`tests_source_reviewer.rs` 以本机 HTTP 模型夹具和磁盘 SQLite 走生产顶层入口，先真实完成阶段，再用独立连接重入；没有伪造模型回执。成功恢复的整个夹具仍是 **7 次**模型请求、4 条初评／工具投影、5 个 assignment。另覆盖预算篡改、ACK 丢失、冻结切片变化、lease 过期四种拒绝分支，重入前后业务表快照相等；第一轮工具响应保存失败的未知结果也保持快照、没有新 HTTP 请求。成功分支在修复前因 `assignment_dedup_conflict` 红测，未知结果分支在修复前观察到错误 root 写入，两处均已修复。

新增初评前缀真实红测先以 **exit 101** 复现第一项初评预算损坏后的错误写入，修复后同一测试 **1/1 exit 0**；它还覆盖两项初评时 ACK 缺失和 lease 过期，逐表快照保持不变。两项正常顶层恢复 **2/2 exit 0**，确认并未把合法重入封死。红测产生的巨大逐表 panic 输出只作为失败观察，不包含用户实际任务数据；测试夹具为本机临时库。

新增 `received` 尚未交付的真实 HTTP／磁盘 SQLite 回归，原生产入口修复前因 `source_partial_outcome_unknown_requires_reconciliation` 红测。修复后首项运行态和第二项已暂停态均走 **7 次**总模型调用、4 条评估／工具投影与 2 条已 ACK 初评消息。另一红绿用例使首项模型响应落库失败成为未知结果，只产生原 **1 次** HTTP 调用，重入前后逐表快照一致。另以原库改变响应哈希、预算、冻结任务、请求、lease、首项 ACK、待结预算和意外能力，全部在顶层准入阶段拒绝且逐表无业务写入。当前专项和下列完整门禁已通过；这仍不是完整 Master Plan 验收。

新增空前缀真实红绿：根 `used_tokens=1` 的原路径已以 **exit 101** 复现错误业务写入；修复后同一专项 **1/1 exit 0**，且 `reserved_requests=1`、孤立账本同样零 HTTP、逐表快照不变。原超过 1100 行的 Reviewer 测试文件按职责拆为执行夹具 `tests_source_reviewer_fixture.rs`（275 行）、Reviewer 规则测试 `tests_source_reviewer.rs`（538 行）和重入专项 `tests_source_reentry.rs`（312 行）；总测试行数未增加，共用原有真实传输夹具与逐表快照，不复制测试服务。后续全量结果以实际退出码为准。

根工具孤儿专项在空前缀及首项初评后两种状态分别先以 **exit 101** 复现错误继续派发，修复后两个专项各 **1/1 exit 0**；Reviewer `received` 边界也以现有真实夹具验证拒绝，**1/1 exit 0**。三处均要求零新 HTTP 调用、全业务表快照不变。本轮重新编译的完整门禁见下；不能据此推断其他未知副作用可自动恢复。

## 本轮验证

- `CARGO_BUILD_JOBS=1 cargo test --offline -j 1 --all-targets --all-features source_ -- --test-threads=1`：主库 **292/292**、导入器 **2/2**，exit 0。
- `CARGO_BUILD_JOBS=1 nice -n 15 cargo clippy --offline -j 1 --all-targets --all-features -- -D warnings`：exit 0。
- `cargo fmt --all -- --check` 与 `git diff --check`：exit 0。
- `npm run test:ui`：**251/251**，exit 0；`npm run build`：exit 0。
- 首轮工具恢复快照的全量 Rust 主库 **1372** 项、导入器 **30** 项、main target **0** 项已 exit 0；它不覆盖随后新增的一／两项初评准入修复。
- 最新初评修复严格 Clippy、fmt、空白检查均 exit 0；全目标／全特性 Rust 主库 **1373/1373**、历史导入器 **30/30**、main target **0** 项，整条命令 **exit 0**。单作业、单线程主库耗时 1449.33 秒；没有从零测试项的 main target 推断额外覆盖。UI 与构建未修改，前述结果是本轮前端快照。
- 最新 `received` 初评本地交付增量：`CARGO_BUILD_JOBS=1 nice -n 15 cargo test -j 1 --all-targets --all-features -- --test-threads=1` **exit 0**，主库 **1376/1376**（1463.06 秒）、历史导入器 **30/30**、main target **0**。新边界专项 **4/4**；`cargo clippy -j 1 --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check`、`git diff --check`、`npm run test:ui` **251/251** 与 `npm run build` 均 exit 0。单线程低优先级执行；未把中途取消的旧编译进程视为测试结论。
- 空前缀修复的源码筛选回归 **297/297，exit 0**，但该二进制在 Reviewer 测试文件拆分前编译，不能证明最终文件布局。拆分后重新编译的 `source_reviewer_` **23/23，exit 0**；最终布局的 `CARGO_BUILD_JOBS=1 nice -n 15 cargo test --offline -j 1 --all-targets --all-features -- --test-threads=1` **exit 0**：主库 **1377/1377**（1457.56 秒）、历史导入器 **30/30**、main target **0**。严格离线 Clippy、fmt、空白检查均 **exit 0**；`npm run test:ui` **251/251**、`npm run build` **exit 0**，本机 Native 浏览器回环 `node tools/test_native_runtime.cjs` **exit 0**（8 个观察请求）。所有长耗时 Cargo 命令单作业、单测试线程、低优先级串行运行。结构规则已写入 Master Plan §4.1 和 §15：新代码按前后端业务边界拆分、共享夹具复用、禁止以删除唯一安全回归减小体量。该结果仅证明本地当前代码快照的门禁，不能替代 §18 安装包与目标环境验收。
- 根工具孤儿准入修复后，重新编译的完整 `CARGO_BUILD_JOBS=1 nice -n 15 cargo test --offline -j 1 --all-targets --all-features -- --test-threads=1` **exit 0**：主库 **1377/1377**（1434.51 秒）、历史导入器 **30/30**、main target **0**。严格离线 Clippy、fmt、空白检查已 **exit 0**。同一代码快照的 `npm run test:ui` **251/251**、`npm run build` **exit 0**、本机浏览器回环 `node tools/test_native_runtime.cjs` **exit 0**（8 个观察请求）。新增业务文件 `native_source_reviewer.rs` 为 363 行，专项测试 `tests_source_reentry.rs` 为 315 行；测试沿用已有真实 HTTP／磁盘库夹具，未复制服务或删除安全回归。结果只覆盖本地代码及回环，不覆盖 §18 安装包、实际目标与跨平台恢复。
- 显式续跑阶段与初评业务拆分后，重新编译的源码筛选测试主库 **297/297**、导入器 **2/2**，exit 0；全目标／全特性离线 Rust 主库 **1377/1377**、导入器 **30/30**、main target **0**，整条命令 exit 0。严格离线 Clippy `-D warnings`、fmt、`git diff --check` 均 exit 0；前端 **251/251**、构建、本机 Native 浏览器回环（8 个观察请求）也均 exit 0。改动后业务文件行数：`native_source_initial.rs` **40**、`native_source_coordinator.rs` **322**、`native_source_reviewer.rs` **369**、`native_source_tools.rs` **233**；既有专项测试 `tests_source_reentry.rs` **315**，没有删减用例或复制夹具。此轮是已完成前缀的调度收敛，不是系统级强杀、过期 lease 接管或未知外部副作用的完成证明。

## 未完成

### 2026-09-28：已收回执、未交付本地工具的源码子任务

新增源码业务文件 `native_source_tool_reentry.rs`（80 行）和轮次审计子模块 `source_rounds/reentry.rs`（100 行），没有继续向调度入口堆叠回执验证。只有原 Coordinator/child lease 仍有效、前缀完成回执与账本一致、唯一运行中工具 child 的模型轮次均已持久化为 `received` 且最后一轮存在未提交的本地工具时，才以同一 assignment/预算恢复。`executing`/`uncertain`、过期租约、错误的响应哈希、预算漂移均拒绝接管；SourceBroker 工具只在同一数据库事务内重做，不能把未知外部副作用推断为成功。已完成前缀不再重新调度；恢复期间失败保留根状态供下一次只读审计，不强制终态化。

复用已有真实 HTTP/磁盘 SQLite 夹具，新增首个工具子任务第一轮、首个子任务 `assignment.finish` 轮、第二个工具子任务第一轮的中断恢复，以及损坏/过期零写入回归；没有删除原有未知结果安全用例。此项仅覆盖仍有效的原租约以及本地事务内 SourceBroker 工具；**过期接管、系统强杀、未知外部结果人工核对和 Master Plan §18 仍未完成**。拆分前的源码筛选测试重新编译通过：主库 **301/301**、导入器 **2/2**、main target **0**，整条命令 exit 0。纯结构拆分后重新编译的 `source_rounds_` **9/9**、`source_received` **3/3**、`source_second_received_tool_reentry` **1/1**、`source_partial_tool_unknown_result` **1/1** 均 exit 0。此后同一代码快照的全目标／全特性离线 Rust 测试也已完整收取：主库 **1381/1381**（1441.59 秒）、历史导入器 **30/30**、main target **0**，整条命令 exit 0；严格离线 Clippy `-D warnings`、fmt、`git diff --check`、UI **251/251**、`npm run build` 和本机浏览器回环均 exit 0（两次捕获、8 个观察请求）。这只是本机开发树的质量门禁，不等于 §18 的安装包、目标环境和跨平台恢复验收。

结构收敛：原 `source_rounds.rs` 的 947 行已经在同一源码业务目录拆为轮次和预算/回执核验主模块 **387** 行、`dispatch.rs` **279** 行、`tool_delivery.rs` **195** 行、`completion_audit.rs` **104** 行，以及既有 `reentry.rs` **100** 行。保留 `source_rounds` 对外 API 和原 SQLite 事务边界；没有删除安全回归或复制测试夹具，相关测试文件分别为 **383** 与 **308** 行。本次前端无代码改动，不能据此宣称前端体量整治已完成：现存 `SentinelBoard.vue` **4650** 行、`AgentWorkbench.vue` **1597** 行、`AgentDialog.vue` **1119** 行、`SentinelSourceResults.vue` **1022** 行。后续涉及这些区域时先按业务状态、展示组件和共享类型/转换拆分，再新增功能；不可把通用类塞进单一页面，也不可通过删除唯一回归测试缩减仓库。

1. 任意部分源码轮次、过期租约和未知响应的人工核对／恢复流程，操作系统级强杀重启、并发接管以及更广泛的时限／取消／美元账本矩阵。
2. Master Plan §18 的跨 Web／源码／灰盒／CI 全链路、历史资产迁移、真实桌面安装包和目标环境验收。单一恢复边界通过不等于 Strix-free Native Multi-Agent 发布完成。
3. 用户提供的授权 URL 尚未探测或运行，不能据本机夹具声称实际环境通过。
