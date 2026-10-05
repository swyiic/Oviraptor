# 原新 Multi 财务退出凭证审计（2026-10-04，受限范围已实现）

Master 仍未完成，Goal active。本批完成 REM-A01 的原新 Multi 财务退出凭证基础与两个实际回放/准入问题；没有实现完整 Multi 付费物理删除，没有开放历史终态补凭证或显式续跑。

## 修改前的问题与原数据作用域

原新 Web creator/HMAC/Root/C1/十维硬限额 → 实际 localhost SDK → 原 Tick、用量与费用 → 原 `finish_coordinator_run`。在这个真实生产调用链的临时 SQLite 中，修改已经退出 Root 的 `terminal_reason`，然后用修改后的原因重复 finish，原代码接受；实际 0/1（0.74 秒）。原终态事务里的 `FinalClock` 只保存内存证明，重入从当前可变终态字段重新捕获，缺少原退出原因的持久依据。

新增凭证后又单独证明：把已付、已退出 Root 的旧状态改回 `prepared`、清空 `finished_at`，原 `RootOwner::require_executable` 仍接受。实际 0/1（0.78 秒），红停在准入断言，不能声称红阶段已执行第二次 SDK。最小在原 Root 准入拒绝已有 Multi 退出凭证；修后原初始化/Tick 均拒绝，全行保持、实际 SDK 总数仍 1。

以上正向依赖原 creator 和真实 SDK transport，不用旧 SQL 标签、测试财务初始化器或角色 mock 代替出生。损坏/旧标签仅用于临时负向故障。模型是本地脚本响应，验证传输、费用与合同，不验证真实模型推理质量。没有访问两个外部验收 URL，没有触碰真实 DB/CAS/资产/安装目录，没有提交。

## 最小实现

- 新 `agent_multi_exit_receipts` 只绑定原 Root/control，唯一 receipt/root 与三种不可变 trigger；FK 指向原不可变 financial control，没有对可删除 run 的 FK，也没有新增活 Root、lease、grant、worker、权限或 resume 行。启动只增加新表/trigger；不迁移、清理或修复既有终态数据，不改 Tick/timeline 的原 RESTRICT。
- 原 `FinalClock::seal` 在同一私有 IMMEDIATE/FULL 原退出事务中、原 SDK idle 检查及指令关闭之后持久保存原 cutoff、终态/错误码/已脱敏原因、原 Root/C/硬合同来源、物理 Root 与终态事件依据、完整费用依据及十维 reserved/consumed/indeterminate 数值。INSERT 改变数和原规范字节必须确实读回；IGNORE 或触发器失败连同终态和本批费用一起回滚。
- 原 writer 仅增加主连接直接 INSERT 新凭证表；业务和跨 Root 的 trigger 写仍拒绝，调用方原 hook 不替换。现有 Source 后续发表仍核验原 financial cutoff；已有 `verify_closed` 保留原活 C 校验，未弱化发表或执行准入。
- 独立 `FinalClock::verify_original_exit` 必须持有一致读事务，严格版本/字段/规范 JSON、原 control、物理 Root、原终态事件、十维及原费用来源核验。私有对象内复用原 financial 证明，返回仅 `()`；不采样到现在、不退款、不释放未知、不写修复、不返回 Connection/RootOwner/C 或可执行能力。自然过期的原 C 可以只读核验；原执行准入与 SDK 仍拒绝。
- 原终态重入只读原凭证，缺失时 `budget_multi_exit_original_receipt_missing`，不回填。原无 Root control 的下层 worker 财务合同不生成新凭证，不能冒充原新 paid Root 出生或删除证明。已有 paid Root 缺失原凭证的历史问题保留为核对债。
- 退出凭证存在即阻止 Root 重新准入，错误码 `budget_multi_continuation_requires_explicit_contract`。即使旧状态被改成开放也不恢复活权；显式续跑合同仍是 REM-A02 待开发项。

本凭证是原始财务退出的不可变依据，引用完整原行的物理 hash；它没有单独备份所有被删来源，也不是删除后的完整审计包。不能只保留本凭证的 hash 后删除 run/事件/worker/source 来假造物理删除完成。后续必须保存全部原来源并重新验证。

## 负向与验证

新增 10 项功能合同最终 10/10（12.45 秒）：已付原因修改拒绝；已退出 Root 改旧开放标签仍不准入；正常原 finish/重放全行/文件/SDK 数保持；自然过期 C 的真实只读连接核验与零新 SDK；真实 503/usage 缺失保留原未知费用、不允许结清或删除；IGNORE/业务/跨 Root 三类故障回滚整个退出；凭证丢失恢复精确不可变 schema 后仍拒绝并不回填；UPDATE/DELETE/两种 REPLACE 拒绝；原终态事件及物理 Root rowid 改动拒绝；schema 弱化/未知 JSON 字段/外来 control 拒绝。临时故障移除 trigger 仅在隔离测试中用于证明损坏拒绝；生产代码不移除任何防篡改 trigger。

初次实现编译遇到一处 rusqlite 迭代借用生命周期错误，保存迭代结果后修正；不算功能红。核心原因篡改修后 1/1（0.68 秒），扩大 9/9（11.63 秒），第二个准入红修后最终 10/10。

最终相关门禁：严格 all-target/all-feature Clippy `-D warnings` 0（30.28 秒）；测试编译 2273 项；逐名 531/531（测试 833.95 秒，命令 834.17 秒），选中集合与实际通过集合完全一致，无忽略；导入 39/39（命令含编译 42.31 秒）；exact 退役 1/1（1.29 秒）。包含上一批 Single 付费删除、Root/角色反馈/指令关闭、原 clock/exceptional exit 和 Source 财务/结果回归。不是完整 Rust/UI/fmt/Native JSON 黄金门禁、安装验收或实际模型质量验收。

日志与逐名清单：`/tmp/oviraptor-multi-exit-final-r1-{gates.json,affected-names.json,compiled-test-list.txt,clippy.log,compile.log,affected.log,importer.log,literal.log}`。本轮选择器最初模块前缀与编译器实际 include 命名不一致，脚本在运行前拒绝；改用实际编译列表逐名匹配，不删选中项、不忽略测试。

## 未提交改动保护

本批 11 个代码路径，5 已有/6 新；先保存原全文及原 Git diff，然后最小增量合入。逐文件完整增量 diff 已复核，1290 个原范围外文件保持，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 未变，diff--check 0。当前 1301 文件集合 SHA `dcee8d177525e74ffb5eb8cb9dd94262936d1fc16297f56b274b321854863162`。

既有增量：`budget/root.rs` 准入守卫；`clock/finalization.rs` 原 seal 与纯财务核验拆分；`clock/closure_writer.rs` 精确写表；`db.rs` 单处加性 schema；`commands/agent_tests.rs` 三 include。新增：`clock/finalization/exit_receipt.rs` 及其 `proof.rs/schema.sql`，三份 `commands/agent_tests_multi_exit_receipt*.rs`（52/156/150 行）；新增生产模块 152/82 行。

保护证据：`/tmp/oviraptor-multi-exit-receipt-{baseline.json,before.json,prior-diffs,scope-final.json,code-snapshot.json,reviewed-merge-diffs,reviewed-files.json}`。文档单独保存原字节与差异记录，原历史完整保留，不重置/覆盖工作树，不自动提交。

## 剩余与风险

REM-A01 仍未完成：Tick/timeline 的原 RESTRICT、原 worker/request/费用/退出/事件/快照/source 的完整独立留存、所有执行义务与 owned/branch finalizer 的真实闭合、Multi 原子物理删除、提交前进程死亡/提交后丢回复/重复删除、受保护业务关联、旧 attempt source、审计 UI/规模/未来 schema。

本批财务凭证不是所有 HTTP/工具/浏览器/进程/父监督退出证明；未知费用即使有财务退出仍不可当结清。原财务/C 来源改变、迟到费用/显式核对的新阶段及 future schema 仍须新合同，不能静默重签旧凭证、采纳 successor C 或删除历史来源。历史 paid terminal 缺原凭证不会自动回填，真实数据迁移先精确盘点与备份；真实资产不删除。

其余 REM-A02—14 全部继续：十维动态 grant/精确对账/显式续跑，六类监督触发、15 角色/General ReAct/Broker/真实并行，证据/独立 Reviewer、聊天/逐路实时日志/整体 UI、非 Web 与旧回归/数据知识；最后完整门禁、同源安装 App 打开、已授权 URL 匿名只读和真实模型质量。InputParser 原自动审批拒绝保持，完整原因不可见，不重试或绕过。Goal active，不标 complete、不暂停。
