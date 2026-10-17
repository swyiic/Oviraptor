# 原 Root 异常最终耗时与停止终态审计（2026-10-03）

状态：本批相关门禁完成；全预算、暂停恢复、Master和最终功能验收未完成。

## 原问题和作用域

已有 FinalClock 普通 sample 在原 elapsed>=hard 或原 C 已失效时拒绝；正常停止无法保存完整异常 elapsed，实际 finalize_agent_target 的完成→Limited fallback 也再次失败成为 persistence_failure。需要分别保存财务事实和消费该事实停止原任务，不能用财务事实授权继续。

本批仅临时 SQLite，实际 finisher/finalization/RootModelCall 财务接口；无 SDK/目标/浏览器 I/O，晚到成本是明确 typed billing fixture，不是真模型费用验收。原 Native Multi Root/control/C/十限额/createdAt/clock/费用UUID/rowid/JSON全部保留，无真实库、CAS、资产、安装、授权URL、重置或提交。

## A 原财务事实

独立 agent_root_elapsed_facts 保存原owner UUID/C、scan/attempt/target、原origin、首次cutoff、原hard、完整实际elapsed、原journaled和unsettled差额、binding/wall物理证明；原ledger不插clamp或越hard费用、不扩限额。只有存在的原Root control才能保存，原C失效也仅财务身份匹配；live successor不能收养旧owner。旧缺owner/legacy worker不回填，本批未补完整旧worker异常财务证明。

private IMMEDIATE全程authorizer只许直插该新表，trigger写业务/其他Root/原费用/事实行一律拒绝；IGNORE用changed=1及exact readback复核。no-update/no-delete及primary/root两唯一键BEFORE INSERT防recursive OFF的REPLACE。首次cutoff冻结，已terminal只用原finished_at，replay全行不变；无FK财务行可在任务删除后存续，不能复活任务。fact停止fresh work，原晚到费用仍可结算且不可使用输出。

## B 停止终态消费

原FinalClock只读验证存在的原fact，纯verifier显式create_fact=false，publication writer不会补fact。sample不追加wall费用，capture使用原fact.cutoff；seal/verify_closed同cutoff/fullactualelapsed与原journaled、完整物理费用/来源一致。Cancelled/Failed/Incomplete/Limited/ResumeIncompatible可停止原任务；完成/有界完成仍由原admission拒绝，并通过既有真实finalization转Limited。失效/接管C不能publication，unknown模型reservation不释放。合法财务fact先提交，之后publication失败仍保留；业务结果和终态写入保持原事务rollback。

Source仅在已有正常closure outer入口记录异常fact，并保留原完整phases/material/runtime/gate和末尾FinalClock证明；金融Source测试使用实际共享finisher/append_event/Source scoped writer，明确不是完整Source专家/gate执行或全outcome finally。当前fresh Source producer相关回归单列。

## 实际红绿和未观察边界

A接口/schema先加空shape，未接producer，真实编译46.30秒后1/7（runtime2.86秒）：6项在缺原fact首断言失败，包括自然1100ms超1000hard、失效/接管C、已关闭重放、REPLACE前提和晚到成本前提；原rollback一项先绿，不能声称red已进入所有trigger分支。实现后A及原elapsed23/23（8.70秒）。

B真实编译34.35秒后3/7（20.57秒）：4项暴露noncompletion关闭、完成真实Limited fallback、Source金融closure和已关闭late-cost前提仍budget_wall_time_exhausted。三个原拒绝/rollback项先绿。实现后两组和原elapsed30/30（35.98秒），包括完整多outcome/trigger/all旧行/physical/重放/late-cost断言。

A的临时“取消finisher应Err”在B实现后改为调用实际financial producer隔离A；B新实际finisher合同证明取消真实成功，不移除C/no-grant/rollback负测。两处新short/late纯金融夹具使用已有cfg(test)显式financial issuer，原live initialize拒绝负向保留；没有生产fallback或历史收编。A00窄rebase只加clock模块，原clock格式/排序保留；两原封存测试patch及root派生SHA独立留存。一次/tmp生成脚本断言发现A原initialize出现两次，只替第一金融setup，第二拒绝负向保持。

## 最后门禁及保护

严格alltargets/allfeatures Clippy0（wall24.70秒）；相关83/83（runtime93.51秒/wall93.90秒）；导入39/39（runtime2.71秒/wall40.45秒）；exact退役登记1/1（wall1.23秒）。首扩大脚本误写source_born_finance前缀，未选择该四项；其余83真实通过。随后纠正过滤单独真实fresh producer4/4（runtime6.35秒），同一代码快照，不能把0项过滤当验证。Cargo全部串行nice15/offline/locked/-j1/testthreads1。集合有重叠，不相加作全量Rust/功能验收。

17代码路径8已有/9新，8新Rust叶及拥有FinalClock共9fmt/check0，最大361行；无全仓格式化。1064精确收集路径SHAc5cc6d313b8a210ab41c65368503034c087510713c3f0963713cf05fdca5e433，1047范围外原路径不变，HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b和diff check通过。collection边界同前批，非docs/build.rs/icons/capabilities/dist/安装包全仓保证。完整before、已有dirty差异、逐文件增量与快照在/tmp/oviraptor-root-exceptional-before.json、-prior-diffs、-reviewed-merge-diffs、-scope-final.json、-code-snapshot.json，实际红绿/最后门禁同前缀。

## 未完成和风险

unsettled是明确未结清财务差额，不是ledgerConsumed/退款/对账/可用额度；相关UI/人工reconcile/dynamic尚未接。Single outer finally及原终态/暂停publisher、真正sameRoot继续、旧worker immutable证明、Source全outcome finally仍缺；脑子完整local ReAct/所有触发/全专家/Broker/实际并发、有序人工执行和SDK/Web全路日志继续开发。最后全量、当前安装启动与授权URL尚未开始。InputParser先前自动审批拒绝仍未交付、不绕过。Goal active，不标Master完成。
