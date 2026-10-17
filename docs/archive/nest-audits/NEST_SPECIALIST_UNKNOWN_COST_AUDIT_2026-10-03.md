# 专家未知费用保留审计 · 2026-10-03

HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`不变；5路径4已有/1新，无重置/批量覆盖/提交/真实DB/CAS/资产/安装/授权URL操作。Master未完成。

## 问题、范围与最小修复

真实ordinary creator/HMAC/原Root/C/UUID worker派发tool-free Mapper到实际loopback SDK，服务返回503。BEFORE uncertain UPDATE IGNORE使原事务回滚，原已发送请求未进入indeterminate。首0/1（0.45秒）在consumed0/indeterminate0而期望0/1失败；原503与阶段写入错误断言已达到，原fact/全表重放与最后SDK次数断言未达到，当时只达到第一IGNORE分支，第二ABORT分支未达到。

record_uncertain将原事务体提为record_uncertain_on，Err必须在该事务drop/rollback之后调用受限failed_cost::uncertain。received和uncertain共用同一私有原费用writer，仅直接INSERT agent_model_cost_facts/budget_entries，原authorizer贯穿BEGIN/COMMIT；原Root/C/worker/role/原pristine request完整绑定与前后证明不变。Uncertain保存原预留为未决，无响应/邮箱/event/snapshot/ACK/grant/退款/自动重试；失败保留原错误并追加specialist_cost_after_uncertain_failure。普通/晚成功uncertain原语义保持，received路径只是复用内部write函数，未改变费用意义。

## 验证

两实际SDK用例包含IGNORE/ABORT两阶段故障及model_cost fact/forfeit entries两触发器业务写故障。前者503与原阶段错误保留、原请求indeterminate1/consumed0、原Root准入拒绝、原Root/assignment/child与fact一致；所有非费用/诊断表不变。后者原业务和费用均完整回滚、503与次费用错误保留；两者同transport重放所有application rows不变，provider仍各1次。不把触发器故障视为未发送，不强行释放预留。

比较器排除native_sdk_log_*及sqlite_sequence，并在费用允许时只再排除两金融表；不能据此独自声称所有physical rowid或日志内容验证。真正第二次transport重放比较全部application表。临时SQLite/loopback只证明生产合同，不证明配置模型质量、安装或授权目标。

修后45/45（49.00秒）。最后93/93（516.62秒）包含Source专家/Reviewer/财务退出回归，严格Clippy、导入39、exact退役1通过；未运行全部Rust/安装态/URL。

- clippy-final：exit0，wall22.25秒；`/tmp/oviraptor-specialist-unknown-cost-clippy-final.log`。
- affected-final：exit0，wall517.45秒；`/tmp/oviraptor-specialist-unknown-cost-affected-final.log`。
- importer-final：exit0，wall27.76秒；`/tmp/oviraptor-specialist-unknown-cost-importer-final.log`。
- literal-final：exit0，wall1.26秒；`/tmp/oviraptor-specialist-unknown-cost-literal-final.log`。

集合1161 SHA `04bff220e77c64f8f6712a526154b7e9adca441d26641886ccd3d5c345a23d7f`；1156原范围外字节不变；逐文件before/prior/reviewed-merge-diffs保存于`/tmp/oviraptor-specialist-unknown-cost`前缀，diff --check0。测试patch5781db2d、生产patch068c2a86；新叶独立fmt，原父文件格式保留。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 128 | 否 |
| `src-tauri/src/commands/agent_tests_specialist_unknown_cost.rs` | 94 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/specialist.rs` | 355 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/receipt.rs` | 314 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/receipt/failed_cost.rs` | 75 | 否 |

## 未完成与风险

普通/晚received以及uncertain/notsent一般写入权限仍需封闭；本批只封费用fallback。非法usage、响应与持久化间强杀、精确provider账单对账/显式继续、动态分配、全角色/Broker/真实并发、有序人工聊天、整体UI/旧九E2E/正常paid删除均未完成。下一项用实际SDK证明普通uncertain触发器副作用后最小修复，不能盲目应用Client草案。InputParser旧自动审批拒绝保持，不重试或绕过。Goal active。
