# Native SDK 原调用阶段日志审计 · 2026-10-03

Master 开发中；不是全量、真实安装或智能体质量验收。共享脏树 HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`，未提交、未重置、未访问真实业务库/CAS/资产/安装与授权 URL。数据仅完整临时 SQLite、localhost SDK 服务与显式模拟界面。

## 问题与最小作用域

原生产 one-shot transport、Single Root、独立 expert/Source 初始、Source 工具物理轮次及协调 Root 调用没有同原金融身份绑定的持久实时阶段。原费用不能靠日志重建；旧 Root/错误 worker/未归属 call 不得因 logger 得到授权。现有阶段只写安全常量和原身份摘要，独立于 Native JSON、model receipt、role completion 与任务终态。

新增 private diagnostics 模块、只读 IPC 与独立前端游标；旧模型/财务/角色/Source 入口只加观察 hook，原取消、原deadline、费用提交、结果语义、发布事务及 Saved 分支保留。新 SDK owner 永远来自已提交原 dispatch，不制造 Root/C/assignment/lease。

## 原身份、提交与缺口

- Root Single、Root Multi 各核原控制 UUID、plan/十限额/clock、原 dispatch hash；Multi 另核原 tick request、basis、原 C/control。日志读取不要求旧原 C 当前可执行，因此晚取消仍可记原费用。
- 逻辑 assignment 的原格式是 `asg-` 加24个十六进制字符；物理 worker/leaseAttempt 仍为 UUID。不得把逻辑ID强制为UUID或放宽两个域。
- owner 与 prepared 同一个 protected IMMEDIATE；私有 RW/no CREATE/FULL 同步、仅允许 diagnostics INSERT、逐行数量与完整 readback。所有 UNIQUE/rowid/UPDATE/DELETE/recursive OFF REPLACE 保留原记录。额外业务/foreign trigger 不可借 logger 写入。
- sent 记录于真实 send future 首次轮询前，不能证明服务端接受；response_received 于原 HTTP body 完整读取之后、解析之前，不把 body 送给 observer。原金融事务提交后才 cost_saved。
- validated/terminal 仅表达 SDK 入口语义；Source 此时尚未完成工具执行，协调 Root 终结在原 publication 提交后。paused/unknown/unsent/withheld 与费用分类仍以独立业务回执为准。
- 日志失败不增加调用、重试或权限。已有前缀尽可能保存不可变 gap；gap 关闭后不能再补阶段。数据库完全不可写或 begin 失败时不能承诺 gap 可保存，消费者必须保持未知/不完整状态。
- read-only/no CREATE/query_only 单 anchored WAL；校验完整原 owner/财务/阶段/metadata 后才分页。schemaVersion2 各集合最多300，明确 truncation/incompleteOwners/gaps；损坏/foreign/未来游标/缺失attempt/已删除scope拒绝，无修复/回填。

## 实际 red / green 与前提错误

以下集合重叠，不能相加。stdout 原文件保留于 `/tmp/oviraptor-native-sdk-*.log`。

1. `logs-first-production-red` 实际0/6，12.07秒：4条真实路径缺日志；2条Source在SDK之前timeout，不算日志red。`source-prerequisite-diagnostic` 0/2实际原因 budget_history_requires_reconciliation，旧v1 Source夹具没有原finance。迁为既有 true-born Source 新创建后 `source-original-born-red` 0/2（1.86秒），真实SDK到达而已提交日志为空。
2. base 首3/6（2.84秒）；child路径原asg逻辑ID被误当UUID，修正身份域后6/6（3.01秒）。生产 worker/leaseAttempt UUID、原 binding/费用准入不放宽。
3. continuation首编译失败：include不在测试模块、unwrap要求Debug。修测试位置及错误处理后 `v2-contracts-actual-red` 14/21（11.28秒），6个真实合同缺口；另1个expert缺持久attempt为夹具前提，公共reader没有放宽。补原attempt前提与原子/阶段/费用/缺口实现后21/21（6.88秒）。
4. closed gap实际24/25（7.13秒），修不准gap后追加；额外原Root plan漂移实际25/26（7.41秒），pure load_original检查后26/26（7.42秒）。漂移仅在临时夹具，读前后全物理行保持，无修复。移除4行unused helper完整preimage另存。
5. 协调Root测试首 E0425 snapshot helper位置错误，不算行为red。修调用作用域后实际0/6（7.23秒），Root真SDK日志缺失；changed-fact用例当时实际 Mapper 开关没有从主线程继承，因此不宣称其独立 Mapper 已真实调用。薄hook后31/32（22.37秒），将显式 test-only RealSpecialistTransport guard 放在真正worker线程后该用例1/1（1.08秒）：bootstrap+Mapper+变化Root三个真实调用，Saved零SDK/零新日志。
6. 前端 API先实际0/1；组件基本14/14；asg实际13/14红后15/15（1021.500458ms），注册及卡片样式后15/15（1044.979959ms）。实际SFC/composable/shared lifecycle以模拟IPC检查边界；不是Tauri业务通知或安装态证明。
7. 第一轮严格Clippy发现canonical JSON owned comparison及冗余借用。保留canonical序列化检查修lint；修借用初次扩大到gap的Transaction造成3个E0308，已纠正，只保留Connection层去冗余借用；该失败原日志 `clippy-fix-scope-error` 留存，非功能red。最终严格门禁单列。
8. 扩大 `affected-original-fixture-red` 177/188（160.99秒）。10个专职角色正向夹具缺可信Mode，在logger/SDK路径前被拒；Single旧测试缺finance，补显式lower accounting fixture后真实调用进入，但Single模式应不消费Coordinator收件箱。旧不适用正向期待改为更强负向：两种实际SDK结果均不送/不确认该消息，原 directive 物理身份与状态/payload/时间字段保持，单独1/1（1.11秒）。这不证明人工SDK闭环完成。
9. 角色正向夹具迁真实creator/HMAC/新Root/C/十限额/clock；原Node/evidence工作目录保留。第一次新helper参数影响四个外部旧调用者，编译失败，已把新原模型 helper 单独命名；原历史 helper 全函数preimage保留后移入test-only叶，四个旧Caller尚待后续迁移，不宣称其全量通过。第二轮1/11因工作目录误查sentinel_scans，修为原attempt表；第三轮2/11，policy重复INSERT与mock Mapper没有已付回执。保留policy其余字段只设置测试授权session handles；前置Mapper/Reviewer改真正SDK+原费用+原决策封口，不置换财务logger。第四轮11/11（18.95秒），最终另加四子角色+四Root合计8SDK与原Root consumed4；子ledger80/4保持。模型reply都是localhost协议夹具，权重效果/完整代理脑子未验收。

## 最后门禁

Backend使用 nice15、offline、locked、all features、-j1、test-threads1，串行执行且测试期间未编辑Rust。

- `clippy-final` exit0，wall 20.65秒，日志 `/tmp/oviraptor-native-sdk-clippy-final.log`。
- `affected-final` exit0，wall 198.62秒，日志 `/tmp/oviraptor-native-sdk-affected-final.log`。
- `importer-final` exit0，wall 55.57秒，日志 `/tmp/oviraptor-native-sdk-importer-final.log`。
- `literal-final` exit0，wall 1.23秒，日志 `/tmp/oviraptor-native-sdk-literal-final.log`。

相关库 `test result: ok. 188 passed; 0 failed; 0 ignored; 0 measured; 1822 filtered out; finished in 178.24s`。SDK新32项包含于188内；导入器39/39、exact retirement literal1/1。严格Clippy all-target/all-feature `-D warnings`。默认前端815/815（39674.328708ms），vue-tsc与Vite通过；这两门禁之后仅Rust夹具变化，前端原文仍相同。21新Rust叶 skip_children=true fmt/check、逐文件增量diff、git diff --check 与范围外 SHA 核验；旧大文件未全局格式化。

## 界面边界与留证

实际组件/composable读取显式模拟四个owner/13条已提交阶段。1200×900深色宽屏、600×900浅色窄屏，以及打开原归属详情，document/log横向scrollWidth均等于clientWidth。原请求、未知费用与gap可区别；不存在预造阶段或智能体思考。截图 `/tmp/oviraptor-sdk-call-visual/dark-wide.jpg`、`light-narrow.jpg`、`light-narrow-expanded.jpg` 已根代理实际查看。临时localhost preview及自建IAB tab已停止/关闭，viewport恢复。未启动真实App或访问两个授权URL。

## 范围与原脏树保护

46代码路径，18已有、28新；21新Rust叶max312。collection1112路径SHA `916c2746a4e746ac784330edf12b24d7b754395835858b1ec5b4038437f24a67`；1066范围外原路径哈希保持、HEAD不变。capture/原dirty diff/完整增量diff/scope SHA/code snapshot保存在 `/tmp/oviraptor-native-sdk-logs-*`；精确已有路径原文未批量覆盖，所有应用patch先SHA与gitapply-check。SDK unused helper、历史Gap test helper完整preimage保存；Native JSON/current原行来源不被诊断摘要代替。

| 路径 | 原已有/新增 | 最终行数 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 原已有 | 110 |
| `src-tauri/src/commands/agent_tests_native_sdk_expert_logs.rs` | 新增 | 187 |
| `src-tauri/src/commands/agent_tests_native_sdk_logs.rs` | 新增 | 297 |
| `src-tauri/src/commands/tests.rs` | 原已有 | 111 |
| `src-tauri/src/commands/tests_native_sdk_source_logs.rs` | 新增 | 244 |
| `src-tauri/src/agent_runtime/model/diagnostics/mod.rs` | 新增 | 157 |
| `src-tauri/src/agent_runtime/model/diagnostics/owner.rs` | 新增 | 305 |
| `src-tauri/src/agent_runtime/model/diagnostics/persist.rs` | 新增 | 234 |
| `src-tauri/src/agent_runtime/model/diagnostics/replay.rs` | 新增 | 195 |
| `src-tauri/src/agent_runtime/model/diagnostics/schema.sql` | 新增 | 57 |
| `src-tauri/src/agent_runtime/model/mod.rs` | 原已有 | 23 |
| `src-tauri/src/agent_runtime/model/openai_compatible.rs` | 原已有 | 359 |
| `src-tauri/src/agent_runtime/model/openai_lifecycle.rs` | 新增 | 31 |
| `src-tauri/src/agent_runtime/model/transport.rs` | 原已有 | 263 |
| `src-tauri/src/commands.rs` | 原已有 | 171 |
| `src-tauri/src/commands/agent_native/root_model.rs` | 原已有 | 145 |
| `src-tauri/src/commands/multi_agent/child_transport.rs` | 原已有 | 314 |
| `src-tauri/src/commands/native_sdk_log.rs` | 新增 | 36 |
| `src-tauri/src/commands/native_source_tool_execution.rs` | 原已有 | 190 |
| `src-tauri/src/db.rs` | 原已有 | 391 |
| `src-tauri/src/lib.rs` | 原已有 | 563 |
| `src-tauri/src/agent_runtime/model/diagnostics/contract_tests/mod.rs` | 新增 | 65 |
| `src-tauri/src/agent_runtime/model/diagnostics/contract_tests/reads.rs` | 新增 | 130 |
| `src-tauri/src/agent_runtime/model/diagnostics/contract_tests/writes.rs` | 新增 | 211 |
| `src-tauri/src/commands/agent_tests_native_sdk_contracts.rs` | 新增 | 223 |
| `tools/native_sdk_log_harness.cjs` | 新增 | 41 |
| `tools/test_native_sdk_log_api.cjs` | 新增 | 9 |
| `tools/test_native_sdk_log_replay.cjs` | 新增 | 78 |
| `src/features/sentinel/api.ts` | 原已有 | 337 |
| `src/features/sentinel/components/NativeSdkLogView.vue` | 新增 | 71 |
| `src/features/sentinel/components/SentinelTaskCenter.vue` | 原已有 | 377 |
| `src/features/sentinel/execution/nativeSdkLogContract.ts` | 新增 | 97 |
| `src/features/sentinel/execution/useNativeSdkLog.ts` | 新增 | 29 |
| `src-tauri/src/agent_runtime/model/diagnostics/owner_saved.rs` | 新增 | 89 |
| `src-tauri/src/agent_runtime/model/diagnostics/replay_metadata.rs` | 新增 | 138 |
| `src-tauri/src/agent_runtime/model/diagnostics/transitions.rs` | 新增 | 82 |
| `package.json` | 原已有 | 40 |
| `src-tauri/src/commands/agent_tests_root_tick_sdk_logs.rs` | 新增 | 312 |
| `src-tauri/src/commands/agent_tests_root_tick_sdk_cancel.rs` | 新增 | 65 |
| `src-tauri/src/commands/agent_tests_root_tick_sdk_changed.rs` | 新增 | 113 |
| `src-tauri/src/agent_runtime/model/diagnostics/owner_coordinator.rs` | 新增 | 99 |
| `src-tauri/src/commands/agent_native/coordinator_tick.rs` | 原已有 | 135 |
| `src-tauri/src/commands/agent_tests_directive_delivery.rs` | 原已有 | 307 |
| `src-tauri/src/commands/agent_tests_specialist_original_fixture.rs` | 新增 | 208 |
| `src-tauri/src/commands/agent_tests_specialist_transport.rs` | 原已有 | 516 |
| `src-tauri/src/commands/agent_tests_specialist_journal.rs` | 原已有 | 554 |

## 未完成与风险

这批只完成当前四类真正SDK及原调用诊断合同。完整ReAct/所有tick触发、所有角色和Broker/真实2–3路batch、Human模型评估与有序逐项执行、Single原执行身份进入reporter/legacy projection/quiescence/显式续跑、Source全outcome finally、动态分配/人工对账、Web helper日志、真实通知/重启/完整门禁/安装态与授权URL仍需完成。历史Gap正向夹具尚待可信新入口迁移；零目标grant拒绝不算角色或浏览器功能已完成。

InputParser先前自动审批拒绝保持未交付，不换词重试或绕过；现有摘要没有保留拒绝的详细原理由。Goal active，Master不标完成，按框架优先级继续。
