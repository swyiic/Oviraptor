# 有序未发出 worker 终态清理审计 · 2026-10-03

Master未完成；仅临时SQLite、localhost实际SDK及实际SFC/composable组件夹具。未操作真实DB/CAS/资产、安装或授权URL，未提交。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`保持。

## 原问题与作用域

新v3有序提案不在旧agent_directive_proposals单项表里；原finish_coordinator_run可提交Root终态和reconciliation_required，但prepared action的原child/worker、4000/1 reservation、lane/capability仍存在。原终态实际首0/2（1.01秒），两项均在资源assertion得到leased/prepared/leased/4000/1而期待cancelled/terminal/cancelled/0/0；费用全维、回放和投影等后续断言未达。第二项场景第一项已真实SDK调用并保存paid receipt。

新89行closure叶在原终态事务内仅处理原v3 assigned。校验原Source确认/hash、完整projection、Root原财务/Native/C、原scope、全部已保存动作；已有completed/failed必须原verified receipt，有executing/received则不走未发出释放。原prepared必须原单个第一UUID worker/leased、child prepared、started/finished/settled为空、used全零、原4000/1预留；specialist/source/Web durable intent、SDK owner、cost fact、model event、ordered receipt/result message全部不存在，不能拿缺response当未发出。

先证明全部候选，再原scheduler cancel_unstarted_child_in_transaction，仍原append释放和原slot/capability/lane/worker终态；不新建schema、身份、SDK或回执，不改ordered checkpoint原prepared。读回核cancelled child/worker、revoked/no lane、原四维reserve+release physical键/source/leaseAttempt与零reserved/consumed/indeterminate。与Root终态/clock同原受限事务，六故障任何一步失败均完整回滚；未放宽Rootclosure authorizer。

原taskClosure只标not_applied/不要求对账，不说人工建议已执行。读原闭合Source/canonical Root后，额外核真实取消与原释放，才派生cancelled_before_dispatch；旧关闭reconciliation_required历史不回填或升级。前项实际paid receipt保持，第二未派发不虚构receipt或模型用量。历史换C只读仍原绑定；此批不处理过期/替换C的写入清理。

## 验证、负向与已知限制

首合入E0308 match arm缺discard返回，未有runtime；最小加block/semicolon修复后2/2（1.17秒）。新增六故障：assignment取消IGNORE、取消trigger额外UPDATE projects、budget release INSERT IGNORE、lane DELETE IGNORE、capability revoke IGNORE、worker取消IGNORE；用当前全应用表snapshot逐行证明整个原Root终态事务回滚，provider0/无回执。另一实际503已发出案例保留原请求未决费用1、第一项outcome_unknown/第二项not_started、reconciliation_required，原终态之后不重发，provider总1；没有把未知费用释放或宣称资源全收净。

新增实际UI首15/16（985.75ms）：旧v-else链因taskClosure遮住paid第一项和关闭第二项。让已验证有序卡片与终态提示并列，单项继续避免localReconciliation/closure/坏receipt重复展示。相关234/234（4.9706秒）后加强cancelled严格六字段closure、原assigned/disposition/flags/code/time形状及畸形IPC负向，再234/234和TS/Vite0（Vite2.16秒）。组件合成IPC不是安装态或真实LLM验收；SFC400行未增长。

首严格Clippy因两个相同disposition分支失败，短路OR合并保持pending/claimed/accepted不调用有序清理；不加allow。最终当前57精确测试名单次全过（52.75秒runtime），包括有序19、新关闭4、旧单项closure及原计划/相关终态；没有说全Master或旧九E2E通过。

- clippy-2：exit0，wall15.22秒；`/tmp/oviraptor-ordered-unsent-closure-clippy-2.log`。
- affected-2：exit0，wall84.58秒；`/tmp/oviraptor-ordered-unsent-closure-affected-2.log`。
- importer-2：exit0，wall36.86秒；`/tmp/oviraptor-ordered-unsent-closure-importer-2.log`。
- literal-2：exit0，wall1.41秒；`/tmp/oviraptor-ordered-unsent-closure-literal-2.log`。

9路径7已有/2新；1208文件集合SHA `a57a6a70bbc0d9d5845a598ca4c015e759bf61eb7a81103d9ab2e309fbcc5036`，1199原范围外字节保持，diff --check0；阶段开始逐文件before/prior git diff备份、后续范围先extend，再SHA/check/apply；最终合并差异在 `/tmp/oviraptor-ordered-unsent-closure` 前缀。

| 路径 | 行数 | 新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 150 | 否 |
| `src-tauri/src/commands/agent_tests_ordered_unsent_closure.rs` | 99 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/closure.rs` | 89 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution.rs` | 59 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/receipts.rs` | 210 | 否 |
| `src-tauri/src/commands/agent_directive_closure.rs` | 262 | 否 |
| `src/features/sentinel/directives/orderedAssessmentPlan.ts` | 128 | 否 |
| `src/features/sentinel/components/AgentDialog.vue` | 400 | 否 |
| `tools/test_ordered_human_execution_ui.cjs` | 132 | 否 |

未完成：executing但尚无durable dispatch、sent unknown/paid待投影worker尚不在本批资源清理范围；费用保留的Root终态不等于quiescent。过期/换C和跨进程重启收尾、所有clock出口、动态十维grant、provider精确对账/续跑、六Root触发、15角色/General ReAct/Broker/真实并发、完整UI、旧九E2E/正常paid删除仍未完成。之后才做完整门禁、安装/app启动与匿名授权URL。InputParser既有自动审批拒绝未重试/改名/绕过，完整理由未提供，不猜测。Goal active。
