# 原执行身份与终态上报审计 · 2026-10-03

Master仍开发中。共享脏树HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`；8条代码路径逐文件增量检查、不重置、不批量覆盖、不自动提交。数据仅完整临时应用SQLite和localhost模型/目标端点，不碰真实业务数据库/CAS/资产/安装态/授权URL。

## 问题和来源

生产run_agent_target返回后丢失原Root绑定；runtime_report/terminal publisher从sentinel_scans当前attempt_count选Root。如果原模型配置在Native JSON之前失败，旧结果可选择后来的Root2，把旧configuration failure发布到新Root。缺Root/外来Root也可被当前状态补选。原Native JSON必须保留，不以它作为无状态失败的身份来源。

## 修改

OriginalAgentTerminalIdentity是私有非serde值，先捕获真实admission的path/scan/attempt/target，在runtime_open_run_for_attempt精确返回时绑定Root。OwnedAgentTargetOutcome在持有原OS invocation owner期间传给record_owned_agent_target_outcome。production runtime report使用显式原attempt，禁止读取当前身份补选Root；历史current lookup wrapper仅cfg(test)。纯runtime_report原函数逻辑移入新叶，完整preimage `/tmp/oviraptor-original-terminal-runtime-report-preimage.rs`。

scope检查原path/scan/target/attempt、未替换/未删除任务、精确original Root native/coordinator/parent NULL/assignment空及scope唯一Root。reporter再次核唯一Root与captured ID；不存在Root的早退保持无Root，不制造owner/finance/历史。原Single finance-first close、immutable receipt、Native JSON/source proof、paused与Saved语义保留。生产scan_execution消费Owned outcome，未把clone outcome与当前轮次重新配对。

## 实际红绿与首断言

1. interface prerequisite仅携带身份并委托旧checked publisher，没有行为修复；四项首次1/4（1.17秒）。晚配置失败无Native、缺captured Root、外来Root三项原publisher返回Ok，确实错误选当前Root。当前原Root+Saved replay一项通过。红轮在第一is_err断言停止，不据后续未到达的snapshot断言扩大证明。
2. production identity implementation后，原四项+Single finally26项合计30/30（22.24秒）。
3. 新测试真正ordinary creator/startup/HMAC→run_agent_target→record_owned_agent_target_outcome。四项首次3/4（4.15秒）：真实SDK费用归原Root、配置失败原Root、轮次替换全物理行零变化通过；duplicate Root损坏时publisher Err被旧pipeline转Persistence后继续写入/计数，第一返回值断言失败。没有将lower publisher通过冒充生产consumer闭环。
4. 最小补scope唯一Root前置检查后，四项+原四项+Single finally26项合计34/34（26.15秒）。late callback全原/current临时表包含rowid保持、tally零；duplicate Root损坏scope同样零写/零计数。真实SDK正向至少一physical调用，所有received账单归原Root，Single projection receipt1、无Coordinator lease。当前model setup测试SDK0，无Native状态，按原configuration terminal发布。

晚轮次第一个测试创建Root2并显式test-only原financial fixture，在任何当前SDK/Native之前；仅证明旧publisher问题，不证明真实Retry生产流程。生产consumer的轮次替换也仅隔离测试行修改，不计完整重新执行验收。SDK答复为localhost严格协议fixture，不计模型质量。

## 最后门禁

后台串行nice15/offline/locked/allfeatures/-j1/testthreads1，Cargo期间未编辑Rust：
- `clippy-final` exit0，wall 31.0秒，`/tmp/oviraptor-original-terminal-identity-clippy-final.log`。
- `affected-final` exit0，wall 178.69秒，`/tmp/oviraptor-original-terminal-identity-affected-final.log`。
- `importer-final` exit0，wall 41.95秒，`/tmp/oviraptor-original-terminal-identity-importer-final.log`。
- `literal-final` exit0，wall 1.31秒，`/tmp/oviraptor-original-terminal-identity-literal-final.log`。

相关集合198项包含所有8个新identity用例，不把34与最后集合相加；导入39、exact退役1，严格all-target/all-feature Clippy -Dwarnings。前端没有代码变化，前一SDK批815/815与TS/Vite证据不重复算本批测试。三新Rust叶fmt/check、gitdiff-check0。

## 原脏树与作用域

8路径5已有/3新，三个新Rust叶189/142/245行；collection1115文件SHA `4589877884907a0a41fc3fcb09cdc4bf4629f775254186588a2147923a78715d`，1107范围外原路径保持。baseline/before/prior-diffs/全部incremental reviewed-merge-diffs/scope-final/code-snapshot保存在 `/tmp/oviraptor-original-terminal-identity-*`。HEAD不变。

| 路径 | 类型 | 行数 |
|---|---|---|
| `src-tauri/src/commands/agent_original_terminal_identity.rs` | 新增 | 189 |
| `src-tauri/src/commands/agent_tests_original_terminal_identity.rs` | 新增 | 142 |
| `src-tauri/src/commands/agent_backend.rs` | 已有 | 1168 |
| `src-tauri/src/commands/agent_tests.rs` | 已有 | 112 |
| `src-tauri/src/commands/agent_terminal_report.rs` | 已有 | 183 |
| `src-tauri/src/commands/native_invocation.rs` | 已有 | 10 |
| `src-tauri/src/commands/scan_execution.rs` | 已有 | 231 |
| `src-tauri/src/commands/agent_tests_original_terminal_consumer.rs` | 新增 | 245 |

## 未完成和风险

identity guard与canonical publisher保留既有财务/发布事务边界；目标状态、adaptive_routing、agent_terminal及fuse仍分开写并忽略错误，发布后attempt旋转可能污染当前legacy投影。原源数据consistent WAL/ABA/quiescence、完整projection原子恢复和显式暂停续跑另需真实负向证明。这批不宣布这些已完成。

Web helper逐路日志、Source全outcome finally、Root完整本地ReAct与全部触发、剩余角色/BrowserBroker/真实并发、人类有序SDK/ACK/聊天、动态预算/人工对账、全量门禁/安装态/授权URL仍未完成。InputParser原自动审批拒绝保持未交付、不重试绕过；原摘要未保存其详细拒绝原因。Goal active，Master不标完成。
