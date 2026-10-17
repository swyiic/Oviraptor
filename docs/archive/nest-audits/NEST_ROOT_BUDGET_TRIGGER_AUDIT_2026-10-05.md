## 2026-10-05 原预算分配事件进入实际 Root 监督

Master 未完成，Goal active。完整十四类剩余如下；下方历史原字节保留。

实际 creator → Root/Mapper → Web SDK/Broker → Client → Root 原链路缺预算分配监督，新增负向0/1：Root仅3次，应4次。现仅当前Native Web4在执行器模型/目标访问前，将原worker、不可变原分配凭证、原付费Mapper派发回执及原父实例C组成预算分配事件，实际Root SDK评估后由Rust允许继续同一原额度或暂缓。新增assess:budget_allocation不增加预算、权限、目标或lease；原版本1/2/3和Single保持原规则，没有新增Bootstrap版本/字段或Strix正向兼容。

Root原模型绑定/十维入账/退出/paid publication和聊天时间线使用原生产者。原task与实际派发政策、物理行逐一核验，陈旧caller无法扩额；父实例弱票据不保持父活性、不领替代C。原事实直接传给后续付费回执复核，避免重读竞态。纯授权检查不初始化账本；Root在途模型调用不能被自身新派发误判为旧未决费用，原调度器初始财务门禁顺序保持，入口前及已关闭评估后仍检查全局未决义务。租约续期/本次费用不生成新的原分配事件。

六新增具名回归证明原全链路Root4付费/4 publication且localhost GET1、预算监督先于Web SDK；同原running worker事件重放全库typed rows/rowid零写、零SDK，paid评估不能转成目标派发政策；暂缓/越权实际入口均没有Web SDK或目标HTTP；实际503保留原两次已付Root和第三次未决request/tokens、原grant和worker身份，不退款/换代/重发。八类授权/投影/原任务/父票据/目标损坏先拒绝且零写；付费回调期间撤权保留已付费用、阻止publication，原临时事实恢复后仅本地发布原回执，不再SDK。测试中的还原不是生产撤权恢复许可。

初次局部修复错误读取原回执JSON层级，实际0/1；扩大轮编译失败只修测试方法名，随后1/6暴露Root自身在途被误判未决；修后6/6。首条短名--exact选择0项不算通过。首轮关联90/90但Clippy比较写法失败，不算最终门禁；最终规范化字符串显式保留同一语义并绑定原捕获事实后，重新按完整90项同源验证。所有失败日志保留。

最终同源码关联90/90（测试181.15秒/阶段202.53秒，选择=报告=通过），严格all-features/all-targets Clippy0（40.87秒），同二进制退役50/50（5.44秒，含literal/当前Native JSON），六叶局部fmt与范围diff0。无ignore。当前2381项全量、整体UI/安装IPC/授权URL及真实供应商模型/美元对账仍未验收；局部结果不累计为整体功能通过。

16代码路径11已有/5新保存逐文件前像和原Git差异，最终增量逐文件审查；1370原范围外源码及HEAD59be3d86保持，1386源码集合SHA c75b7b30581b3f0968071fd92d1e26c3dd16a120d51b24b6fc5094dae543bf13。只用临时Git/SQLite/CAS/localhost脚本SDK，不触碰真实DB/CAS/asset、UI/安装/授权URL，不自动提交。

本批只接原分配这一种预算变化，不代表全部预算/保护事件或六类监督完成；Root16,000/1仍仅最低容量，其它角色竞争/更大多轮调用、全十维动态分配/精确对账/未知显式恢复、完全无上限utility保持未完。并发准入不是SDK真并行；脚本SDK不是实际供应商推理质量。全15角色、聊天/逐路日志/整体UI、删除/取消/恢复、非Web/数据知识和回归债仍需完成，框架后才做安装App打开/历史崩溃、全量门禁及两授权URL匿名只读，登录身份待提供。

逐文件增量：

- `src-tauri/src/commands/agent_tests.rs`：原增量，213行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_tests.rs.diff`。
- `src-tauri/src/commands/agent_tests_root_budget_trigger.rs`：新建，103行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_tests_root_budget_trigger.rs.diff`。
- `src-tauri/src/agent_runtime/multi_agent/scheduler/schedule_authority.rs`：原增量，177行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__agent_runtime__multi_agent__scheduler__schedule_authority.rs.diff`。
- `src-tauri/src/agent_runtime/multi_agent/supervision_ticket.rs`：原增量，157行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__agent_runtime__multi_agent__supervision_ticket.rs.diff`。
- `src-tauri/src/commands/agent_native/coordinator_dispatch.rs`：原增量，141行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_native__coordinator_dispatch.rs.diff`。
- `src-tauri/src/commands/agent_native.rs`：原增量，41行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_native.rs.diff`。
- `src-tauri/src/commands/agent_native/coordinator_frame.rs`：原增量，218行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_native__coordinator_frame.rs.diff`。
- `src-tauri/src/commands/agent_native/coordinator_rust_policy.rs`：原增量，125行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_native__coordinator_rust_policy.rs.diff`。
- `src-tauri/src/commands/agent_native/executor.rs`：原增量，429行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_native__executor.rs.diff`。
- `src-tauri/src/commands/agent_tests_fresh_multi_production.rs`：原增量，118行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_tests_fresh_multi_production.rs.diff`。
- `src-tauri/src/commands/agent_tests_changed_fact_tick.rs`：原增量，306行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_tests_changed_fact_tick.rs.diff`。
- `src-tauri/src/commands/agent_native/coordinator_budget_frame.rs`：新建，137行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_native__coordinator_budget_frame.rs.diff`。
- `src-tauri/src/commands/agent_native/coordinator_budget_hook.rs`：新建，126行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_native__coordinator_budget_hook.rs.diff`。
- `src-tauri/src/commands/agent_tests_client_root_feedback.rs`：原增量，180行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_tests_client_root_feedback.rs.diff`。
- `src-tauri/src/commands/agent_tests_root_budget_trigger_fixture.rs`：新建，61行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_tests_root_budget_trigger_fixture.rs.diff`。
- `src-tauri/src/commands/agent_tests_root_budget_trigger_negative.rs`：新建，188行，diff `/tmp/oviraptor-root-budget-trigger-reviewed-merge-diffs/src-tauri__src__commands__agent_tests_root_budget_trigger_negative.rs.diff`。

同源原证据 `/tmp/oviraptor-root-budget-trigger-{code-snapshot,scope-final,final-result,retirement-result,clippy-result}.json`；逐文件前像/原Git差异及失败日志均保留。原首轮90/90但lint失败证据保存在 `/tmp/oviraptor-root-budget-trigger-before-final-review/`；最终计数只取最终源码。文档原字节备份/新增diff：`/tmp/oviraptor-root-budget-trigger-docs-final/`。
