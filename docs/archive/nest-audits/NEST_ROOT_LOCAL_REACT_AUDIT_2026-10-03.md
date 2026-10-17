# Root 有界本地 ReAct 审计 · 2026-10-03

本批只交付新Multi Root的有界本地工具循环、原费用/恢复/最后派发证明链。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 不变；原未提交改动逐文件保留，没有真实DB、CAS、资产、安装、授权URL、提交或重置操作。Goal active、Master未完成。

## 问题与作用域

原协调Root仅一次无工具SDK，不能读取冻结快照并继续决定。首两项真实SDK红测0/2（runtime0.83秒），单次实际调用在root_tick_decision_invalid退出；后续上限断言尚未达到。最小生产循环后2/2（2.46秒）：实际工具步骤再一次最终SDK，同API已存结果重放0SDK；工具-only真实3轮保留3份账单且不发第4轮。

新增合同只由真正before-INSERT私有Mode证明冻结到新Multi Root，原金融contract同时绑定localDeliberation。Single/Source/原Native v1 None保持原JSON/一次无工具合同；没有给旧Root补控制、工具、预算或新origin。即使尚无费用，篡改Mode删除/添加合同仍与原金融owner冲突，不通过改正文或重签旧行升级。

## 已实现边界

- 实际注册snapshot.read、evidence.read、capability_budget.read、plan.propose；前三只读原捕获内容，最后仅offered step或defer且dispatched=false。没有HTTP、任意文件/SQL/命令/凭据工具。
- 每轮实际SDK有独立Original请求、claim、原invoice和semantic/publication；3轮/4调用/96KiB模型上下文固定上限，每次都核原C/provider/clock/10维。新增模型claim保留15k tokens和1 request Reviewer下限；旧None下限不变。
- 只保存经过验证的本地工具结果及Rust公共摘要，不保存私有assistant正文。未知工具、任意path、重复ID、未提供的proposal、超调用数均先保留实际原费用，再拒绝决策/后续SDK；未知usage不伪造0费用。
- 已付本地步骤和最终决策恢复均使用原receipt；publication失败不会重发已付调用。snapshot去掉internal localParents/localHistory，防止物理证明重复嵌套撑大实际上下文；没有扩大原预算上限。
- 下轮/最终请求绑定前两轮原3类tick receipts、2类model journal、原budget rows、timeline/chat/event的rowid与精确字段。在最后派发require_executable及published两处继续核同一原证明；不把父证明检查放到金融received之前，保证晚到已发生费用仍保存。

## 红测、故障与修正

边界首33/36：旧tools[]期待、未知usage中known model request=1的断言、反升级测试JSONValue改变canonical字节导致更早Mode拒绝。只纠正前提，保留费用/所有行不变与0后续SDK；反升级改typed canonical后确认Mode有效再由原金融绑定拒绝。

最后派发证明红测0/1（0.79秒）复现父local event时间变更仍可require_executable。修正后36/37，唯一tool-only第3轮因snapshot含internal proof增长而原预算拒绝；删除model snapshot中的内部proof后41/42，最后旧paid-proposal期待空tools在实际7SDK后失败。校正为真实4注册函数、原保存request与实际wire逐字段一致；expanded43/43（52.32秒）。这些集合重叠不相加。

首最终130/132（197.79秒）：duplicate multi prepare夹具没frozen evidence，等待endpoint超时；gap followup同样root_tick_evidence_missing。该frozen evidence要求文件在阶段基线hash34000ebef0378f3e07997f356df0c9bd56c074ec5bfb83c8265498f91386cd55与当前逐字节一致，未通过修改生产检查收绿。夹具现在真实创建/冻结模型和原预算；duplicate实际Root/Mapper/Root3SDK，阻塞首SDK期间独立caller全部应用行不变，先释放/join再断言。followup真实4Root+独立Mapper/Reviewer/Investigator7SDK，不再用人工sealed Reviewer或后填endpoint；本地HTTP artifact只验证来源，未声称目标请求真实发生。

扩大143/149（231.18秒）六失败保留：真实creator目标原来就有1行；项目ID为9001不能假定1；target-00001→自身rename并未实际移动目录。修正为原目标计数加全行rollback、动态来源项目、明确不同target-00002，保留body same-length替换、symlink/marker/retarget拒绝。剩一项旧测试直接删除已付SDK来源，与Root tick RESTRICT/no-delete审计合同冲突；保留新草案删除的永久回执/不得重建，来源改断言原running Root及已付证据不被直接DELETE或生产delete API删除，全部应用行不变。此处测试名从source_deletion_is_not_blocked改为paid_source_remains_protected；不宣称已付且完全settled任务删除问题已解决，没有改外键/immutable保护。此前直接DELETE扫描中来源从未构成合法生产删除验收。

## 最终验证

- clippy-completed：exit0，wall8.48秒；`/tmp/oviraptor-root-local-react-clippy-completed.log`。
- affected-completed：exit0，wall229.42秒；`/tmp/oviraptor-root-local-react-affected-completed.log`。
- importer-completed：exit0，wall45.03秒；`/tmp/oviraptor-root-local-react-importer-completed.log`。
- literal-completed：exit0，wall1.19秒；`/tmp/oviraptor-root-local-react-literal-completed.log`。

实际受影响149/149 runtime229.01秒，包括原132和共享followup/submission使用方；导入39/39与exact1/1，不是全Rust门禁。新10Rust叶max208及重写loop叶单独fmt，既有文件保留原格式；strict Clippy/全targets/features与差异检查通过。

29路径19已有/10新；1146代码路径SHA `4294e48bdcbe78670ef30aa755c3329210417bf53c2f3170c9a97255a74a8c97`，原1136基线外1117路径保持。`/tmp/oviraptor-root-local-react-baseline/before/prior-diffs/reviewed-merge-diffs/scope-final/code-snapshot`保存原文和合并差异；原失败日志未覆盖。关键patch：sdk-red a3fd61c7；production f979f0eb；dispatch-proof 16e84e69；local-history 3a4857c6；floor 9baf2b9b；真实fixture41e1adc1；scope20024864。每项应用前完整审阅及SHA/check。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 119 | 否 |
| `src-tauri/src/commands/agent_tests_root_local_react_sdk.rs` | 185 | 是 |
| `src-tauri/src/agent_runtime/web_mode/root.rs` | 202 | 否 |
| `src-tauri/src/agent_runtime/web_mode/root/local.rs` | 45 | 是 |
| `src-tauri/src/commands/native_web_mode_plan.rs` | 135 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root.rs` | 288 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_basis.rs` | 41 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_common.rs` | 164 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_changed_request.rs` | 118 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick.rs` | 301 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/local.rs` | 170 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/storage.rs` | 98 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/publication.rs` | 129 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model.rs` | 177 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_tick_once.rs` | 146 | 是 |
| `src-tauri/src/commands/agent_native/coordinator_tick.rs` | 110 | 否 |
| `src-tauri/src/commands/agent_native.rs` | 32 | 否 |
| `src-tauri/src/commands/agent_test_fixture.rs` | 171 | 否 |
| `src-tauri/src/commands/agent_tests_root_local_react_boundary.rs` | 208 | 是 |
| `src-tauri/src/commands/agent_tests_root_local_react_recovery.rs` | 108 | 是 |
| `src-tauri/src/commands/agent_tests_root_local_react_authority.rs` | 125 | 是 |
| `src-tauri/src/commands/agent_tests_tick.rs` | 223 | 否 |
| `src-tauri/src/commands/agent_tests_root_local_react_dispatch_proof.rs` | 46 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/history.rs` | 146 | 是 |
| `src-tauri/src/commands/agent_tests_paid_proposal_feedback.rs` | 312 | 否 |
| `src-tauri/src/commands/agent_tests_root_local_react_floor.rs` | 43 | 是 |
| `src-tauri/src/commands/agent_tests_web_mode_runtime.rs` | 223 | 否 |
| `src-tauri/src/commands/agent_tests_gap_followup.rs` | 270 | 否 |
| `src-tauri/src/commands/agent_tests_gap_submission.rs` | 163 | 否 |

## 未完成与风险

本地capability_budget只给原硬上限，未给动态remaining/grant；不能称预算决策完整。首次prepare仍无条件Mapper、六类触发和全部15角色生产工具/Broker/真实并发未完成。Root摘要消费者尚未专门展示实际本地工具名；有序人工执行、用户聊天闭环、终态quiescence/重启/对账/显式Resume、已付审计记录与正常删除关系及整体UI仍待后续。原九项旧E2E真入口迁移未完成。localhost SDK脚本不是配置中真实LLM脑力、真实安装包或两个授权URL验收。

InputParser既有自动审批拒绝不绕过，具体拒绝原因不在当前摘要，未声称交付。继续开发，不停在本批绿色。
