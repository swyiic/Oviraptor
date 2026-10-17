# 普通 Web 未派发尝试的人工恢复（2026-09-26）

## 1. 结论与范围

本增量承接 `NEST_WEB_TOOLCHAIN_BINDING_AUDIT_2026-09-26.md`，交付一个有界入口：**人工确认后，检查并派发从未取得执行权的普通 Web 原尝试**。不是自动恢复，不是忽略门禁，不是重置已领取凭据，也不是建立新 attempt。

当前主机边界不变：Web 任务仍为 `web_only`，没有新增 Host Agent、Linux 远程执行器或主机审批后端。是否在内网、是否已有 Linux 机器，不会自动扩大 Web 任务权限。将来如需主机验证，必须另行批准精确主机、动作、身份、时窗和预算，并使用独立任务／attempt；现有聊天或本恢复按钮都不能完成这个升级。

本轮没有部署桌面安装包，没有访问用户提供的外部 URL。整个 Master Plan 仍未完成。

## 2. 操作者看到的流程

1. 任务详情的 Native 执行分支卡读取真实派发回执。
2. 后端的只读提示认为任务符合基本恢复条件时，显示“检查并恢复未派发任务”。缺失字段的旧 API 不显示按钮。
3. 点击后显示任务 ID、原 attempt 编号和范围说明。此时不派发。
4. 操作者点击“确认检查并派发”，才调用 `recover_never_dispatched_web_attempt`。该命令不是 Agent tool，不接收模型生成的授权合同。
5. 后端进行完整检查并提交执行权；随后把同一个 guard 交给工作线程。
6. 返回“已提交原尝试的派发”。这不证明进程仍在运行，也不证明扫描成功，后续状态以实际执行回执为准。

轮询、打开页面、刷新状态均不执行恢复。旧页面的异步结果不能修改新任务／新 attempt，也不能重新启动它的轮询。提交中不允许重复点击。IPC 错误或回执不匹配显示“未确认”，刷新只读状态，不自动重试。

`manualRecoveryAvailable` 只是低成本的检查入口提示，**不是完整授权结论**。它不在每次轮询中读取工具二进制或认证文件；文件损坏、身份过期、工具变化等可以在点击后被拒绝。

## 3. 后端契约

### 3.1 所有权与持久化顺序

`operatorConfirmed=true + 精确 scanId/attemptNumber`
→ 生命周期锁
→ 分支调用锁
→ `synchronous=FULL` / `IMMEDIATE` 事务
→ 范围检查及只读重建／HMAC 校验
→ 原派发凭据空值 CAS 为已领取
→ 再次范围检查及重建／校验
→ 提交
→ 转交已持有 guard
→ 创建工作线程。

提交前不启动 Node／浏览器、不发送模型／目标请求、不安装工具、不改配置、不重新生成密钥或启动文件。claim 前后的检查使用同一事务；数据库 trigger 导致的变更也要被第二次检查识别并回滚。

恢复入口持有生命周期锁直到派发交接。工作线程通过公共 `claim_web_pipeline_branch` 取得转交的 guard，不进行第二次 CAS；数据库路径、任务、attempt、分支不匹配时拒绝交接。原启动与人工恢复竞争时，共用分支锁和持久凭据，只有一个调用者可取得执行权。

线程创建使用可返回错误的接口；创建失败不会清空派发凭据。已提交但无结果的情形不能因没有活锁而被认定为从未执行。

### 3.2 基本恢复范围

必须同时满足：

- 当前精确 attempt，scan 和 attempt 均为 `scanning`，唯一分支为 pending Web。
- 普通 `scan_type=web`，无源码路径；项目活跃，任务未软删除，无环境准备独占租约。
- 存在有效版本的私有启动绑定；持久派发回执从未领取。
- 模型请求／输入／输出／缓存／总 Token 计数未偏离本 attempt 起点。
- 当前 attempt 没有 Agent run，没有登记的目标进程，没有未确认的容器清理义务。
- 不是缺口补证派生任务，当前 attempt 没有跨对象授权控制组。

最后一条是**明确的首版限制**：这些任务仍需单独设计复核和恢复，不能借普通 Web 恢复继承另外的授权合同。源码／CI／混合任务同样不受本入口支持。

### 3.3 重建不能替换原输入

- 从当前 attempt 的账本读取工作目录和原 backend plan；目录必须是应用数据下该 scan 的精确 `attempt-NNNN` 规范路径，task_path 必须匹配。
- 只允许已知启动文件和可安全读取的诊断日志。发现额外 worker 输出、日志链接或特殊文件，交人工核对。
- 当前 attempt 的目标必须为 HTTP(S) URL，不能包含 URL 内嵌账号密码；目标集合与 task.json、targets.json、backend plan 必须一致。
- backend plan 必须完整解析，不能用 `filter_map` 静默丢弃坏行；必须为当前任务／attempt，所有目标为 Native，Node／浏览器需求齐全。
- 使用只读 runtime resolver 重建模型、技能、代理、预算、路径和前端配置，不重写 policy。
- 比较 HMAC 绑定的数据库、文件、settings、模型、工具入口、环境和应用构建；变化直接拒绝。
- 重查有效熔断区、会话有效期、状态、凭据和项目归属。
- **本增量另补充任务身份归属与目标主机检查**：普通 Web 初始派发与人工恢复均逐目标调用 `validated_scan_identities`。仅同项目、同凭据不够；会话被转给其他任务或 scopeHosts 不覆盖当前目标时拒绝。
- 至少存在可执行的 Node 和浏览器候选入口。预检只检查文件身份／可执行位，不运行 `node -v`，不下载或替换工具。

工具存在不等于工具健康或版本兼容；执行失败仍须如实记为失败／未完整完成。共享库、浏览器 framework、二级依赖、全部环境以及校验到 spawn 的宿主替换窗口仍未被不可变环境解决，本功能不是完整沙箱。

## 4. 代码定位

- `src-tauri/src/commands/web_dispatch_recovery.rs`：只读提示、恢复范围、生命周期与分支持有、精确计划重建、异步命令。
- `src-tauri/src/commands/web_dispatch_binding.rs`：既有启动 HMAC 和新增逐目标任务身份校验；`WebDispatchAdmission` 可携带已领取 guard。
- `src-tauri/src/commands/scan_execution.rs`：原启动与恢复共用的 guard 交接和可失败线程创建。
- `src-tauri/src/commands/native_scan_branches.rs`：只读恢复入口提示，`automaticReplayAllowed` 始终为 false。
- `src/features/sentinel/components/NativeRunStatus.vue`：两步人工确认、真实回执措辞、防重复和异步换页隔离。
- `src/features/sentinel/api.ts`、`src/types.ts`、`src-tauri/src/lib.rs`：类型和 IPC 注册。
- `src-tauri/tests/backend_retirement_allowlist.json`：人工审阅新增恢复测试后登记完整文件摘要；唯一旧后端名称仅用于临时数据库坏计划的拒绝回归，文件只由 `#[cfg(test)]` 模块引入，不授权任何生产后端。

## 5. 验证证据

新增 10 项 Rust 回归（其中日志链接测试限 Unix）：

1. 必须显式确认、必须精确 attempt；重复状态读取不领取执行权。
2. guard 只交接一次，生命周期锁保持到交接，不创建新任务或重置预算，释放锁后仍拒绝重放。
3. 非普通 Web、进度变化、项目归档、暂停、附加分支、现存进程、补证合同和绑定缺失均拒绝。
4. claim 前 ABORT／IGNORE、claim 后目标／用量／熔断篡改、延迟外键 commit 失败均回滚且不交付 guard。
5. 原启动与恢复真实线程竞争，只有一个赢家。
6. task／targets／额外输出、后台计划坏行／旧后端／错 attempt、错目录、丢失密钥拒绝。
7. 缺工具、无执行位、缺 descriptor 拒绝，不做安装或替代。
8. 真实 bundled worker／runtime／HMAC 的完整只读重建；机器缺工具时验证精确的缺工具拒绝，而非跳过或假装成功。
9. 任务身份改归属、过期、失效、目标主机不匹配、认证材料改变拒绝，原认证文件不被改写。
10. 旧日志符号链接不得重定向 worker 写入。

扩充既有身份绑定回归；新增 12 项实际 Vue SFC 回归，覆盖两步确认、轮询不执行、未知或已领取回执、重复点击、换任务／换 attempt 的晚到成功和失败、确认期间状态变化、IPC 未确认、卸载和回执错配。

中间全量快照为 864 通过／1 失败：`retirement_literal_allowlist_matches_reviewed_sources_and_packaged_inputs` 检出新增测试文件含旧后端拒绝 fixture，但尚无人工审阅登记。确认唯一引用只用于拒绝旧后台计划、无旧后端加载／执行后，登记该测试文件完整 SHA-256 与审核理由，未修改退役扫描范围或放宽校验。此中间快照还不包含后补的诊断日志链接回归，不能作为最终快照。

最终本地门禁已通过。日志前缀：`/tmp/oviraptor-20260926-web-recovery-`，最终 Rust／Clippy／fmt／diff 使用 `verified-` 前缀。全量命令退出码为 0；该结果仅覆盖这里记录的本地回归，不代表完整产品、桌面强杀或真实授权目标验收。

| 检查 | 当前结果 |
| --- | --- |
| fmt / 严格全 targets、features Clippy | 通过，`verified-fmt.log`／`verified-clippy.log` |
| 离线全 targets、features Rust | 主库 866／历史导入器 30 通过，0 失败，`verified-full.log` |
| 实际 Vue SFC | 88 通过，0 失败，`ui.log` |
| 前端构建 | 通过；主 JS 795.73 kB 拆包警告保留，`build.log` |
| Native 本地浏览器回环 | 通过；8 请求、身份隔离 true，`native.log` |
| diff 空白检查 | 通过，`verified-diff.log` |

## 6. 下一步，不得假定已完成

1. 已领取、缺失历史派发凭据、IPC／commit／线程创建后结果未知的人工对账；绝不能清空 claim 来“修复卡住”。
2. 配置已变化但原 attempt 尚在 scanning 的解释与安全结案／新 attempt 流程；本恢复入口只拒绝，不擅自终止旧任务。
3. 补证／跨对象授权控制任务的专门恢复，以及源码／CI／combined 的完整启动原子性。
4. 运行期间的完整撤权和跨层终态对账、实际桌面强杀／断电后的端到端验收。
5. 不可变工具供应和轻量 Linux 沙箱、性能测试、安装包和真实授权环境验收。
6. Master Plan 的其余专家协同、资产与知识／skills 生命周期、UI 整体优化及 JS 拆包。

现有 Strix 退役和历史 JSON 只读兼容要求不变。不得因为本恢复被拒绝而启动旧后端、改用宿主 shell，或把任务升级到主机测试。
