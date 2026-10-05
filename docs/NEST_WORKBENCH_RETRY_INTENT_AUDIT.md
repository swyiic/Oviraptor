# 工作台重试意图保留审计

后续源码 CI 策略修复见 `NEST_SOURCE_CI_POLICY_AUDIT.md`：重试保留的阈值与开关现进入独立 attempt 的发布策略，而非读取全局默认值；原 scope／实际分析范围未落实的限制仍有效。下文验证数字属于本增量历史终态，不代替后续修复的验证。

状态：本增量已实现并取得最终组合门禁通过终态；首轮完整回归暴露的新增测试字面量问题已修正，失败证据保留。不是 Master Plan 全部完成的声明。

## 1. 修复的实际问题

原 `rescan_workbench_scan` 在任务锁外读取 task JSON，读取或解析失败后默认空对象；人工 instruction 重建为空串，skills 按可变显示名称查 ID，找不到的项被忽略。空列表随后进入新任务的“继承所有启用 skills”逻辑。缺失 mode、scope、预算和 CI 字段也会回落默认值。这会让重试丢失操作者限制，甚至扩大原来的执行范围。

共享认证 fixture 补齐真实生产 policy 后，原认证回归实际暴露另一个问题：把单身份与多身份字段合并解析，会让仍存在的单身份掩盖已清空的多身份列表。本增量同时修正生产发布校验，两个字段必须分别匹配实际身份集。

## 2. 实现与生效边界

- 新任务／重试以显式请求类型进入统一启动函数；先取得 scan lifecycle 锁，再重建重试输入。旧 command 不再自行读取 JSON 或按 skill 名查询。
- 从当前 attempt 对应的 task.json 与数据库 policy 恢复人工指令、精确 skill ID、预算、mode/scope/diffBase、目标、认证句柄和 CI 控制。必需字段缺失、类型不符、身份／任务／项目／attempt／backend／目标不匹配时明确拒绝；拒绝本身不创建新 attempt。
- 新任务空技能选择仍继承启用技能；重试的显式空列表保留为空。选中技能缺失或被禁用时拒绝，不静默替换。改名不改变 ID，仍可重试。
- 工作台 policy 记录实际完整选择，不受普通 Web UI 的 32 项归一化截断影响。原有 Web 内置工作流技能语义不变；显式列表为空不等于禁用内置工作流。
- 准备时捕获原 attempt/status、task 路径与内容摘要、policy 摘要、选中技能当前名称和 instructions 摘要。发布事务在写入前重新捕获并比较；变化则拒绝，旧 ledger 与旧文件保留。技能摘要是本次准备时版本，不是历史首次运行版本快照。
- 原任务 JSON 读取有 16 MiB 上限；Unix 使用非跟随最终符号链接、非阻塞打开并验证普通文件。没有据此声称已解决所有祖先目录替换或跨平台路径安全问题。
- 浏览器认证发布要求 policy 的单身份字段与多身份列表分别准确匹配；旧材料刷新、当前身份有效性和事务末尾材料重查仍由既有认证合同负责。

主要文件：`commands/workbench_retry.rs`、`commands/code_analysis.rs`、`commands/scan_control.rs`、`commands/workbench_startup.rs`、`commands/workbench_auth.rs`。

## 3. 回归与证据

新增 `tests_workbench_retry.rs` 共 12 项生产 helper／事务回归，覆盖：

1. 人工限制、预算、quick/diff、diffBase、CI 控制、目标和身份句柄保留。
2. 空选择不因新增启用技能扩大；新建任务的继承语义不变。
3. 技能改名继续按 ID 恢复，缺失／禁用拒绝。
4. 必需字段缺失不回退到默认值。
5. 任务范围、backend、策略、目标或认证句柄不一致拒绝。
6. 损坏／缺失 task JSON 不产生新账本记录。
7. 准备后 task、policy、status、attempt、技能内容变化均阻止发布，保留旧 attempt。
8. 匹配的准备依据成功发布独立 attempt，不改旧 task 文件。
9. 超过 32 个工作台 skill ID 完整保留，非法 ID 集拒绝。
10. task 与数据库同时含有无效控制值，仍不能获准。
11. 正在执行或 task 路径不属于当前 attempt，拒绝。
12. Unix task 读取器拒绝最终符号链接和目录。

既有认证反例新增单身份字段不一致，未删除或放松原多身份不一致反例。共享 fixture 使用完整 policy 和实际 task 文件；重试测试补齐生产要求的准备依据，未通过删除发布条件使测试变绿。

开发过程保留：

- `/tmp/oviraptor-workbench-retry-targeted.log`：27 通过／1 失败，真实暴露多身份列表被单身份兜底的问题。
- `/tmp/oviraptor-workbench-retry-targeted-v2.log`：新增测试后 36 通过／同一认证反例失败。
- `/tmp/oviraptor-workbench-retry-targeted-v3.log`：修复后工作台组合 **40 项通过**，零失败。
- `/tmp/oviraptor-workbench-retry-clippy.log`：严格全目标／全功能 Clippy 通过；组合 session 47558 退出 0。

以上重试测试为与实现同期新增，不把它们描述成修改前已运行的红测证据。

首轮完整 Rust 流水线 session 67932 已退出 101，日志 `/tmp/oviraptor-workbench-retry-full.log`：主库 **1018 通过／1 失败**，用时 513.43 秒。唯一失败为 REM-012，提示新出现 `tests_workbench_retry.rs`，无 stale／changed 文件；后续导入器／Clippy／fmt 因顺序短路未执行。不能把这个运行报告为完整门禁通过。

前端流水线 session 88099 已退出 0：七组实际 SFC 测试 146 项通过，vue-tsc／Vite 构建通过，localhost 实际浏览器匿名采集／身份隔离测试通过（8 次观察请求）。日志为 `/tmp/oviraptor-workbench-retry-ui.log`、`/tmp/oviraptor-workbench-retry-build.log`、`/tmp/oviraptor-workbench-retry-loopback.log`；主 JS 828.82 kB，超过 500 kB 的拆包警告仍保留。

失败原因是新增通用错误 backend 测试直接使用了未登记的退役后端名称。确认 session 67932 终止后，将该值改为 `unsupported-backend`，继续验证非 Native backend 被拒绝；没有扩大 allowlist 或降低检查范围。

最终顺序流水线 **session 52449 退出 0**：

- retirement 定向 **4 项通过**，日志 `/tmp/oviraptor-workbench-retry-retirement.log`。
- `cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1`：主库 **1019 项通过**、历史导入器 **30 项通过**，零失败；主库用时 436.17 秒，日志 `/tmp/oviraptor-workbench-retry-full-v2.log`。包含本增量全部 12 项、既有真实多智能体执行／用户指令／独立 Reviewer／mailbox 与退役门禁回归；不据此替代打包和真实授权环境验收。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` 通过，日志 `/tmp/oviraptor-workbench-retry-final-clippy.log`。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --all --check` 通过，日志 `/tmp/oviraptor-workbench-retry-final-fmt.log`（空输出）。

两轮全量分别运行至终态，期间未修改／重编译 Rust；只在第一轮已退出后修改错误 backend 测试值。`git diff --check` 通过。未更改前端行为，前端／构建／浏览器流水线证据见上文。

## 4. 兼容和仍需继续的工作

- 缺少可验证 task／attempt／policy 数据的旧任务拒绝原地重试；历史导入与回看能力不受影响。操作者需显式创建新的 Native 任务，不能用旧 JSON 自动恢复执行授权。
- 这是重试重建与发布前检查，不是工作台提交后的完整冻结派发 binding、每次工具调用前权限重验、在途撤权或未知效果恢复。
- 当前技能内容允许在两次任务之间更新；准备期间变化会拒绝。这不是历史技能内容永久冻结或技能进化质量验收。
- 手工 cookie/bearer/header 的完整生命周期元数据、source／greybox 完整恢复、所有专家执行链、skills 质量治理、资产分析及桌面／授权环境验收仍需继续。
- 后续只读核对发现：`native_source_scan.rs::run_native_source_scan_using` 当前按 `diff_base` 建立／恢复快照，尚未接收并执行保存的 scopeMode／scanMode／人工限制合同；CI 投影还固定使用 `CiScope::Diff`。本增量证明这些输入在重试中保留，不证明所有源码分析步骤已经落实它们。需另行实现 scope 合同、实际 analyzer 入参和 CI freeze 的一致性，并以不同 scope 的真实文件集合／拒绝行为验证。
- 本增量不授予主机权限，不访问外部目标，不修改历史导入执行权限，不替代总体 Master Plan 逐项验收。
