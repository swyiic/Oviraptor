# 人工提案原监督 SDK 审计 · 2026-10-03

本批27路径，13已有/14新；保留原未提交差异，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`不变。未触碰真实库/CAS/资产/安装或授权URL，未提交。Master未完成。

## 原问题、范围与修改

已确认人工提案原来在 command 直接调用 Gateway，实际成功响应没有原 worker 的 SDK 派发与费用证明。首实际 loopback provider 测试0/1（0.47秒），收到响应后原 worker dispatch count 0而应为1；该断言后的实时阶段/ACK/次数断言未到。接口测试准备曾补缺少 include、真实 blocked-provider 阶段采样与原 timeout 夹具，不能把这些准备误记成已执行负向结果。

现在仅支持既有单个 Mapper/DeepInvestigator 只读提案，4000 tokens/1 request。共享 child transport 的可选绑定入口在原完整 immutable request 保存 humanDirectiveDispatch，不把私有证明写进 provider prompt 或公开日志。每一步验证原 live parent、Root 原财务合同、原 C、UUID worker/attempt、confirmed draft/hash/revision、原 consumed mailbox 和冻结输入；不创建替代原权属或续时。read-only proof/回执 API 只读取原 received，原请求一致、原费用与实际 worker 派发缺一不可。

command 在prepare之前核全部原入口，再通过事务内 authorized prepare/start回调核同一原合同；缺 parent、run 或串改 context 零 SDK、零 grant/mailbox/费用变化。未发出由原 typed transport 记录，不把 clock错误字符串直接作为开放码。503/401未知结果仍原预留与 indeterminate，不退款、不重试；known saved received重入可零 SDK重建投影。

专用 dispatch guard 放在新建内部连接的原 start事务之前并贯穿提交，只准直接写原 specialist call/预算 entries，拒trigger附带业务写。结果投影在私有RW/no-CREATE/FKON/syncFULL连接，authorizer贯穿BEGIN到COMMIT，限定原proposal/result字段及真实通知，比较其他proposal与协作行；delivery同样私有事务核原回执、费用、六/七原通知与11类运行表副作用。它不替换调用方authorizer，也不加入caller打开事务。owned call 增加SQL保护，禁止旧REPLACE去掉proof及原Root仍存在时的直接DELETE；不是一般paid task删除方案。

历史receipt/reconciliation优先核实际owned call完整来源；有损坏owned call不回退旧decoder。不存在owned call的既有 Native历史结果仍按原严格单event格式只读/本地送达。不补原Root/C权限、不改当前Native JSON。旧 raw prepare/start/response/uncertain、validated proposal和one-shot Gateway仅测试构建可见，生产Nest入口改走有监督链。

## 验证、失败与纠正

新增14项actual SDK/边界覆盖blocked provider期间原live SDK阶段、received/late/unknown/invalid body、费用保留、原输入/worker/C篡改、触发器副作用、caller hook/事务、投影与送达失败后原费用与零SDK恢复。非法body只形成有界失败评估，不宣称执行建议；缺reported usage/未知账单不能发布成功评估。received恢复不多造 ModelRoundCompleted。

原proposal文件612行拆13项live回归到两叶，保留全部23测试名；正向夹具改真实creator/HMAC/Root/parent且保留其Drop。37项首35过/2测试语义失败：重复未知调用应返回原预算需对账错误，不能expect空成功；Reviewer余量测试需实际将已授权executor标running。纠正测试后两项通过。该余量测试是实际human SDK和本地scheduler合同，不能当实际WebExecutor执行验收。

严格Clippy两次发现已退役接口未cfg(test)及一let-and-return，纠正后通过。扩大177项170过/7失败：旧closure正向夹具缺Mode/HMAC/原财务合同，故障注入前已被正确拒绝。拆到201行新叶，真实创建和原冻结合同，3项完整入口实际2 Root+Mapper+ExternalSurface+WebExecutor共5 SDK再注入401/settlement/finalize错误；原调用错误保留、Root/directive原子回滚、未决executor保留原reservation。另3项local closure/settlement现在原Root2+Mapper1实际SDK，后续本地清理/状态断言，不冒称另有实际Reviewer SDK；bootstrap双故障在原Root创建后注入初始化失败，不放松生产。迁移中一次编译失败遗漏供旧九E2E使用的layout helper，已原字节保留；一次系统名遗漏空格导致故障落ExternalSurface，修真实系统匹配后3项通过。所有失败日志保留，不能算生产功能失败已验收或测试全过。

最终7 closure在同一次7/7通过；此前170通过项的生产代码未再改变，精确名字交集0、并集177。最后严格Clippy、导入39和exact退役1均通过。不是177单次全过、不是全量Rust或整体Master验收。原closure25/proposal23名称集合完整保留，旧closure前577行和旧layout helper字节保留。退役字面量检查未放宽。

- closure-final：exit0，wall9.84秒；`/tmp/oviraptor-human-owned-sdk-closure-final-4.log`。
- clippy-final：exit0，wall13.41秒；`/tmp/oviraptor-human-owned-sdk-clippy-final-4.log`。
- importer-final：exit0，wall36.01秒；`/tmp/oviraptor-human-owned-sdk-importer-final-4.log`。
- literal-final：exit0，wall1.27秒；`/tmp/oviraptor-human-owned-sdk-literal-final-4.log`。

集合1180 SHA `6b0b143bafcbf86f14784809f05dcdacd09b15c58700dd984005bf91345e3c89`，1153原范围外字节不变，diff --check0；原before/prior-diffs、逐批已审查patch及reviewed-merge-diffs保存在`/tmp/oviraptor-human-owned-sdk`前缀与`/tmp/oviraptor-human-owned-rebase`。新Rust叶均不超过400行，旧父文件未批量格式化。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/owned/schema.sql` | 13 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/specialist.rs` | 362 | 否 |
| `src-tauri/src/commands/multi_agent/child_transport.rs` | 349 | 否 |
| `src-tauri/src/db.rs` | 396 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals.rs` | 254 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/owned.rs` | 276 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/owned/delivery_guard.rs` | 262 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/owned/dispatch_guard.rs` | 56 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/owned/write_guard.rs` | 118 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/receipts.rs` | 222 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/reconciliation.rs` | 355 | 否 |
| `src-tauri/src/commands/agent_directive_proposals.rs` | 330 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/owned_proposal.rs` | 47 | 是 |
| `src-tauri/src/commands/agent_tests.rs` | 139 | 否 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_fixture.rs` | 57 | 是 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_sdk.rs` | 389 | 是 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_guards.rs` | 182 | 是 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_delivery.rs` | 95 | 是 |
| `src-tauri/src/commands/agent_test_fixture.rs` | 175 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/directive/proposals/scheduling.rs` | 239 | 否 |
| `src-tauri/src/commands/agent_tests_directive_proposals.rs` | 277 | 否 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_regression_1.rs` | 210 | 是 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_regression_2.rs` | 134 | 是 |
| `src-tauri/src/commands/agent_tests_human_proposal_owned_boundaries.rs` | 103 | 是 |
| `src-tauri/src/agent_runtime/model/openai_compatible.rs` | 361 | 否 |
| `src-tauri/src/commands/agent_tests_directive_closure.rs` | 590 | 否 |
| `src-tauri/src/commands/agent_tests_directive_closure_fresh.rs` | 201 | 是 |

## 未完成和边界

本批是有界单个只读评估，不是有序多动作执行、全role或智能体脑力的最终验收。prepare/start/consume阶段仅特定证明回调，不是全部SQL写入已隔离；delivery允许的六种原emitter并非逐个canonical digest核验，保守快照还可能拒绝并发无关变化。cancel后parent检查错误仍可先于transport主错误返回，未声明所有主错误链完备。非human普通expert clockremaining Err字符串出口仍待typed修复。

动态十维child grant/精确provider费用对账与显式继续、六Root触发、15角色/General ReAct/真实Broker/并发batch、唯一终态/跨进程恢复、paid任务删除、整体UI和旧九E2E仍未完成；之后才做完整门禁/安装/app打开和已授权匿名URL验收。InputParser既有自动审批拒绝保持，不重试或改名绕过；既有材料没有完整拒绝理由，不猜测补写。Goal active。
