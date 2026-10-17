# 人工多角色计划合同审计 · 2026-10-03

Master未完成。仅临时SQLite/localhost provider与组件IPC夹具；未触碰真实DB/CAS/资产、安装态、授权URL，未提交。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`保持。

## 实际问题与最小修改

原两个只读角色仅按角色集合记录4000 tokens/1请求，未冻结动作顺序、每项费用及前项依赖。增加可选readonlyAssessmentPlan，只有新建有效read_only agent_proposal_request且恰好Mapper/Investigator（可含Coordinator）、无contracts/scope-change/pause时产生v2 marker与冻结计划。按原文本首次mentions排序，两项各4000/1，maxOutput512，0目标请求；第二项绑定前项actionId和valid_advisory_receipt。originalRoot/C/source/revision等形成binding hash，完整计划加入原draft hash和confirmed payload，确认仍使用原rev/hash校验。未新建执行table/grant/child/mailbox/SDK/ACK。

原历史未标v2 rows仍原hash recipe及原Native JSON；不迁移原row，不补确认或权限。原Source Root的persisted surface=source阻止Web计划附加。v2只是Web评估预览，不能替代当前WebMode/原财务/parent权限证明；尚未接执行，明确proposal_ordered_execution_not_connected暂缓。immutable human review仍not_started/无receipt。

实际SFC和composable加载49行typed consumer，严格核计划keys/角色/费用/顺序/依赖；展示逐动作及总上限。confirm返回完整计划须与原draft相符，损坏IPC不能清空composer。frontend不授予权限，不将确认标执行完成。

## 负向、回归与证据边界

首Rust2/6（0.79秒）：实际draft合计4000而期待8000、计划缺失、estimated_tokens=4000篡改未被拒、修订缺计划；原legacy和unsupported用例通过。首实际组件UI3/6证明卡片缺失、完整计划confirm比较缺失及坏IPC清空输入。修后同Rust6/6（1.46秒）和相关UI224/224（4.553秒），TS/Vite构建通过。纯preview测试夹具并非真实财务/SDK验收。

单独原Source回归0/1先证明新Web计划意外改变Source二角色暂缓原因；保留原测试原语义，读取原persisted surface后修生产Scope，扩大回归该测试通过。扩大97项94通过/3失败（269.89秒runtime、302.09秒wall）：两旧正向队列fixture缺当前Mode/原财务identity，目标优先级行为前即被正确拒；另一项新Web二角色期待旧decomposition原因。两正向移出原长文件到77行新叶，原测试名称保留，真实creator/HMAC/冻结原Root/原财务/parent；新计划断言仅改明确暂缓码，未放松生产。3项再通过（3.47秒runtime）。原94生产代码未再改变，精确名字集合交集0、并集97；不宣称97单次全过。

真实Native优先级测试实际2 Root+1 Mapper+2 WebExecutor SDK，两执行者请求pendingQueue首authorization，原已应用指令不冒充待执行模型context。另child队列测试真实Root2+Mapper1后仅本地队列动作和completed-rule只读重放；人工改C不用于adopt任何已付Root/工作，不宣称另有WebExecutor SDK。两项均原supervisor先Drop再临时目录清理。

- clippy：exit0，wall30.44秒；`/tmp/oviraptor-human-ordered-plan-final-clippy.log`。
- importer：exit0，wall45.84秒；`/tmp/oviraptor-human-ordered-plan-final-importer.log`。
- literal：exit0，wall1.25秒；`/tmp/oviraptor-human-ordered-plan-final-literal.log`。

18路径12已有/6新；集合1187 SHA `a4c880e42581ba34ecdd1e244a97cd37550027711974a36b95995f097e68f456`，1169原范围外字节保持，diff --check0。baseline/before/prior-diffs及最终逐文件merge diffs在`/tmp/oviraptor-human-ordered-plan`前缀。Source边界和fresh actions补丁均先extend、SHA核验、apply --check再最小hunk应用；无批量格式化。

| 路径 | 行数 | 新增 |
|---|---|---|
| `src-tauri/src/agent_runtime/multi_agent/directive.rs` | 179 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/draft_store.rs` | 437 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_plan.rs` | 220 | 是 |
| `src-tauri/src/commands/agent_tests_ordered_human_plan.rs` | 238 | 是 |
| `src-tauri/src/commands/agent_tests_ordered_human_plan_guards.rs` | 170 | 是 |
| `tools/test_ordered_human_plan_ui.cjs` | 91 | 是 |
| `src-tauri/src/commands/agent_tests.rs` | 142 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/human_review/store.rs` | 218 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/scheduling.rs` | 243 | 否 |
| `src/features/sentinel/components/AgentDialog.vue` | 384 | 否 |
| `src/features/sentinel/composables/useAgentDialogDirectives.ts` | 351 | 否 |
| `src/features/sentinel/directives/orderedAssessmentPlan.ts` | 49 | 是 |
| `src/types.ts` | 1732 | 否 |
| `tools/agent_dialog_harness.cjs` | 310 | 否 |
| `src-tauri/src/commands/tests_human_directive_review_roles.rs` | 89 | 否 |
| `src-tauri/src/commands/agent_tests_directive_actions.rs` | 370 | 否 |
| `src-tauri/src/commands/agent_tests_directive_actions_fresh.rs` | 77 | 是 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_regression_1.rs` | 210 | 否 |

## 未完成和风险

二角色实际执行、费用/回执/ACK前项依赖及任务关闭清理尚未接线。后续新v3仅新creator可声明按顺序模型评估授权，原v1/v2不自动升级；必须实际Web Multi/原parent/财务/C/worker和原scope再检查。当前WebExecutor分配可能只留单项人工headroom，需真实Native入口证明并修二项容量；不可拿无executor的RootTick fixture成功代替该集成。

动态十维grant/精确对账/显式继续、六Root触发、15角色/General ReAct/Broker/真实并发、唯一终态/跨进程恢复、正常paid删除、整体UI/旧九E2E仍未完成；后续完整门禁、安装/app打开和匿名授权URL验收。InputParser原自动审批拒绝保持，不重试或改名绕过；已有材料缺完整拒绝理由，不猜测。Goal active。
