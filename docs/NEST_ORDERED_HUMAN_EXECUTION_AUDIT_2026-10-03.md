# 人工有序双角色执行审计 · 2026-10-03

Master仍在开发。仅临时SQLite、localhost provider、真实SDK消费者及组件IPC夹具；未操作真实数据库/CAS/资产、安装态、授权URL，未提交。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`保持。

## 问题、作用域与行为

旧新Web二角色仅v2 not_connected预览，真实执行入口没有有序worker、派发、前项回执和ACK。新增v3仅新creator：按原书写顺序Mapper/Investigator，两项4000 tokens/1模型请求/maxOutput512，合计8000/2/0目标请求。完整计划冻结hash/原确认，仍requires_dispatch_checks而非直接授权；人工审批原immutable actions为not_started、receipt空。历史v1/v2不自动升级，原Source语义不升级，unsupported reviewer/host/scope仍原规则拒绝。

执行只认原Web Multi、原parent监督、Root财务合同、C与UUID worker，原shared SDK冻结请求加入humanDirectiveDispatch绑定完整费用/原scope/input/计划/predecessor。每次原Root迭代一个人工角色；第二项必须前项有效advisory receipt/原paid event/usage/原worker及真实mailbox ACK，distinct worker/child，复用首项原冻结证据，不吸收后来context。没有工具或目标请求；建议、人工确认、执行回执均不冒充已执行建议、漏洞确认或独立Reviewer批准。

实际返回先保存原物理SDK账单，再local assessment/checkpoint/settlement/result mailbox消费ACK/immutable receipt。无usage、invalid、unknown第一项都不能派发第二项；重放不重新HTTP或伪造新SDKstage。singleton读v3会完整校验原projection/source并拒legacy singleton混入，然后返回None；不能仅按schemaVersion忽略坏历史。timeline/UI读取相同已验证投影；前端严格核完整作用域、receipt顺序/usage/predecessor，未知不当零费用或完成。

Schedule/Consume/Start/Receive/Finish及预算Defer采用新RW/no-CREATE/FK ON/sync FULL私有连接；caller已在事务内拒绝，不替换caller hook。authorizer在BEGIN之前启用，贯穿COMMIT/ROLLBACK；Writer.verify只读检查，不在Drop清hook。仅原阶段相关表/列和准确canonical协作event集合；触发器IGNORE、额外业务写、foreign Root/draft、篡改/伪造emitter和旧event均拒并回滚。通用SDK start等全部路线隔离尚未完成。

原Web prepare为充足budget保留Reviewer8000/1和人工两项8000/2，再给Executor原余量，仍确保原最低4000/1。较紧pool保留旧单项或零人工余量，v3第一项预算不足时只把本指令accepted→deferred/ordered_budget_unavailable，严格一条原canonical通知，无child/worker/call/额外reservation；原Executor正常继续。已付第二项不足及未知对账错误仍严格拒绝，不退款或当普通暂缓。原Native Root hard/deadline未改。

## 实际负向与修复过程

首Rust0/12（15.77秒）在实际入口缺派发、checkpoint/receipt缺失及provider到达超时失败；未知第二项等后续断言未达，不能当分支全证明。首实际SFC/composable UI3过/5失败。初次合入编译E0505，没有runtime测试；仅修guard borrow。

第二Rust1过/12失败（18.08秒）：Schedule拒原run_create UPSERT编译时引用run_update trigger，多数尚未SDK。仅允许该原emitter并继续精确event集合，不能允许新run偷写update。Native首次实际7SDK到第二项时旧singleton reader误识别v3回执，严格完整v3读验证修复；原executor headroom扩成两项。

扩大新14首10过/4失败（21.08秒）：两生命周期provider capture在handler返回后，断言seen时机错误；原不可变SDKtrigger阻止故意损坏receipt测试；caller fixture未先claim。修这些测试边界后实际2过/2失败（6.69秒）中的caller hook丢失证明生产安全缺口，私有连接修复；另测试错用immutable trigger名更正。随后14/14（25.72秒）。

紧预算首次16k fixture在bootstrap容量就失败，未达目标行为；改创建前原hard modelRequests6，不改live预算，实际Native4SDK前因ordered_budget_reserved_for_review使pipeline提前终止（1.18秒），证明预算暂缓缺口。最小Defer修复及IGNORE/business/forged canonical event三故障完整rollback，新增16有序+原容量case1共17/17（28.25秒）。首严格Clippy发现两个type_complexity和一个never_loop，改显式SQL row alias与同语义单项选择，无allow压制。

扩大273首271过/2失败（453.94秒）：headroom期待旧单项4000/1而实际二项8000/2；新v3 human review能力错误仍按多角色未分解条件blocked。前者仅修测试费用/请求期望，后者只对完整typed schema3/requires_dispatch_checks免旧分解条件，后续unsupported角色约束保持，actions仍not_started/空receipt/原policy仍必须。最终当前生产273单次全过。

- clippy-3：exit0，wall15.41秒，`/tmp/oviraptor-human-ordered-execution-clippy-3.log`。
- affected-3：exit0，wall473.31秒，`/tmp/oviraptor-human-ordered-execution-affected-3.log`。
- importer-3：exit0，wall49.29秒，`/tmp/oviraptor-human-ordered-execution-importer-3.log`。
- literal-3：exit0，wall1.28秒，`/tmp/oviraptor-human-ordered-execution-literal-3.log`。

相关UI232/232（4.8447秒）及TS/Vite0；之后仅Rust变更，未重复未变化前端门禁。阶段未跑全量Master门禁。

36路径18已有/18新，1205文件集合SHA `70583e346cd78e8eae85f6b67ebf78404231c5ad780398c4ef95dbe7910331b8`，1169原范围外字节保持，diff --check0。开始前逐文件before/prior git diff备份、所有范围extend先于修改，补丁SHA/check/apply及最终逐文件合并差异在 `/tmp/oviraptor-human-ordered-execution` 前缀。

| 路径 | 行数 | 新增 |
|---|---|---|
| `src-tauri/src/agent_runtime/multi_agent/directive.rs` | 180 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/human_review/store.rs` | 221 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution.rs` | 57 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/schema.sql` | 68 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_plan.rs` | 232 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/scheduling.rs` | 244 | 否 |
| `src-tauri/src/db.rs` | 398 | 否 |
| `src-tauri/src/commands/agent_tests_ordered_execution.rs` | 328 | 是 |
| `src-tauri/src/commands/agent_tests_ordered_execution_guards.rs` | 230 | 是 |
| `src-tauri/src/commands/agent_tests_ordered_execution_helpers.rs` | 177 | 是 |
| `src-tauri/src/commands/agent_tests_ordered_execution_lifecycle.rs` | 185 | 是 |
| `tools/test_ordered_human_execution_ui.cjs` | 107 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/binding.rs` | 223 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/financial.rs` | 223 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/lifecycle.rs` | 113 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/receipts.rs` | 207 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/scheduling.rs` | 242 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/writer.rs` | 124 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/writer/authorization.rs` | 178 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/writer/events.rs` | 184 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/mailbox/saved_message.rs` | 153 | 否 |
| `src-tauri/src/commands/agent_directive_ordered_actions.rs` | 155 | 是 |
| `src-tauri/src/commands/agent_directive_proposals.rs` | 335 | 否 |
| `src-tauri/src/commands/agent_tests_ordered_human_plan.rs` | 235 | 否 |
| `src-tauri/src/commands/native_scan_branches/status_timeline.rs` | 382 | 否 |
| `src/features/sentinel/components/AgentDialog.vue` | 400 | 否 |
| `src/features/sentinel/composables/useAgentDialogDirectives.ts` | 351 | 否 |
| `src/features/sentinel/directives/orderedAssessmentPlan.ts` | 123 | 否 |
| `src/types.ts` | 1744 | 否 |
| `src-tauri/src/commands/agent_tests.rs` | 148 | 否 |
| `src-tauri/src/commands/agent_tests_ordered_execution_native.rs` | 89 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/receipts.rs` | 234 | 否 |
| `src-tauri/src/commands/multi_agent/prepare.rs` | 336 | 否 |
| `src-tauri/src/commands/agent_tests_ordered_execution_caller.rs` | 51 | 是 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_regression_1.rs` | 210 | 否 |
| `src-tauri/src/commands/agent_tests_tick_prepare.rs` | 85 | 否 |

## 未完成与风险

旧单项退出清理不识别有序动作，prepared/已发出unknown/paid未投影需按原scope分别处理；不能把Root终态当作worker全静止或费用已清。SDK返回主错误目前可能被parent/receipt错误遮盖；typed clock出口/精确provider账单对账与显式续跑仍未完成。动态十维grant、六Root触发、15角色/General ReAct/Broker/真正并发、唯一静止终态及跨进程重启、正常paid任务删除、整体UI与旧九E2E仍未完成。之后才做完整门禁、安装/app启动和已授权匿名URL。

InputParser既有自动审批拒绝未重试、改名或绕过；现有材料缺完整拒绝理由，不猜测。Goal active，本批不标Master完成。
