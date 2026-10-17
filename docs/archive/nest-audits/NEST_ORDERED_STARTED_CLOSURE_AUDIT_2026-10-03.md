# 有序已派发 worker 退出审计 · 2026-10-03

Master未完成，仅临时SQLite与实际localhost SDK；无真实DB/CAS/资产、安装/授权URL或提交。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`保持。

原未发出清理仅覆盖prepared；实际503 SDK返回后原finish_coordinator_run提交Root终态，但3个capability与1个lane仍活跃。新测试最初编译因Rows缺Debug（未有runtime），仅换boolean相等断言，不改生产Rows；首实际负向0/1（0.53秒）在active4而期待0失败，后续费用/状态/重放未达。

原closure变typed NotApplied/ReconciliationRequired，并新增83行started叶。v3原Source确认/完整计划projection、Root财务原C与scope、UUID worker和原frozen input/请求消费/完整durable SDK metadata先核；received需原费用/response/event projection。只认running/paused原readonly assignment/child/worker、无finished/settled；不采用新fence/expired worker，不造identity或grant。原合法paid前项receipt先验证，未知第二项不能覆盖前项。

在原Root终态受限事务，仅暂停原assignment/child/worker、设置固定reconciliation failure、撤原scope capability、删除原readonly lane。预先生成完整原runtime期望行，仅state/failure/updated/revoked允许变化；原所有Root账本/费用entries/specialist calls/model cost facts/messages/ordered checkpoints/receipts快照前后完全相同，真实SDK metadata再次核验。费用/预留/原SDK错误由原组件保留，不settle、不refund、不产生结果或ACK。RootClosure authorizer与准确原emitter检查不放松，任一写/读回失败整个Root/clock/指令关闭回滚。

首修后实际1/1（0.56秒）；新增4/4（4.99秒）：503原费用/派发行和4000/1预留不变、0执行leases、worker paused且终态纯回放；200实际paid response因receipt IGNORE留received，关闭不补发布/ACK/费用；第二项实际503保留第一项原paid receipt与独立identity/provider总2；assignment/capability/lane/worker IGNORE和附带projects/账本写六故障用全应用行snapshot证明整个Root事务回滚，原provider1/费用保留。

最终61个精确测试名单次全通过（55.02秒runtime），包含原57有序/人工计划/单项关闭/相关终态及新4，不宣称完整Master或旧九E2E通过。前端与原Native JSON/合同未改；不重复未变化前端门禁。

- clippy：exit0，wall26.24秒；`/tmp/oviraptor-ordered-started-closure-clippy.log`。
- affected：exit0，wall55.63秒；`/tmp/oviraptor-ordered-started-closure-affected.log`。
- importer：exit0，wall45.92秒；`/tmp/oviraptor-ordered-started-closure-importer.log`。
- literal：exit0，wall1.24秒；`/tmp/oviraptor-ordered-started-closure-literal.log`。

6路径4已有/2新，1210文件集合SHA `6bd08cdf0a27ba2fb82f9000c926846b8ea83ec8213e218a4ae571edfff6069d`，1204原范围外字节保持，diff --check0；阶段开始逐文件before/prior diff备份，扩范围后SHA/check/apply，最终合并差异在 `/tmp/oviraptor-ordered-started-closure` 前缀。

| 路径 | 行数 | 新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 151 | 否 |
| `src-tauri/src/commands/agent_tests_ordered_started_closure.rs` | 102 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/closure.rs` | 100 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution.rs` | 59 | 否 |
| `src-tauri/src/commands/agent_directive_closure.rs` | 268 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/ordered_execution/closure/started.rs` | 83 | 是 |

未完成：逻辑paused/撤权不证明模型HTTP、队列或进程全部join，也不是canonical Root全静止；在途/晚费用尚需完整收口，未知费用与已付待发布不可当结清。executing但无durable intent、SDK typed not_sent的精确释放、过期/换C/跨进程恢复仍需补齐。动态grant/精确provider对账/显式续跑、六触发、15角色/General ReAct/Broker/实际并发、整体UI、旧九E2E、正常paid删除仍未完成。之后完整门禁/安装app启动/授权URL；InputParser既有自动审批拒绝未重试/改名/绕过，完整理由未提供不猜测。Goal active。
