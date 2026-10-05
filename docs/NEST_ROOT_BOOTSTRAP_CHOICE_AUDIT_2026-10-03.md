# 首次 Root 付费选择与 Mapper 准入审计 · 2026-10-03

本批在原未提交代码上增量修改，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 不变。没有重置、提交、批量覆盖、真实数据库/CAS/资产、安装或授权URL操作。Goal active，Master未完成。

## 问题与数据作用域

首次Root已经真实付费生成决策，prepare仍无条件签发Mapper；模型暂缓并不能停止原8k/1模型请求子额度。首真实负向测试0/1，第一次空建议后原路径仍调用Root/Mapper/Root3SDK并成功；循环第二种未提供建议及后续拒绝断言在此红测未达到。只允许new before-INSERT私有Multi Mode的bootstrapDispatch合同接受新语义；同金融OriginalOwner绑定。None/当前Native原合同不升级、不补授权、不改变计划JSON。

新增4项准入证明覆盖：真实paidRoot后只读grant的原event/summary/basis绑定；精确重放0SDK、全部rowid保持；三种SQLite触发器（业务旁路、改原Root状态、IGNORE启动）整子事务回滚；付费event/frozen file变化拒child；typed canonical Mode增删字段与原金融owner冲突且0SDK。grant测试只实际1Root SDK，不声称Mapper已执行。原普通prepare正向另有Root/Mapper/Root3SDK。

## 实现

- 最终Root明确选择dispatch:spa_api_mapper才签发；local中间步骤或空/别的建议返回root_bootstrap_did_not_select_mapper。相关Web后续root_decision_did_not_select_step均映射为Incomplete/coordinator_deferred，不当作工具失败。
- 同IMMEDIATE内检查原C/owner、冻结文件、原basis、整条paid tick/local parent/publication和原canonical emitter；签发/预留/启动后再次核同一证明。独立连接authorizer只开放原派发表及明确启动列；不允许触发器额外写业务。
- 子任务带Rust确定的initial_frozen_evidence准入记录；目标grant0、model8k/1、targetEvidenceProven=false。该固定初始角色门禁不是所有提案动态utility或智能体能力验收。
- 原共用prepare_readonly_child保持同一实现，仅checked入口增加前/后callback；原None继续supervised恢复。新Some目前拒绝过期worker，没有自动替换旧付费授权。
- 准备入口在C获取/业务Root激活前验证实际冻结frontend evidence；真实不一致输入在0新SDK/完整表不变边界停止。

## 实际回归与故障

生产初修3/3（3.15秒）后追加暂缓outcome分类0/1（0.56秒），再两代码行映射。首次扩大57/61（62.15秒）四失败完整保存：三项晚派发恶意trigger原安装在首Mapper之前，SQLite即使WHEN=false仍授权trigger body，被新首次准入更早拒绝；改为实际Mapper返回后的Root SDK时安装，原3SDK/2pub/0Webchild/原费用断言全部保留。第四项allow=false同时拒首Mapper，改只拒实际changed Mapper事实，保留原第二Root paid defer/无Web child。后相关61/61（62.18秒）。

扩大共用恢复159/165（266.43秒）六失败：旧直接Root没有原Mode和财务凭证，observer超时、部分0SDK、原输入断言未到。六项改existing genuine HMAC/普通creator fixture；Root SDK由真实endpoint独立处理，child count和原费用断言保留。9/11（14.42秒）剩过期C由stale_coordinator_fencing_token早拒、输入由root_tick_original_evidence_conflict早拒。原无授权replacement Root仍保留，用完整表快照证其Mode missing零写拒绝，没有给其补新合同。observer首等待失败先release/join，正式重复调用比较全表不变。

追加输入冲突负测用原paidMapper交付失败，改变context输入后安装临时项目副作用trigger；0/1（0.69秒）显示返回原冻结冲突但projects已变。生产仅前移一行原校验，随后11/11（13.80秒）。首次红测没有达到后续完整未决费用断言，green已达到。真实Root/Mapper/Root3SDK和原2pub分开计数，fence失效前后全部SDK count不增加。

严格Clippy两次实际失败日志保留：首次match-like-matches，仅改新增匹配为matches；恢复夹具六处不必要空String改&str，未禁lint。最后脚本替换日志标签误将single_finally筛选写成single_sealedly，实际139/139，不能称165；独立补跑原26/26，逐用例名不重叠并集精确等于原165集合，不隐藏或忽略失败。

## 最后门禁与快照

- clippy-sealed：exit0，wall6.88秒；`/tmp/oviraptor-root-bootstrap-choice-clippy-sealed.log`。
- affected-sealed：exit0，wall242.82秒；`/tmp/oviraptor-root-bootstrap-choice-affected-sealed.log`。
- importer-sealed：exit0，wall40.4秒；`/tmp/oviraptor-root-bootstrap-choice-importer-sealed.log`。
- literal-sealed：exit0，wall1.23秒；`/tmp/oviraptor-root-bootstrap-choice-literal-sealed.log`。
- single-finally-followup：exit0，wall21.31秒；`/tmp/oviraptor-root-bootstrap-choice-single-finally-followup.log`。

19路径14已有/5新；1151路径SHA `324df7fb8024e50967269fd8cb598207e4c4f09ff60c68eb2ce71681b34a8331`，1146原基线外1132保持。`/tmp/oviraptor-root-bootstrap-choice-baseline/before/prior-diffs/reviewed-merge-diffs/scope-final/code-snapshot`保存原文、逐文件差异和最终摘要。新5Rust叶单独fmt，既有原格式债保留；git diff --check通过。首bootstrap tests patch e4b1f907；production be2c3d8d；proof0b24f62b；followthroughc22660c6；recovery fixture6e83ab5b，全部应用前完整审閱/SHA/check。原未应用draft保留。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 122 | 否 |
| `src-tauri/src/commands/agent_tests_root_bootstrap_choice.rs` | 72 | 是 |
| `src-tauri/src/agent_runtime/web_mode/root.rs` | 220 | 否 |
| `src-tauri/src/agent_runtime/web_mode/root/bootstrap.rs` | 30 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root.rs` | 289 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_basis.rs` | 45 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_common.rs` | 160 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick.rs` | 304 | 否 |
| `src-tauri/src/commands/agent_native.rs` | 33 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/bootstrap.rs` | 169 | 否 |
| `src-tauri/src/commands/multi_agent/prepare.rs` | 328 | 否 |
| `src-tauri/src/commands/agent_native/coordinator_bootstrap_dispatch.rs` | 112 | 是 |
| `src-tauri/src/commands/agent_tests_changed_fact_tick.rs` | 298 | 否 |
| `src-tauri/src/commands/agent_tests_fresh_multi_production.rs` | 118 | 否 |
| `src-tauri/src/commands/agent_tests_tick_prepare.rs` | 85 | 否 |
| `src-tauri/src/commands/agent_tests_root_bootstrap_proof.rs` | 185 | 是 |
| `src-tauri/src/commands/agent_tests_root_bootstrap_boundary_fixture.rs` | 28 | 是 |
| `src-tauri/src/commands/multi_agent_external_surface.rs` | 379 | 否 |
| `src-tauri/src/commands/agent_tests_specialist_recovery.rs` | 347 | 否 |

## 未完成与风险

预算本地工具仍只提供原硬上限，live余额快照/动态grant/精确对账另批；六触发只有部分真实接线，全角色/Broker/真实并发、Source/Single显式恢复、canonical最终quiescence/重启、有序人工/用户聊天、实际Root工具显示与整体UI、旧九项E2E真入口、正常已付任务删除仍未完成。新bootstrap expired worker不能沿用旧supervised替换绕过原PaidRoot准入。localhost SDK脚本与临时数据库不代表真实配置模型脑力、安装或授权URL验收。

InputParser先前自动审批拒绝不绕过，当前摘要没有其具体理由，不声称交付。继续框架开发，不标Master完成。
