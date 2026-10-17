# Web 确认／恢复／重试启动原子性审计（2026-09-26）

后续增量说明：本文保留完成时的 828 项基线和历史边界。之后已补入同 startup 事务登记的分支持久派发凭据、一次领取及真实子进程强杀防重测试，见 `NEST_BRANCH_DISPATCH_RECEIPT_AUDIT_2026-09-26.md`。下文“尚无派发 journal”按该后续记录更新；它仍不代表完整配置恢复、外部未知结果对账或桌面崩溃恢复已完成。

## 1. 范围与结论边界

本增量修复普通 Web 任务从草稿或上一轮状态进入新执行批次的同步交付路径，不表示 Master Plan 或 Stage 7 全部完成。

所有测试仅使用临时 SQLite、临时本地目录以及既有本地 HTTP／浏览器回环。不访问用户提供的外部 URL，不部署，不启用主机执行器，不放宽 `web_only`、目标授权、环境准备互斥、身份控制或预算门禁。

特别区分两类故障：

- **提交前的已知错误**：事务回滚；只尝试清理本次独占创建的目录内已知启动文件。故障注入验证了正常文件系统条件下的清理；真实磁盘权限／I/O 故障下的清理不是无限重试或必然成功的承诺。
- **提交返回错误或进程崩溃**：不假定数据库一定未提交，不删除可能已提交执行所依赖的文件，不派发错误返回后的 worker。普通 Web 启动尚无通用持久化派发 journal／重启恢复器；提交后、worker 派发前强杀仍可能留下 `scanning` + pending branch，不能称为已恢复。

## 2. 本轮确认的缺陷

原 `confirm_sentinel_scan` 在实际目录分配前，用旧 `attempt_count.max(1)` 冻结后端计划；实际新执行目录可能是下一批次或更后面的批次。计划写入使用额外数据库连接，后续 task、targets、attempt、当前结果面、native branch 分步提交。

原路径还存在：

1. 熔断更新忽略错误，且把已归档熔断历史当成有效限制。
2. task JSON 反复覆盖共享 `sentinel-tasks/{scan_id}.json`。
3. 启动状态无原状态／旧批次条件检查，两个窗口可能重复准备。
4. 分支登记失败时，前面的 `scanning` 与账本已落库，但没有 worker。
5. 恢复失败时无条件写回 `paused`；重试失败时无条件恢复目标行，可能覆盖竞争者状态。
6. 草稿取消在删除任务成功前先删身份／文件，后段失败会留下局部修改。

## 3. 交付实现

### 3.1 单一 Web 启动入口

`confirm_sentinel_scan`、Web `resume_sentinel_scan`、`rescan_sentinel_scan` 均调用 `start_web_scan`；以 `Confirm`、`Resume`、`Retry` 区分用户操作，不通过先提交草稿再调用确认的方式串联。

同 scan 的确认／恢复／重试／暂停／取消／删除共用 OS 活调用锁；锁文件不删除，不把锁文件存在当作派发收据。普通 Web 启动持有锁至提交和派发调用完成。

本增量**没有把源码／CI 工作台启动改造成同样的事务**。共享分支登记函数加固适用于其既有调用，但不能由此宣称源码启动也完成了全程原子化。

### 3.2 一次 IMMEDIATE 事务

以下步骤在同一数据库写事务中完成：

1. 重验项目 active、任务存在且未 tombstone、Web 类型及当前状态。
2. 恢复／重试所需状态与目标队列准备，执行模式登记。
3. 补证来源与控制组预检；必须在新 attempt 目录建立前预测批次。
4. 有效熔断排除和完整目标快照。
5. 按任务计数、执行账本上界及已有 attempt 文件／目录选择真实新批次；不复用历史或未确认目录，溢出拒绝。
6. 冻结本批次后端计划，持久化运行期有效策略。
7. 条件更新任务状态，更新本次实际目标的 `last_attempt_number`。
8. 用严格 INSERT 建立新 attempt；不 UPSERT 覆盖旧执行账本。
9. 当前结果面切换和后端计划投影。
10. pending Web branch 登记；验证实际任务、目标、策略、账本、计划、模式标记和分支内容。
11. 提交成功后才调用 `launch_sentinel_url_pipeline`。

失败不再通过另一个连接进行“猜测式恢复旧状态”。SQLite 回滚恢复事务前的状态，包括恢复／重试先前的终态、原因、目标路由和历史结果。

### 3.3 后端与历史兼容

新 initial／fresh Web 批次只生成 Native 计划；不访问、安装或执行 Strix。

resume 必须从明确上一批次账本读取冻结计划，验证 scan、显式 attempt、完整 targets 和 URL 唯一性，覆盖所有待续跑 URL。缺行、损坏、目标缺失或退休后端不能悄悄变成 fresh Native 启动；明确返回不兼容。已封口旧执行也不能通过 Retry 的 resume 模式复活。

原 JSON 和旧目录保留只读兼容；新执行计划写入 `agent-jobs/{scan_id}/attempt-NNNN/task.json`，与身份文档、目标列表、instruction、prompt audit 同批次保存。

### 3.4 文件所有权及取消

新目录使用 `create_dir` 独占创建；Web scan ID 仅允许有限安全字符，检查 `agent-jobs` 和 scan 目录不是符号链接。Unix attempt 权限为 `0700`。

已知失败清理检查目录类型、scan marker，以及 Unix 设备号／inode。仅清理本次固定启动文件名，最后尝试删除空目录；不递归删除历史目录、不删除任意数据库 `task_path`、不追随替换的根目录。空的 scan 根目录可以保留。

草稿取消的身份删除与任务删除也改为一个 IMMEDIATE 事务并检查实际删除结果。启动成功的任务不能再被按草稿取消。取消不再根据历史 `task_path` 擅自删除磁盘文件；彻底删除任务仍走已有删除入口。

## 4. 新增回归测试

`src-tauri/src/commands/tests_web_startup.rs` 共 14 项测试：

1. 已有 attempt-0004 时新计划、目标、账本和目录全部绑定 attempt 5，历史文件保留；再次确认不增加批次。
2. 七类写入的 ABORT／IGNORE 故障矩阵，完整数据库快照回滚和本次文件清理。
3. 分支写入后的删除／内容篡改矩阵，包括账本、策略、任务、目标、项目、tombstone、结果面标记、计划和分支报告。
4. 暂停恢复、partial 续跑、completed fresh 重试的晚期失败原状回滚及成功对照。
5. 缺失／损坏／错 scan／缺目标／退休后端的 resume 计划拒绝，不能偷偷 fresh。
6. 已归档熔断不阻断，生效熔断继续排除。
7. 已删除、归档、错误任务类型及熔断静默写入失败拒绝。
8. 两线程同时确认只有一个执行批次；同 scan 活调用锁互斥并可释放。
9. 延迟外键导致 COMMIT 失败时保留文件，数据库无局部提交；后续显式启动使用新目录。
10. Unix 符号链接根拒绝、同名目录替换不被误删。
11. 重试准备时队列／模式／状态静默写入失败和目标状态篡改不能丢失目标。
12. 账本比 scan 计数更大时不覆盖历史；批次编号耗尽拒绝。
13. 草稿取消故障原子回滚、取消后启动拒绝、历史 task path 文件保留。
14. 已提交执行不能按草稿取消，状态和文件保留。

上述测试调用生产准备／持久化／取消核心，而非只检查源代码字符串。测试启动 harness 不创建 Tauri App，也不派发真实 worker；完整桌面点击、IPC 响应丢失及进程强杀不能用这些单测替代。

## 5. 最终质量门禁

最终结果全部基于本轮代码，日志前缀 `/tmp/oviraptor-20260926-web-start-`：

- `cargo test --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1`：主库 **828** 项全部通过（269.93 秒），历史导入器 **30** 项全部通过（1.76 秒）；`final-full.log`。
- 启动专项 `cargo test --offline --manifest-path src-tauri/Cargo.toml --all-features --lib web_start -- --test-threads=1`：**14** 项全部通过（5.94 秒）；`final-targeted.log`。
- 严格 `cargo clippy --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`：通过（12.43 秒）；`final-clippy.log`。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`：通过；`final-fmt.log`。
- `node --test tools/test_agent_dialog.cjs tools/test_agent_workbench.cjs`：实际 SFC **70** 项全部通过；`ui.log`。
- `npm run build`：通过；`build.log`。主 JS **791.55 kB** 拆包警告仍保留，不宣称已优化包体。
- `node tools/test_native_runtime.cjs`：通过；本地浏览器回环 **8** 请求，匿名与身份对照完成、`isolatedIdentity:true`；`native.log`。
- `git diff --check`：通过；`diff.log`。

早期全量快照（826 项）有一项 REM-012 白名单失败，原因是新增测试文件内的三处退休后端 JSON 负例未登记。逐处检查确认仅测试缺损／退休父计划拒绝，没有生产回退；随后仅添加该文件的 fixture 理由、完整 SHA-256 和出现次数，不修改扫描范围或降低门禁。最终 828 项全量已包含通过的 REM-012 检查；旧 `full.log` 作为排错记录，不作为绿色验收凭据。

## 6. 尚须继续

- 通用 Web startup 持久派发记录、提交前／提交后强杀分类、孤立私有文件安全处理及显式恢复 UI。
- Source／CI／组合任务入口的完整竞争与数据库／文件交付审计。
- 终态／跨 fencing 通用对账、完整多角色补证与授权流程、非 HTTP 证据可信来源验证。
- Master Plan Stage 9 其余专家、Stage 10、资产／知识／skills 生命周期、沙箱工具供应与性能、桌面打包及真实授权环境验收。

本轮不标记总体目标完成。
