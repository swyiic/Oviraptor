# Root 人工确认聊天投影审计 · 2026-10-05

Master未完成，Goal active。本批对应REM-A03/A08/A09/A10/A11的局部闭环，不能勾完任何整类。

## 原问题与作用域

上一批真实人工确认已进入Root SDK并产生原费用/发布回执，但public_projection与状态聊天行仍固定coordinator线程；原team、target或worker聊天看不到同一次评估，HumanDirective步骤和延期原因可能直出内部字符串。

先加入临时真实creator/HMAC/Root/Mapper生产者测试；有效Rust修改前5项2通过/3失败，失败均为原线程与原绑定读。Vue新六项1通过/5失败，仅模拟IPC。首次Rust红灯5失败中有两项夹具错误：错误要求100000超预留token直接消费、全表比较未分离生产Native入口合法worker心跳。更正为保留原received journal报告及未决token、实际入口财务不变与直接事务零写各自断言后再运行有效红灯。没有重置费用、修改限额或SQL还原心跳，也没有宣称现有财务分类有生产缺陷。

## 最小生产变更

- 原已付投影先验证冻结原request、basis、invoice、publication、timeline，再从human-directive输入选择schemaVersion/directiveId/draftId/revision/draftHash/confirmationReceiptId/threadKey/targetKey八项显示字段。revision为安全正整数、hash为小写64hex，身份及线程有界且无控制字符，原target相同。
- 状态聊天行采用上述原线程；非人工决策继续原coordinator线程。现有Native schemaVersion1、原事件/call/modelEvent/游标/费用不改，context为可选当前Native显示元数据；无Strix正向兼容、无存储迁移/旧事实补造。
- Vue严格校验context八个字段、原线程/target、事件/call/usage及字段个数；坏行不前移游标或消费当前视图。显示原确认版本、已保存建议和使用量，人工步骤与延期原因译为可读文案；建议保持advisoryOnly，不能声称已派发或执行。
- 没有修改财务cost/receipt策略、Root重试/lease、确认和动作准入。已付历史读取不查询当前directive/draft，不借显示取得执行权限。

## 六项真实后端回归

1. 原team/target/worker/root分别确认并实际付费；实际status增量、timeline分页、原事件去重及回执元数据一致。每个producer原Root4/Mapper1共5SDK，无Web或目标调用。
2. 同原Root的两次team/worker确认有各自原directive/thread/event；两次实际Human SDK后总6SDK，纯重放无新SDK/写入。
3. 临时当前指令thread损坏仍返回原付费thread；再仅在临时DB解除immutable测试触发器并篡改已付request，原hash验证以root_tick_original_input_conflict拒绝。真实schema未改。
4. 实际drop原parent session后执行授权拒绝，原已付摘要仍可纯历史读取；typed全表与物理rowid均零写，无新SDK/恢复/费用。
5. 实际100000token超原预留的响应保留原received journal usageReported与inputTokens；token未决、known原modelRequests4，真实Native返回REQUEST_RECONCILIATION_REQUIRED，无人工publication、动作或Web。此项证明原已有分类正确，非财务修复。
6. 临时deferral writer伪造budget/indeterminate错误仍是真正AGENT_STOP_PERSISTENCE，原财务不变；生产入口合法heartbeat不当作deferral写入，直接deferral事务typed/physical/inbox零变更。

新增六项6/6（测试23.43秒，阶段45.32秒）；原五项绿灯记录已保留在/tmp/oviraptor-human-chat-five-green，不覆盖原红灯证据。

## 同源门禁与证据

本轮完成原人工确认付费摘要的聊天投影：只有核验原request、invoice、publication与timeline后，才从原冻结输入选择八项确认元数据并绑定原线程；非人工Root保留coordinator线程。实际临时SDK生产链路覆盖team/target/worker/root、两确认隔离、损坏与篡改、父退出历史读取及原财务退出分类。已付历史读取不查询当前指令、不恢复或授予执行权限，不迁移旧账本或补造费用/发布回执。

同源码关联356/356（测试581.58秒/阶段582.40秒，含原341及九项既有投影、六项新增，无ignore），严格Clippy0（9.54秒），退役50/50（5.36秒，含literal/当前Native JSON）；四叶局部fmt与16路径diff0。聊天组件模拟IPC345/345（14.08秒），vue-tsc/Vite构建0（阶段7.19秒）。这些是组件回归与构建，当前2427项Rust全量、整体UI、安装App打开/历史崩溃、授权URL与真实模型质量/美元对账尚未验收。

16代码路径12已有/4新保存逐文件前像、原Git差异与最终增量；1393原范围外源码与HEAD59be3d86保持，1409源码SHA aba34b5e74134bdc53afd379431fa769d6e60179029f19340f7165bc5a6d10e8。修改前有效Rust2/5、UI1/6；已更正首次财务测试错误预期：超预留已报告usage保留在原journal且token归未决，现有分类已经待对账，未改生产财务策略；真实worker心跳与事务零写分别核验。仅临时SQLite/CAS/localhost SDK与组件模拟，未操作真实DB/CAS/asset、安装App或授权URL。

| 门禁 | 对应源码集合SHA | 通过数或退出码 |
|---|---|---|
| 有效Rust红灯 | c1214146243b6a5321705e088b665c8ddfc4946aba5e90cacc1aeaad525c17d4 | 2 |
| Vue红灯 | 208e767c9cd34b16cf83bc7aef623c9418724f8c79bdb719fb0abb6666ddfd0f | 1 |
| 新增Rust绿灯 | aba34b5e74134bdc53afd379431fa769d6e60179029f19340f7165bc5a6d10e8 | 6 |
| 同源关联 | aba34b5e74134bdc53afd379431fa769d6e60179029f19340f7165bc5a6d10e8 | 356 |
| Clippy | aba34b5e74134bdc53afd379431fa769d6e60179029f19340f7165bc5a6d10e8 | 0 |
| 退役 | aba34b5e74134bdc53afd379431fa769d6e60179029f19340f7165bc5a6d10e8 | 50 |
| 聊天组件 | aba34b5e74134bdc53afd379431fa769d6e60179029f19340f7165bc5a6d10e8 | 345 |
| 前端构建 | aba34b5e74134bdc53afd379431fa769d6e60179029f19340f7165bc5a6d10e8 | 0 |

第一次完整关联356项355通过/1失败（测试629.73秒/阶段631.41秒，原源码ec73be3e），完整失败日志保存在first-full-red。原executor_allocation_original_receipts_scope_and_foreign_target_history_reject_zero_write主动将worker票据过期；真实原父监督会另行撤权，失败是web_mode_assert_rows观察到agent_assignment_attempts改变。原测试单独重跑12.23秒通过并不足以收口。唯一额外修改是该既有用例在worker负向先等待并证明原父实际完成过期/paused/撤回能力，核验原物理worker身份、C/预算/原模型费用保持，再比较被拒绝调用typed全表及物理rowid零写。未停监督、伪造新父、还原SQL、扩大限额或删除断言；原三测试名均保留。修正后该叶三项3/3（测试15.25秒/阶段15.26秒），最终重新跑整个356，不能用旧355与单独通过拼接。

关联356严格包含原341，以及原tests_native_root_decision_timeline四项、tests_root_decision_chat_cursor_guards五项与新增六项。选择/报告/通过集合完全相等，无ignore；原既有测试文件未改。全二进制列出2427项，不能把356当2427。聊天345为npm test:agent-dialog整组组件测试，前批28为其中聊天决策相关子集；组件模拟不等于实际安装IPC。

临时证据前缀/tmp/oviraptor-human-chat：baseline/before/prior-diffs/reviewed-merge-diffs/scope-final/code-snapshot、red/green/final日志与result、selection-proof、clippy/retirement/ui-green/frontend-build结果。首次红灯另保存在first-red，Vue红灯独立ui-red，财务测试错误预期保留可追溯。一次临时Node结果采集器未识别ℹ报告格式而退出，产品345项已全通过；修正采集器后完整重新运行同源345项及build并取得terminal0，未改产品源码。

## 逐文件保护

基线1405文件SHA ce3d09e2255769d11c4d58e2de2e3fff6573b6633424bef0aa45685cb4c1d5d5；原范围外1394不变。每条路径都有前像、原git diff及仅本批增量；新增4路径，已有12路径不批量覆盖。HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b未改，无commit/reset。

| 路径 | 本批范围 | 最终文件SHA |
|---|---|---|
| `src-tauri/src/commands/agent_tests_root_human_chat.rs` | 新增 | `15ec83730367b7134feaa895d21ad04eed6813c0c8104299d0f5825b8d730b2a` |
| `src-tauri/src/commands/agent_tests_root_human_stop.rs` | 新增 | `93204bb39265acd4bfd080ad11b54480af8ef57f8fa7405808dc24d8333c73d6` |
| `src-tauri/src/commands/agent_tests.rs` | 保留原改动后增量 | `2e55f83f67c3141e0dfe1b85cc096e83445b8e40740a092364fc763e19633db7` |
| `tools/test_root_human_chat_ui.cjs` | 新增 | `85c19cddd90e5ec1dac17396c9c16ca6024e11d6aa218b3065020cba435c7308` |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/human_projection.rs` | 新增 | `8ce072ba2c9afd2ff59b2ee9c493db6b170cc477ad51b906c3776d924045b303` |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick.rs` | 保留原改动后增量 | `1600b170c7b0d87efd594430bb6d91206d392dafe25d0c0d3572acafcc5e2368` |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/public_projection.rs` | 保留原改动后增量 | `c3b6e771a0d556a4a8011a68ffb5144a01a8d8cf06eda49f307f11dd143d51dd` |
| `src-tauri/src/commands/native_scan_branches/status_root_decisions.rs` | 保留原改动后增量 | `0581c9ac660ec691527f254dfe86ef40f1d18f32b65d5fe18f3e0bba6eee8d0c` |
| `src/types.ts` | 保留原改动后增量 | `e39b9b25ebfed55fe6f506708fd711bb5f2ab5108d2ccfce84591f93958147ef` |
| `src/features/sentinel/traces/rootDecisionContract.ts` | 保留原改动后增量 | `4aad9157a15fe52028bbf2304575a6d7d4cabdf4476d3660110ee38cc91025eb` |
| `src/features/sentinel/timeline/rootDecisionContract.ts` | 保留原改动后增量 | `6f95b5e0610fb1198463c0301913f96cead8ae24f1b15626c37da1d4358c967f` |
| `src/features/sentinel/components/RootDecisionSummary.vue` | 保留原改动后增量 | `d517b910160c78eb7b157a2020cdfeb65a4bfd1b915f899ca8b6d617632d45b9` |
| `src/features/sentinel/components/agentDialogLabels.ts` | 保留原改动后增量 | `71074137a4067c35bcd679c6ff048fff1543dc3701407edd5d546340f9f09b5e` |
| `src/features/sentinel/components/AgentDialog.vue` | 保留原改动后增量 | `83b5dbfb54429b3db946f5b0d84658af09f0e80c76a455d1f575fc765ff53e89` |
| `package.json` | 保留原改动后增量 | `5885913e0228ca9d28e80662f9abd65e8f63705395e774c1295d0a7e148cb00c` |
| `src-tauri/src/commands/agent_tests_executor_allocation_negative.rs` | 保留原改动后增量 | `0fb9445409fa8b30d01bf8b7ce694290352b67f4cb75abea95f8a927c8a62505` |

## 未完成与风险

下一处理原确认评估的未知/失败与显式恢复闭环，以及剩余Root触发；继续十维预算、全15角色真实执行与并行、逐路日志和整体UI。框架完成后再做全量门禁、同源安装App打开/历史崩溃、两授权URL匿名只读及真实模型质量。真实清理先精确盘点与备份，不删除asset。

本批不解决人工未知费用的明确对账/恢复、所有失败和超时展示、自由聊天ReAct或真实模型推理质量。父退出历史读取仅是读，不证明重启/续跑。小版本徽标与可读文案不是整体UI美化完成。临时脚本SDK运行真实生产链路，但不证明真实供应商理解、费用美元正确或漏洞验证。无真实DB/CAS/asset操作、安装App或授权URL访问。

十四类完整剩余见[Master顶部](NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md)，全部保持未完成范围。
