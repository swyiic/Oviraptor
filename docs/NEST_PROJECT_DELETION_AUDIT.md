# 工作空间删除：事务准入与关联数据保护

状态：本增量已实现并完成完整回归；最终证据见第 5 节。本增量不是 Master Plan、未知效果结案或全通道清理完成声明。

## 1. 产品合同

工作空间只允许在没有关联数据时删除。已有任务、调查证据、知识、配置或其他关联记录时，使用归档保留历史，不通过项目级删除绕过任务级的停止、清理和未结执行义务检查。

- UI 的影响预览只帮助操作员选择，不是删除授权凭据。
- 删除后端必须在同一写事务中重新核对项目存在、关联数据和删除结果。
- 原生扫描采用 `ON DELETE SET NULL` 的关联也必须保护：不允许靠删项目丢失任务归属。其他采用 CASCADE 的关联不允许被误删。
- 不按项目级删除清理全库孤儿资产，不发进程信号，不清理 JSON 或任务产物目录。
- 本次仅在隔离临时数据库运行破坏性测试；未删除真实项目、未部署、未测试外部目标。

## 2. 复现的两个缺陷

原删除顺序是：在事务外读取影响统计 → 判断为空 → 开启 deferred 事务 → DELETE。第二个连接能够在检查与 DELETE 之间插入记录。旧代码仍报告删除成功，关联数据可能被级联删除或解除归属。

同时，显式统计遗漏了公开暴露任务/发现、资产归属配置、调查和知识等部分表。即使完全没有并发，也可能把有数据的项目当作空项目。

先只抽出实际删除调用的内部函数作为测试入口，保留旧执行顺序。两个新增回归均失败，退出码 101：

- `project_deletion_holds_writer_admission_until_commit`：检查结束时第二连接实际写入成功。
- `project_deletion_preserves_previously_uncounted_records`：仅有 `exposure_runs` 的项目被统计为零。

日志：`/tmp/oviraptor-project-deletion-before.log`。这不是仅检查源代码字符串的测试。

## 3. 后端修复

主要代码为 `src-tauri/src/commands/workspace_projects.rs`。

1. 删除连接设置 FULL 同步级别，在任何准入读取前开启 IMMEDIATE 写事务。
2. 在该事务内重新读取项目和全部已建模的直接关联记录；有记录则拒绝，提示归档。
3. 执行精确 ID 的 DELETE，要求影响一行，并验证目标项目确实不再存在。
4. 提交成功才报告成功。数据库拒绝、IGNORE、重新插入项目和可复现的提交约束失败均回滚；不做数据库外的补偿删除。
5. UI 的只读 `project_impact` 命令另使用读取事务，使一次预览中的各分类处于同一快照。删除仍独立重查，不能复用旧预览作为准入。

保留已有具名分类，同时新增 `otherRecordCount`：通过当前数据库 schema 发现未归类的 `project_id` 列，以及直接引用 `projects.id` 的其他名称列。无外键的旧 `project_id` 也纳入；一个表中多个补充归属列采用 OR 计数，不重复累计同一行。标识符来自数据库 schema 并进行引号转义，项目 ID 使用绑定参数；查询失败拒绝操作，不以零作为回退值。

资产归属 profile 由显式保存接口写入，不是每次读取自动创建的默认行。已经保存的授权范围/主体配置属于需要保留的数据，因此也阻止硬删除。不会为让删除通过而清空这些设置。

内部回调仅是确定性并发测试接缝；桌面删除命令传入空回调，没有新增可供 UI/Agent 注入的执行入口。

## 4. UI 和回归合同

`ProjectImpact` Rust/TypeScript 模型增加补充记录数量；实际 `App.vue` 归档卡显示其他调查/知识/配置记录，并补上已有 Nest 目标、测试机会字段的提示。

- 影响查询期间清除旧确认卡、显示检查中、禁止重复查询或确认其他项目。
- 删除期间禁止重复提交、取消确认或换目标。
- 后端因数据变化拒绝删除时保留项目和确认信息，显示错误，不假报成功，也不自动改为归档。
- 查询失败后可以重新查询；真正的归档仍需用户确认。

新增 7 项 Rust 回归位于 `commands/tests_project_deletion.rs`：

1. 第二连接在准入后写入被 BUSY 拒绝；删除提交后外键阻止该连接向已删除项目写入。
2. 仅有暴露任务/发现、归属 profile/rule 或知识策略时，拒绝且数据库记录保持不变。
3. 新表、旧无外键列、异名直接外键、多个归属列及带引号表名被正确统计；具名分类不重复累加。
4. 旧预览后新增各种状态的扫描，删除重查并拒绝，不将项目关联设为 NULL。
5. 其他写事务先取得锁时，删除不能准入；对方提交后仍因记录存在而拒绝。
6. 空项目正常删除，其他项目数据和孤儿资产保留；不存在 ID 不改变数据库。
7. ABORT、IGNORE、AFTER 重新建项目和 deferred FK 提交失败完整回滚，并能在排除故障后重新操作。

新增 5 项实际 SFC 回归位于 `tools/test_project_deletion.cjs`，执行真实 App setup、模板、i18n 和 InlineConfirm；仅隔离桌面启动、无关子面板、IPC 和计时器。覆盖归档文案/动作、后端拒绝、查询竞争、提交竞争、失败重试。不是桌面端到端或浏览器操作验收。

## 5. 验证证据

- 旧逻辑红测：0 通过 / 2 失败，退出码 101。
- 修复后定向 Rust：7 通过，退出码 0，日志 `/tmp/oviraptor-project-deletion-targeted.log`。
- 新 SFC 定向：5 通过，退出码 0，日志 `/tmp/oviraptor-project-deletion-ui-final-targeted.log`。
- UI 全套：125 = 45 + 26 + 32 + 7 + 7 + 3 + 5，完整脚本组合退出码 0，日志 `/tmp/oviraptor-project-deletion-ui.log`。
- 前端构建退出码 0，主 JS 812.62 kB；既有大 chunk 警告保留，日志 `/tmp/oviraptor-project-deletion-build.log`。
- 全 targets/features Rust：主库 960 通过（370.73 秒），历史导入器 30 通过（1.91 秒），无失败；日志 `/tmp/oviraptor-project-deletion-full.log`。7 项新增定向回归是主库 960 项的子集，不重复计数。
- 严格全 targets/features Clippy 通过（8.80 秒），日志 `/tmp/oviraptor-project-deletion-clippy.log`；fmt 通过，日志 `/tmp/oviraptor-project-deletion-final-fmt.log` 为空。Rust → Clippy → fmt 使用 `&&` 的完整命令链实际退出码 0。
- 最终 `git diff --check` 通过。本增量未修改退役字面量允许清单、未放宽既有门禁。

UI 测试接入阶段出现过 3 通过 / 2 失败：夹具先将 i18n 的 computed 字典错误替代为函数，随后还缺少已加载状态及真实 Project 数值字段。修正夹具，改用真实 i18n 并补齐项目字段后，原渲染断言通过；没有放宽生产代码或断言来消除失败。

复现命令：

```sh
cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --all --check
npm run test:project-deletion
npm run build
git diff --check
```

## 6. 明确未覆盖的范围

- 计数依据现有关系 schema，不尝试推断 JSON 内、自由文本中的项目归属，也不自动修复既有孤儿/污染记录。
- 不迁移历史所有权，不新增文件回收或数据库备份/恢复机制。
- 当前 schema 发现与计数优先保证不丢数据；大库性能、未索引新表的查询成本仍需要独立压测，不能用小夹具推断生产规模延迟。
- 不改变主机边界、授权审批或安全预算，不新增 Host Agent。
- 完整未知效果结案/新任务交接、全通道停止和清理、其余多智能体/知识技能/沙箱/安装包验收仍需继续；项目删除修复不是整体完成。
