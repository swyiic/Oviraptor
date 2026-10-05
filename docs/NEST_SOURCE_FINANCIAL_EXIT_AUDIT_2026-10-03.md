# Source 原财务退出审计 · 2026-10-03

Master开发中。本批覆盖原财务身份捕获之后的Source错误出口、完整elapsed原账及真实停止竞态，不是全部框架或安装验收。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 保持；未重置/批量覆盖/提交、未触真实数据库/CAS/资产/安装或两个授权URL。

## 证据、数据作用域与最小修复

旧Source top-level成功终态有财务收口，provider/工具错误、自然期限与旧attempt返回却可能跳过原wall账。新增首五项实际0/5（33.20秒），生产wrapper分出owned flow后5/5（34.02秒）；第一red只达到首断言，后续故障分支实际green单列，不扩大red证明。

只有真正creator生成Root及原Original财务控制、对应C绑定之后才进入owned wrapper。missing finance或入场时已被替换的C拒绝且不写；不将历史timer/空费用Root补成原金融身份。原运行捕获后失效C仍可以记录已经发生的费用，不能继续工具/结果入库/ACK或借此授予恢复。

`record_original_exit` 使用独立RW/no CREATE连接，busy250ms、FULL/FK；调用方已有未提交事务拒绝，原caller authorizer保持。一个IMMEDIATE事务和一个SQL cutoff完成原proof、异常elapsed fact、wall sample、readback、commit，只允许直接写agent_root_elapsed_facts和agent_budget_entries；金融/业务触发器副作用拒绝并回滚。普通wall成对reserve/consume；超hard或失效C保存完整elapsed与未结清差额，不截断、不扩限、不退款，不写Root业务terminal。

自然30秒cutoff红测0/1（29.57秒）复现两事务之间跨hard导致费用丢失；同一cutoff修复后相关6/6（65.02秒），原普通lease SQL保持。测试里的等待确实让hard自然到期，没有改写limit/origin或拿可变started_at伪造生命周期。

真实SDK阻塞后生产pause、attempt旋转、C替换三条路径检查原owner exit与late输出；第一扩大8/9暴露Root错误terminal。晚endpoint记录在handler返回后写入，第二4/5中的计数前提已修为释放后真实drain并复核全表，未放宽生产边界。private作者器/金融IGNORE/业务副作用/外层事务三项3/3。

外侧检查后再暂停的确定性cutover红测0/1（0.76秒）复现TOCTOU；Source专用失败收口放入shared closure_write原IMMEDIATE事务，重核Original C、活动attempt、原source授权与clock，再写原Source失败状态及事件。相关3/3（4.32秒）包含真实1SDK500、检查后生产pause和live500边界；未改全局共享finish语义。

## 原回归失败的迁移与保留

首扩大259/293（1676.46秒），34失败日志保留。全行snapshot仍捕获所有表rowid,*及sqlite_sequence，只允许原Root新增wall账pair和异常elapsed单fact；所有已有物理行、别Root/业务/维度保持，Saved及入场拒绝仍完整零写。

真正live Source specialist/guidance/cancel/provider/review材料夹具改为before-INSERT typed Original eight-request合同，Native v1 JSON原文不换，不使用历史金融回填。closure11个金融/业务触发器分支使用实际inventory→finish两角色，共7SDK/140tokens，确实触发完整coverage写入；纯finish夹具不算该覆盖。

复制worker只有run不具Original attempt/费用；namespace测试保留原callback费用/跨scope/同callId保护，然后工具准入必须source_tool_original_finance_unavailable，closure不得执行且全应用行不变，不把copied executing round当新授权。

原失败34+金融10集合43/44（656.48秒），最后一项历史旧finance缺失先于timer拒绝，实际码source_root_original_finance_missing。先加错误值保持旧断言复现，再只校正精确code并保留started_at2000/0assignment/全表零写，exact单项1/1（0.49秒）。一次短名加exact误过滤0项的命令明确不算证明。现在384集合包含这项，全部通过；未删/ignore失败、未靠补兼容收绿。

## 最终门禁与快照

- clippy-final：exit0、wall14.28秒，`/tmp/oviraptor-source-financial-exit-clippy-final.log`。
- affected-final：exit0、wall2157.86秒，`/tmp/oviraptor-source-financial-exit-affected-final.log`。
- importer-final：exit0、wall37.58秒，`/tmp/oviraptor-source-financial-exit-importer-final.log`。
- literal-final：exit0、wall1.26秒，`/tmp/oviraptor-source-financial-exit-literal-final.log`。

受影响384/384 runtime2157.47秒、导入39/39、exact登记1/1，不是全Rust测试门禁。集合重叠不相加。27路径19已有/8新，8新Rust叶max187，fmt与diffcheck0；collection1136 SHA `aa52b6e194a6177cb49359fba1002808cda4b732cd6935dbbad7d05e0e97ba95`，1109原范围外保持。

`/tmp/oviraptor-source-financial-exit-baseline/before/prior-diffs/reviewed-merge-diffs/scope-final/code-snapshot` 保留逐文件原文/增量。Source先封存，门禁运行时独立改5个UI路径；当前核原Rust/resource全部hash保持、范围外变化精确只这5UI且等其已封存hash，没有在最终源码变化后重用门禁。原owned body preimage在`/tmp/oviraptor-source-financial-exit-owned-preimage.rs`。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/tests_source_financial_exit_support.rs` | 155 | 是 |
| `src-tauri/src/commands/tests_source_financial_exit.rs` | 187 | 是 |
| `src-tauri/src/commands/tests.rs` | 117 | 否 |
| `src-tauri/src/commands/native_source_execution_owned.rs` | 165 | 是 |
| `src-tauri/src/commands/native_source_assessment_authority.rs` | 232 | 否 |
| `src-tauri/src/commands/native_source_fresh_finance.rs` | 166 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/clock.rs` | 220 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/clock/finalization.rs` | 384 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/clock/observation.rs` | 81 | 是 |
| `src-tauri/src/commands/tests_source_financial_exit_cutoff.rs` | 72 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/budget/clock/elapsed_fact.rs` | 236 | 否 |
| `src-tauri/src/commands/tests_source_born_deadline_fixture.rs` | 83 | 否 |
| `src-tauri/src/commands/tests_source_original_timer.rs` | 215 | 否 |
| `src-tauri/src/commands/tests_source_finish_reentry.rs` | 56 | 否 |
| `src-tauri/src/commands/tests_source_coverage_reentry.rs` | 99 | 否 |
| `src-tauri/src/commands/tests_source_tool_deadline.rs` | 109 | 否 |
| `src-tauri/src/commands/tests_source_deadline_reentry.rs` | 212 | 否 |
| `src-tauri/src/commands/tests_source_guidance.rs` | 829 | 否 |
| `src-tauri/src/commands/tests_source_reentry.rs` | 376 | 否 |
| `src-tauri/src/commands/tests_source_financial_exit_owned_stop.rs` | 140 | 是 |
| `src-tauri/src/commands/tests_source_specialists.rs` | 386 | 否 |
| `src-tauri/src/commands/tests_source_specialist_cancellation.rs` | 298 | 否 |
| `src-tauri/src/commands/tests_source_dispatch.rs` | 402 | 否 |
| `src-tauri/src/commands/tests_source_review_material.rs` | 162 | 否 |
| `src-tauri/src/commands/tests_source_financial_exit_private.rs` | 58 | 是 |
| `src-tauri/src/commands/tests_source_financial_exit_cutover.rs` | 74 | 是 |
| `src-tauri/src/commands/tests_source_worker_namespaces.rs` | 112 | 否 |

既有829行guidance只改原财务夹具/全行作用域，未继续堆入新业务；402行dispatch原否定合同只校正实际拒码并保存错误值。本批新合同全拆新叶。旧结构债后续随其业务继续拆，不做无关大搬迁。

## 未完成与风险

金融wrapper不覆盖原Root/finance捕获前的不可信输入；这类输入仍零写拒绝，不能宣称所有应用出口都已结清。暂停保留金融Running Root记录，UI已说明原状态；费用保存不提供Resume或新C权限。Source重派/完整父存活及跨进程恢复、所有角色/Broker/完整Root循环、动态预算/人工对账、有序聊天/整体UI和旧9项E2E迁移仍待完成。SDK localhost合同与合成UI不代表配置中真实LLM效果、安装态或授权URL验收。InputParser原自动审批拒绝不绕过；具体拒绝原因未保存于当前摘要。Goal active、Master不标完成。
