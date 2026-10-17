# Executor 原审核余额与有限纯重放审计（2026-10-05）

Master/Goal仍未完成。数据作用域为临时creator→原Root/Mapper/changed-fact SDK→原schedule→实际runtime window，三次付费调用；无Web Executor SDK、目标HTTP、真实供应商或安装验收。

## 问题与修改

原prepare在发放事务外按8,000tokens留底，并从陈旧余额提取Executor预留，违背原LocalDeliberation Reviewer15,000/1。在60,000/5原creator场景三次SDK各20费用后只留8,000，原负向1/1日志`/tmp/oviraptor-executor-allocation-red.log`。新建Bootstrap3规则固定按原Reviewerfloor和有序proposal，从同一IMMEDIATE事务当前gross余额分配；陈旧调用者额度不构成权限。原grant四类不可变Reserve及第一worker/C/RootOwner固定pair，运行窗口使用实际pair。已有Native1/2/缺Bootstrap保持原规则，不升级旧Root或引入旧格式执行。

原纯调度重放被fresh-target检查自己挡住。仅verify_scheduled_authority后排除同一原run/assignment，所有其它target记录（包括只留run或assignment）仍拒绝；普通prepare无豁免。只查询同一活worker，不续租、补授权、换代、重发SDK、释放退款或恢复target会话。

## 证明与负向

有限44,940/1额度、余15,000/1；混合无限实际窗口(0,1)/(36,940,0)，Native0仍为无上限。完全无限分配只读(0,0)，utility仍拒绝，不计为执行通过。扩大初次1/3、修后2/3失败以及宽松诊断日志保留，最终改为确切逐配置断言，未放宽生产评分。

七新增具名测试：实际三SDK完整prepare；陈旧提示及原grant重放零写；混合/完全无限原规则与拒绝；混合无限实际prepare窗口；八损坏/失效/外国target历史；三忽略写入/恶意业务写故障回滚；两个线程同paid frame单原worker/四类Reserve。原费用/日志/权限及typed rowid保持。并发发放无真实Executor SDK重叠证据。

最终同源码关联78/78（测试136.73s/阶段150.67秒，选择=报告=通过）；严格all-features/all-targets Clippy0（21.76s）；同二进制退役50/50（5.09秒，含literal/当前Native JSON）；六叶局部fmt与范围diff0。无ignore。当前2371项Rust全量、整体UI/安装IPC/授权URL/真实供应商质量仍未验收；不累计局部结果为整体通过。

12代码路径8已有/4新均保存逐文件前像、原Git差异并审查最终增量；1366原范围外源码及HEAD59be3d86保持，1378源码集合SHA 11e28f1a30b70d787f2f5073416e1e5203d0605e5e58e8f47e8f2a117a3e3090。原费用生产者/写入权限/SDK释放机制及所有已有入口测试保持，原合同单元测试增补版本2原字节回环与版本3字段拒绝。仅临时Git/SQLite/CAS/localhost脚本SDK，无真实DB/CAS/asset、UI/安装/URL操作或自动提交。

## 逐文件作用域

- `src-tauri/src/commands/agent_tests.rs`：已有增量，207行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__commands__agent_tests.rs.diff`。
- `src-tauri/src/commands/agent_tests_executor_allocation.rs`：新建，241行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__commands__agent_tests_executor_allocation.rs.diff`。
- `src-tauri/src/agent_runtime/web_mode/root/bootstrap.rs`：已有增量，105行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__agent_runtime__web_mode__root__bootstrap.rs.diff`。
- `src-tauri/src/agent_runtime/multi_agent/budget/model/mapper_allocation.rs`：已有增量，73行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__agent_runtime__multi_agent__budget__model__mapper_allocation.rs.diff`。
- `src-tauri/src/agent_runtime/multi_agent/budget/model.rs`：已有增量，327行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__agent_runtime__multi_agent__budget__model.rs.diff`。
- `src-tauri/src/agent_runtime/multi_agent/budget/model/web_allocation.rs`：新建，70行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__agent_runtime__multi_agent__budget__model__web_allocation.rs.diff`。
- `src-tauri/src/commands/agent_native/coordinator_executor_allocation.rs`：新建，65行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__commands__agent_native__coordinator_executor_allocation.rs.diff`。
- `src-tauri/src/commands/agent_native/coordinator_dispatch.rs`：已有增量，117行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__commands__agent_native__coordinator_dispatch.rs.diff`。
- `src-tauri/src/commands/multi_agent/prepare.rs`：已有增量，351行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__commands__multi_agent__prepare.rs.diff`。
- `src-tauri/src/agent_runtime/multi_agent/scheduler/bootstrap.rs`：已有增量，250行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__agent_runtime__multi_agent__scheduler__bootstrap.rs.diff`。
- `src-tauri/src/agent_runtime/multi_agent/scheduler/assignment.rs`：已有增量，239行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__agent_runtime__multi_agent__scheduler__assignment.rs.diff`。
- `src-tauri/src/commands/agent_tests_executor_allocation_negative.rs`：新建，150行，前像与最终差异`/tmp/oviraptor-executor-allocation-reviewed-merge-diffs/src-tauri__src__commands__agent_tests_executor_allocation_negative.rs.diff`。

## 剩余与风险

Root自身监督费用未预留完整六类触发；Reviewer余额不等于实际审核完成。完全无上限utility仍拒绝，其他角色和十维分配/释放/原费用/美元对账、未知确认显式恢复、跨attempt/崩溃及全部15角色工具推理/实际并行、聊天/日志/整体UI仍未完成。其他删除/保护/取消/历史义务、回归债、非Web和数据知识保持。全框架之后才全量Rust/UI、安装App打开/历史崩溃、授权URL匿名只读和真实模型质量；不能把本审计当整体功能验收。

原历史文档字节备份及增量`/tmp/oviraptor-executor-allocation-docs/`，完整十四类剩余见Master顶部；不自动提交、不删除真实asset。
