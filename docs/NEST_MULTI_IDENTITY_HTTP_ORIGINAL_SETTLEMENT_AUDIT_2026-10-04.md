# Multi身份对照、HTTP在途撤权与原SDK最终结算审计（2026-10-04）

范围：当前Master持续开发中REM-A01直接回归、完整身份权限链及REM-A02原Web费用最终结算的一部分；不缩小原Master14大项或§18最终条件。

## 问题证据及数据作用域

全部运行使用临时SQLite、合成捕获会话与localhost真实SDK/HTTP传输；任务通过新creator、草稿绑定、startup/HMAC及原Root出生入口产生。没有SQL改Single/Multi标签、补造Root原授权或财务退出证明。

- 原HTTP回归旧裸结果不能代表原执行；先证明裸回调拒绝且全部行/费用/tally不变，再迁到实际Single原入口。两个真实停止分别保留未知HTTP效果（target 0预留/0已知/1未决）和收到后撤权（0/1/0）；实际SDK1/HTTP1、第二工具不发、原model账单、Root/终态checkpoint/暂停及重复owned保持。初次1/1，测试2.45秒，日志`/tmp/oviraptor-original-http-terminal-first.log`。
- Multi匿名对照真实红：SDK18次、仅匿名公开HTTP1次、身份比较0，原prepare重建context丢失匿名句柄；日志`/tmp/oviraptor-multi-anonymous-original-red.log`（0/1，4.95秒）。仅追加固定无凭据anonymous control，三个Mapper/IdentitySession身份数量字段使用完整验证后的登录会话数量。真绿日志`/tmp/oviraptor-multi-anonymous-original-green.log`（1/1，12.13秒）；公开匿名1、登录比较1、匿名比较1，登录数量1、原mode不变，角色实际SDK、费用/退出/owned重放通过。
- Multi在途撤权真实红：公开HTTP1＋登录HTTP1，现场SDK计数证明撤权后无SDK、第二侧未发，但终态误报`http_observation_assignment_or_lease_denied`，随后最终费用因checkpoint滞后触发`budget_usage_below_recorded_cost`；日志`/tmp/oviraptor-multi-anonymous-mid-http-red.log`（0/1，2.37秒）。只修观察producer原错误码后仍红，底层已正确`execution_authorization_denied/tool_identity_binding_denied`，低报保护仍拒绝：`/tmp/oviraptor-multi-anonymous-observation-red.log`（0/1，2.32秒）。这是两层独立生产问题，不是通过改期望掩盖失败。
- 最小最终结算改为同一IMMEDIATE事务中读取并核验原child/assignment/Root/epoch/fence/round/requestHash/responseHash及原财务receipt，checked_add完整已报用量，再执行既有双账本结算。未知或未报费用要求对账，原已发生费用不退；原不可变guard、低报/预留/超额/重放拒绝、supervisor准入保持，不写伪checkpoint。删除已不再使用的checkpoint差分helper/字段及两测试构造中的字段，其他内容保持。
- 新实际Multi三项最终3/3，测试11.04秒、编译56.40秒：`/tmp/oviraptor-multi-anonymous-original-three-fixed.log`。原SDK费用与子run汇总精确一致，checkpoint仍滞后；无事务、六原scope字段＋错assignment/child、7回执绑定错配、hash/modelRequests/cache/usageReported四篡改均拒绝且读取零写，rollback后所有原行/rowid/费用恢复、SDK/HTTP无增量。篡改仅隔离测试库模拟导入损坏；真实原库guard未触碰。

## 当前源码门禁

- `expanded-before-fixture-fix`：exit101，阶段546.48秒，逐名通过436；日志`/tmp/oviraptor-original-http-terminal-related-final.log`。
- `targeted-final`：exit0，阶段51.0秒，逐名通过20；日志`/tmp/oviraptor-original-http-terminal-targeted-final.log`。
- `clippy-final`：exit0，阶段19.82秒，逐名通过0；日志`/tmp/oviraptor-original-http-terminal-clippy-final.log`。
- `retirement-final`：exit0，阶段5.08秒，逐名通过46；日志`/tmp/oviraptor-original-http-terminal-retirement-final.log`。
- `original-debt-final`：exit101，阶段32.1秒，逐名通过9；日志`/tmp/oviraptor-original-http-terminal-original-debt-final.log`。
- `source-pause-debt-final`：exit101，阶段2.01秒，逐名通过0；日志`/tmp/oviraptor-original-http-terminal-source-pause-debt-final.log`。

扩大440项初次436通过/4失败（546.48秒）；直接两项是已失效generic权限别名预期及用未报账401撞已知结算的故障夹具。前者改精确原拒绝码并加强全原行零写；后者在实际Web付费后撤权，summary writer失败与Root私有writer拒绝残留撤权trigger的越界SQL同时保留，paid invoice1/未知0、预留/原暂停/Root与directive回滚保持。独立401及其directive-close故障路径保持并当前重验。第一次付费夹具仍期待directive-close触发，实际更早由私有writer拒绝，见`/tmp/oviraptor-original-http-terminal-fixture-boundary-fixed.log`（1/2）；随后改为精确期待该拒绝，未弱化生产guard。定向6/6之后首Clippy拒绝比较时临时String分配；改为相同固定canonical字节，不禁用lint，旧日志及元数据另存before-literal-fix/pre-clippy-fix。

当前定向20/20选择与通过集合一致，含三Multi/旧HTTP终态/直接两修正/两个独立401/预算Web模型原回执负向；具名集合`/tmp/oviraptor-original-http-terminal-targeted-final-names.json`。440未整体重跑，不能宣称全部通过；初次扩大名单`/tmp/oviraptor-original-http-terminal-related-names.json`及元数据`/tmp/oviraptor-original-http-terminal-expanded-initial-checks.json`保持，最终元数据`/tmp/oviraptor-original-http-terminal-final-checks.json`。当前lib2312项编译，并未全量运行；不将前批134/18或本批初次3拼为全量验收。

原14独立当前9通过，以下5仍失败，未ignore、未恢复旧兼容：

- `commands::agent_tests::bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure`
- `commands::agent_tests::e2e_missing_side_stops_at_insufficient_evidence`
- `commands::agent_tests::every_target_ends_in_exactly_one_terminal_state`
- `commands::agent_tests::policy_native_coverage_only_is_ready_without_a_reviewer`
- `commands::agent_tests::policy_native_routes_without_runtime_adapter`

另两Source暂停具名债，当前独立复核仍失败，均`scan_quiescence_original_target_exit_missing`；实际Source SDK/费用/elapsed断言之后，finish_sentinel_pause要求外层原目标退出证明。尚未证明生产入口缺陷或全部夹具缺口，下一项从实际外层生产入口核验；不能删断言或给裸Source调用补造退出证明：

- `commands::tests::source_financial_exit_actual_sdk_stop_preserves_original_fee_and_only_observes_elapsed`
- `commands::tests::source_financial_exit_pause_between_live_check_and_closure_cannot_publish_terminal`


局部fmt：新增Multi leaf及web_lifetime完整检查，已改Single回归/执行函数和共享helper局部规范化；旧文件未批量fmt。git diff--check0。InputParser原自动审批拒绝仍不绕过，完整拒绝原因在当前上下文不可见。

## 未提交改动保护

HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`；源码1323集合SHA `2bd96feb09b6801d5ff7f540549e9217233e4a83a2becbab8afbf28284882a57`。14路径13已有/1新，1309原范围外逐SHA不变；完整原bytes/既有Git diff备份、每补丁SHA/git apply--check及最终增量全文审查保存于`/tmp/oviraptor-original-http-terminal-*`。

- `src-tauri/src/commands/agent_tests_http_journal.rs`
- `src-tauri/src/commands/agent_tests_fresh_single_production.rs`
- `src-tauri/src/commands/agent_tests.rs`
- `src-tauri/src/commands/agent_tests_multi_anonymous_control_original.rs`
- `src-tauri/src/commands/multi_agent/prepare.rs`
- `src-tauri/src/commands/agent_tools_http/evidence.rs`
- `src-tauri/src/agent_runtime/multi_agent/budget/web_lifetime.rs`
- `src-tauri/src/commands/multi_agent/execution.rs`
- `src-tauri/src/commands/multi_agent/gap_proposal.rs`
- `src-tauri/src/commands/multi_agent_runtime.rs`
- `src-tauri/src/commands/agent_tests_client_side_production_boundary.rs`
- `src-tauri/src/commands/agent_tests_client_side_production_fixture.rs`
- `src-tauri/src/commands/agent_tests_directive_closure_fresh.rs`
- `src-tauri/src/commands/multi_agent/tests/broker.rs`

初始新文件补丁格式导致apply--check失败、随后错误过滤0选择、冻结夹具plan冲突、移除字段后两旧构造编译失败均单独保留日志，不算功能红或通过。修复只覆盖具名最小作用域，不重置、不批量覆盖、不提交、不push。

## 仍需完成与验收边界

本批没有真实DB/CAS/资产写入，真实旧数据仍按盘点/备份/授权边界处理；没有App安装/打开或外部URL测试。脚本provider用于确定传输、费用、权限和恢复，不代表真实模型推理/工具选择/纠错质量。

保持Master顶部REM-A01至REM-A14全部剩余：其余删除/残余活路径及回归；预算动态分配/全通道对账/恢复；六监督触发/全15角色/General ReAct/Broker/实际并行；独立证据审核；聊天闭环/逐路实时日志/整体UI；非Web/数据知识；最后全量门禁、安装App打开、授权URL与真实模型质量。Goal active，不标完成。
