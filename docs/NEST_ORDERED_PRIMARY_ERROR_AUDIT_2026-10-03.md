# 有序 SDK 主错误审计 · 2026-10-03

仅临时SQLite和实际localhost SDK，未操作真实DB/CAS、资产、安装/URL，未提交。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`保持，Master未完成。

新实际401负向0/1（0.51秒）证明原SDK返回错误被丢弃，只返回budget_indeterminate_requires_reconciliation；错误断言之后费用、replay与503分支未达。原已付/未知账单仍由原SDK处理。

仅将丢弃返回改保存transport_result，后续parent检查和原received合并为Result；两者失败时Err(original)先于ordered_proposal_receipt次错误，Ok保留原local错误。返回值不成为发布或重试依据，仍只凭原已提交physical receipt发布。未知结果、原worker/C/费用/Native JSON/ACK与建议未执行语义保持。此修复限定有序parent/received边界，不宣称所有错误/clock出口或后续delivery失败均已完整处理。

新增三个实际测试：401/503原错误+未决请求费用1、无usage确定请求消耗1与Token未决4000、provider尚阻塞时pause引起实际SDK取消先于parent次错误。均不得发布paid评估/派发第二项，原Native合同保持；纯本地apply_ordered_human_actions回放全行不变/SDK1。原take_human_directives队列入口会合法renew C，不冒充零写只读函数。

第一次扩大19编译E0373，无runtime结果；仅scoped thread显式borrow+move修测试。之后17/19（29.51秒）两个测试预期失败：完整队列入口refresh C导致全行比较变化，无usage原已知request consumed1而测试错期待indeterminate1。未放松生产，改直接有序本地回放并核原费用分类；最终19/19（29.15秒）。原16有序用例名保留，未重复未变化前端或完整273，全Master门禁仍待框架完成。

- clippy：exit0，wall14.86秒，`/tmp/oviraptor-ordered-primary-error-clippy.log`。
- importer：exit0，wall20.94秒，`/tmp/oviraptor-ordered-primary-error-importer.log`。
- literal：exit0，wall1.23秒，`/tmp/oviraptor-ordered-primary-error-literal.log`。

3路径2已有/1新；1206文件集合SHA `b1f1d09dc27e1eefaf53bec7bf2f7691715790628408e65d59c8f323b466ad34`，1203原范围外字节保持，diff --check0；before、prior git diff和逐文件最终合并差异在 `/tmp/oviraptor-ordered-primary-error` 前缀。首次apply错误SHA校验拒绝，未写文件；重新核实际SHA后check/apply。

| 路径 | 行数 | 新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 149 | 否 |
| `src-tauri/src/commands/agent_tests_ordered_primary_error.rs` | 81 | 是 |
| `src-tauri/src/commands/agent_directive_ordered_actions.rs` | 159 | 否 |

未完成：有序prepared/已发出未知/paid待发布的原任务退出资源清理；动态十维grant、精确provider对账/显式续跑、六触发、全15角色/General ReAct/Broker/实际并发、唯一静止终态/跨进程恢复、整体UI、旧九E2E与正常paid删除，之后完整门禁/安装app打开/授权URL。InputParser既有自动审批拒绝保持，原完整理由不可见，不猜测。Goal active。
