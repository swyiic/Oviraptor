# Root 原账本逐轮预算观察审计 · 2026-10-03

HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`保持；增量17路径，保护原dirty文件。没有真实数据库/CAS/资产/安装/URL/提交操作；Master未完成。

## 问题与作用域

原Root本地预算工具仅返回hard ceiling，真实SDK看不到已付Root/child余额与未决。首2SDK测试0/1（0.93秒）在remainingModelRequests断言发现null，后续断言未达到。只对新before-INSERT私有Multi声明绑定liveBudgetObservation；原None/Single/Source不升级，金融OriginalOwner同时冻结该字段，Native plan_json逐字节保持。

## 实现与原凭据

同一次IMMEDIATE原claim中读取完整十维hard/consumed/reserved/indeterminate/remaining及gross model token/request和原wall deadline；null不限额维保持null。金融Fine/coarse先校验，checked算术拒负数/溢出。实际SDK只追加budgetSnapshot，不把内部localParents physical proofs塞进模型message。96KiB上下文/估价在追加后再次检查，原Reviewer余量及3轮/4调用上限保持。

逻辑basis key在原creation合同存在时去掉budgetSnapshot，避免余额变化把相同paid事实变新请求；完整original request hash包含全部快照并决定callID。保存快照重放先canonical/exact基础再由Tick核完整原请求/费用/物理行，0新SDK、0修复写入。未知费用不通过新观察消失。新budget工具描述明确冻结观察/无grant；原None工具合同原样。

新增4项真实localhost SDK：逐轮20→19请求/60000→59980token和十维余额；原finance字段增删零SDK拒绝；已付request快照篡改拒重发，公开诊断精确native_sdk_log_original_binding_changed且全表不变；真实Mapper费用后第三SDK看到18请求/59960token及消耗2，0目标HTTP。没有真实配置LLM脑力证明。

## 失败与实际回归

生产主修后17/17（18.78秒），边界后63/63（60.48秒）。首次严格Clippy cmp_owned失败（10.78秒），只先保存canonical字符串比较；无lint禁用。扩大162/169（247.75秒）六日志真实SDK后公开rows空：旧diagnostic用完整basis哈希，却新逻辑key已移除预算快照。改用Tick::basis_hash，另核basis原observation合同一致，仍核canonical、完整request hash、原owner与callID，未弱化日志测试。其余一项静态旧预算tool description期待改实际新描述，名称/参数/另外三个完整工具合同保持精确，actual wire仍等于原保存request。

诊断修首次10/11（10.23秒）：六实际日志及新增篡改拒绝都通过，唯一夹具重新prepare使用已切到child的context返回root_tick_context_conflict；撤该无必要prepare，直接比较实际wire合同。保留7实际SDK/原4Root费用/0重放/all rowid。最终下面门禁均通过。

## 最后门禁与快照

- clippy-completed：exit0，wall13.12秒；`/tmp/oviraptor-root-live-budget-clippy-completed.log`。
- affected-completed：exit0，wall280.38秒；`/tmp/oviraptor-root-live-budget-affected-completed.log`。
- importer-completed：exit0，wall40.29秒；`/tmp/oviraptor-root-live-budget-importer-completed.log`。
- literal-completed：exit0，wall1.36秒；`/tmp/oviraptor-root-live-budget-literal-completed.log`。

1155代码集合SHA `6356580467ac84cc2c26da3b4e46a6ce7faf45d6b582a1b99b6aff40d3014364`；1138原范围外不变，HEAD不变，逐文件before/prior/reviewed-merge-diffs及scope-final在`/tmp/oviraptor-root-live-budget`前缀保存。新增4叶独立fmt，既有格式保持；diff --check通过。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests_root_live_budget.rs` | 103 | 是 |
| `src-tauri/src/commands/agent_tests.rs` | 124 | 否 |
| `src-tauri/src/agent_runtime/web_mode/root.rs` | 234 | 否 |
| `src-tauri/src/agent_runtime/web_mode/root/observation.rs` | 28 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root.rs` | 290 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_basis.rs` | 46 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick.rs` | 305 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/round.rs` | 39 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/public_projection.rs` | 133 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/observation.rs` | 86 | 是 |
| `src-tauri/src/commands/agent_native/coordinator_tick_once.rs` | 158 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_bootstrap_dispatch.rs` | 113 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/local.rs` | 172 | 否 |
| `src-tauri/src/commands/agent_tests_root_live_budget_boundary.rs` | 174 | 是 |
| `src-tauri/src/commands/agent_native/coordinator_common.rs` | 166 | 否 |
| `src-tauri/src/agent_runtime/model/diagnostics/owner_coordinator.rs` | 100 | 否 |
| `src-tauri/src/commands/agent_tests_paid_proposal_feedback.rs` | 324 | 否 |

## 未完成与风险

快照不是可执行grant、动态额度分配、精确provider对账或显式续跑。固定child预算/new-bootstrap expired worker拒绝保持；六触发/全角色/Broker/真实并发、最终quiescence/重启/正常paid删除、有序人工聊天、Root工具显示与整体UI、旧九E2E迁入口、完整门禁/安装态/授权URL未完成。

InputParser先前自动审批拒绝保持，当前摘要无具体理由，不绕过、不声称交付。Goal active，继续开发。
