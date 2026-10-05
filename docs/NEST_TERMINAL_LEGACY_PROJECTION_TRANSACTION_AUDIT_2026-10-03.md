# 终态派生投影事务审计 · 2026-10-03

Master开发中；全量门禁存在9项旧正向失败，不宣称整体通过。共享脏树HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`。只操作完整隔离tmp应用数据库/localhost模型与测试目录，未操作真实DB/CAS/资产/安装或授权URL。未重置、未自动提交。

## 实际问题和作用域

旧pipeline先调用canonical publisher，再分别写agent_terminal、目标状态/adaptive_routing、fuse，并忽略错误且先计数。静默RAISE(IGNORE)可留下缺检查点/目标；checkpoint触发器可写业务projects。原publisher提交后并发attempt2开始，旧scope仍能投影到无attempt字段的当前目标/checkpoint。相同Owned outcome重放可重复tally。

只改变派生投影消费者和私有writer，保留原费用先提交及canonical publisher/Saved/Native JSON语义。原函数完整preimage `/tmp/oviraptor-terminal-legacy-consumer-preimage.rs` 后移至168行左右独立叶；旧大文件仅include、tally clone/原Root集合及捕获日志path。原producer/OS invocation一直由Owned outcome持有。

## 实现合同

- 原金融/canonical publication不回滚。先构造完整derived route/terminal/fuse和staged tally；成功提交私有projection事务之后才替换tally。
- 私有SQLite READ_WRITE/no CREATE、FULL同步、FK、250ms busy timeout、IMMEDIATE事务。从同事务核current/nondeleted/精确原native/coordinator父空assignment空且唯一Root；目标scope必须恰一行。
- 私有authorizer仅允许checkpoint/FUSE INSERT及各表固定derived列UPDATE；禁止触发器写入、foreign/业务表、Root/财务/Native/DML删除及DDL。不会替换调用者的连接hook或外部事务。
- 写agent_terminal、目标/路由、adaptive_routing、可选fuse全部readback；IGNORE、DROP/业务side effect等错误统一rollback，不能部分提交。内容完全相同的Saved projection不写、不改物理行/时间；同一tally原Root只计一次。
- actual consumer内error返回Result；record_owned保存原log_path，将安全错误信息追加原runner日志并停止流水线。正常paused返回false，paused projection不生成agent_terminal。cfg(test)旧接口只适配bool，不产生生产current身份回填。

## 实际失败、修复与边界

1. cfg(test) barrier只暂停真正production consumer在canonical commit之后，不替代writer或SDK。真实ordinary creator/startup/HMAC→run_agent_target路径，四项first-red全部失败0/4（1.97秒）：agent_terminal IGNORE、目标UPDATE IGNORE、checkpoint项目副作用、另一真实线程在commit后改attempt2。前三项第一成功返回值断言失败，race返回true/tally1；不据未到达的后续snapshot断言扩大红轮证明。
2. 一处私有IMMEDIATE writer及staged tally后，相关38/38（27.57秒），四负向的所有临时业务表含rowid保持；原金融/canonical已提前发布，失败后同样保持。此轮unused doc comment只因include上文档注释，已改普通注释，不算功能红。
3. 追加7合同首5/7（3.28秒）。真实owned重复消费时全部物理行已经保持，但tally实际2，计数缺口是真红。adaptive fault最初route与producer已存内容相同，Saved正确免写、未走trigger；调整请求的derived route为变化内容后才证明其负向，不放宽writer。额外两个lower writer用例验证FUSE原插入/更新IGNORE和业务side effect完整回滚，明确不是WAF执行/浏览器证明。
4. 原Root scoped tally去重+夹具前提修正后相关41/41（30.34秒），原原生SDK和Single finally都包含其中。
5. 扩大所有已知旧直接consumer调用者的运行206/215（217.92秒），9失败保存在 `/tmp/oviraptor-terminal-legacy-atomic-expanded-legacy-callers-red.log`/json。分别为bootstrap_recovery_refusals、anonymous/missing-side/personalized/equal/no-finding/challenge E2E、every_target状态矩阵及outer invocation。只读检查其遗留tmp库：anon/equal已SDK执行，但agent_terminal明确budget_history_requires_reconciliation、原Root budget attempts=0；outer invocation没有Root/财务且endpoint等待超时。无Rootsynthetic复原用例被身份guard提前拒绝。未授旧数据原finance、未把当前Typed Mode补到旧活路径。先前commentary把这些统称为SDK之前，已依据实际库更正。它们是Master尚未完成的旧正向迁移/功能验收欠项，不能用当前集合通过代替。
6. 转发typed projection error的第一次patch漏删if，rustfmt返回expected '{'，set-e阻止Cargo启动；一行修正后再format/gate。该静态错误不算功能red。

## 最后当前合同门禁（不含9项未迁移旧正向）

- `clippy-final` exit0，wall 13.57秒，`/tmp/oviraptor-terminal-legacy-atomic-clippy-final.log`。
- `affected-final` exit0，wall 211.68秒，`/tmp/oviraptor-terminal-legacy-atomic-affected-final.log`。
- `importer-final` exit0，wall 53.25秒，`/tmp/oviraptor-terminal-legacy-atomic-importer-final.log`。
- `literal-final` exit0，wall 1.28秒，`/tmp/oviraptor-terminal-legacy-atomic-literal-final.log`。

当前集合206/206，包含七个本批合同及所有前批原身份/SDK/预算/Source合同，不把41与206相加。严格all-target/all-feature Clippy -Dwarnings、import-existing-results39、exact退役1；全部serial nice15/offline/locked/j1/testthreads1，Cargo期间没有编辑Rust。前端无代码变化，不重复计前批815/815/TS/Vite。新四Rust叶与原identity叶fmt/check及gitdiff-check0。

## 原脏树保护

8路径4已有/4新，新叶max176；collection1119路径SHA `cbf91b2ce6dc0050108a735ef3b8641d879d6358958c3473a8a87f922e9d8f69`，1111范围外原路径保持。baseline/before/prior-diffs/完整incremental diff/scope/snapshot均 `/tmp/oviraptor-terminal-legacy-atomic-*`；旧文件逐hunk合入并保存完整移出函数原文。HEAD不变。

| 路径 | 类型 | 行数 |
|---|---|---|
| `src-tauri/src/commands/agent_backend.rs` | 已有 | 1031 |
| `src-tauri/src/commands/agent_original_terminal_identity.rs` | 已有 | 232 |
| `src-tauri/src/commands/agent_tests.rs` | 已有 | 113 |
| `src-tauri/src/commands/agent_tests_terminal_legacy_atomic.rs` | 新增 | 176 |
| `src-tauri/src/commands/agent_terminal_legacy_consumer.rs` | 新增 | 166 |
| `src-tauri/src/commands/agent_terminal_legacy_writer.rs` | 新增 | 134 |
| `src-tauri/src/commands/agent_terminal_legacy_fuse.rs` | 新增 | 33 |
| `src-tauri/src/commands/native_invocation.rs` | 已有 | 11 |

## 未完成与风险

canonical publication与derived projection仍为先后两个事务。崩溃于两者之间时original invoice/publication保留，但安装重启恢复完整derived closure尚需实际证明；这一批没有新恢复grant或将terminal Root自动重跑。源Native/原财务最终consistent snapshot/ABA/quiescence及停止诊断的真实Tauri回放需下一切口，不把仅current身份guard当完整原金融证明。非fuse Limited旧分支状态/tally语义和全部outcome还需要统一核验。

9项旧正向场景尚需迁真正新建Mode/HMAC/original accounting，不能在生产回填旧历史或降低防回退准入。Web helper日志、Source all-outcome finally、Root完整ReAct/所有触发/全角色/Broker/真实并发、人工有序SDK/ACK/聊天、动态预算/对账及最后全量/安装态/授权URL未完成。InputParser原自动审批拒绝不重试绕过，原摘要未保存详细拒绝原因；Goal active，Master不标完成。
