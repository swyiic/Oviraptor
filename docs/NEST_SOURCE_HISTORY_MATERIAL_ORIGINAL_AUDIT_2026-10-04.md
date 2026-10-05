# Source历史隔离与材料失效原入口核验（2026-10-04）

本批完成两项Source结果回归的真实入口迁移，不改生产权限/预算/Native JSON/allowlist。迟到历史在首个原工具回执完成后、第二轮实际SourceAnalyst SDK前导入；原结果/摘要/来源不变，历史key精确result_not_found，独立Reviewer及coverage完成，脚本SDK9次。材料六类故障分别通过list/get两个实际工具，12场景属一个具名测试；实际SDK5次已付后，在原工具交付事务内破坏材料，精确拒绝并回滚结果输出及材料破坏，原工具仍planned，原材料typed rows/rowid保持。随后同一临时库持久化故障，冷分析准入拒绝、全部typed rows/rowid不变、分析器不得重跑，原费用/退出保持。

最终同源码关联23/23（204.39秒测试/204.84秒阶段，选择=报告=通过集合）、严格all-targets Clippy0（12.35秒阶段）、退役46/46（5.51秒，含exact及当前Native JSON原字节回环）、三执行叶局部fmt/diff--check0。首次迟到历史1/1（编译52.93秒/测试27.48秒）、材料失效1/1（编译47.90秒/测试26.93秒）；12场景不报成12个具名测试。Source结果全15在同源码分组核验14过/1失败；剩新attempt归属exact0/1（0.83秒，旧context权限拒绝），未ignore或放宽断言。另三scope历史拒绝夹具和Master其余任务仍未完成。

4代码路径2已有/2新逐文件保护、四最终增量全文审查，其他Source结果函数/helper原字节及全部15测试名保持；1334原范围外源码SHA和HEAD59be3d86不变，1338源码集合SHA b1d50d272e5d469a0cbd90b7c28295e8c5350e6a26b09f36f256938a421072ca。仅临时SQLite/CAS/localhost、分析器回调和脚本SDK；没有写真实DB/CAS/资产、提交、安装或外测，也没有本批UI修改。脚本SDK不能证明真实模型脑力或整体功能验收。

## 精确故障合同

| 临时材料故障 | 实际执行及冷分析拒绝 |
|---|---|
| 缺失结果回执 | analysis_result_receipt_unavailable |
| 回执摘要、analysis digest、attemptNumber变化 | analysis_result_receipt_binding_changed |
| records损坏 | analysis_result_receipt_invalid:missing field `bundleId`（只不固定serde位置） |
| 导入信封变化 | accepted_result_envelope_changed |

六类分别经analyzer.list_results和analyzer.get_result执行。历史SARIF实际导入且key不同于本attempt原结果。AFTER UPDATE临时fault只在原Source工具planned→completed时触发；不修改真实库或生产授权，未借closed Root通用拒绝代替材料失效检查。交付后的生产authorize/check拦住提交，SDK已付账单与原退出证明独立保留。冷分析用panic回调保证无重新分析，拒绝前后对比数据库每张表typed rows与rowid；不能重建丢失的接受证据。

迟到历史测试在同一实际SourceAnalyst执行中先读取后导入、再读取；使用真正SourceRound回执对照原candidate/revision及analysisResultsDigest，并核对冻结AnalysisResults序列化和digest不变。历史详情拒绝、实际详情可读、真实engine筛选不受历史影响，SourceClaims原值保持。原Root成功闭合后重复执行无额外SDK，全部typed rows/rowid保持。

## 修改及证据

- tests_source_broker_scope.rs：只替换上述两项旧无原子任务授权的context测试体，其他函数/helper原字节保持，全部测试名保留。
- tests_source_broker_original_fixture.rs：测试SDK增加可选回合前hook；既有调用接口/默认脚本/签名发布/原Root/费用/退出/闭合重放检查保持。hook不授予权限或生成原结果回执。
- tests_source_result_original_history.rs：迟到历史实际双轮读取及原结果隔离。
- tests_source_result_original_material_failures.rs：六类材料×两工具的实际交付回滚和冷分析禁止再生。

范围保护：/tmp/oviraptor-source-history-original-baseline.json、before.json、prior-diffs/、initial.patch、material.patch、reviewed-merge-diffs/、scope-final.json、code-snapshot.json、semantic-review.json（同一前缀）。原范围外1334文件及HEAD保持，所有修改代码文件小于400行。

验证日志：/tmp/oviraptor-source-history-original-first.log、material-first.log、related.log、clippy.log、retirement.log、remaining.log；具名选择和核验汇总见selected.json、checks.json。首次以及尚未修复的失败日志完整保留。全部Cargo离线/锁定/单作业串行，未将先前批次结果拼成此批全绿。

## 未完成及风险

继续新attempt实际发布/原结果归属、三历史拒绝夹具及删除/残余活路径，再依完整14类清单推进动态预算/对账/恢复、六监督/全角色/真正并行、用户聊天/逐路日志/整体UI、非Web及知识资产生命周期。框架完成后才完整门禁、同源安装App打开、两授权URL匿名只读及真实模型质量；登录凭据待用户提供。真实Nest写入/CAS已授权，真实清理先精确盘点备份，不删除asset；当前Native JSON保持。InputParser原拒绝不绕过。Master未完成，Goal active。

本批只完成两项回归迁移和这些边界的自动化取证。没有新增动态预算分配、全角色监督或真实并行实现，没有完成聊天/UI/实时日志、真实模型质量、安装App打开及授权URL验收；数据库/CAS真实规模、历史清理、其他Source删除/人工/失败合同仍待完成。模型和分析器均为明确测试回调，不代表模型理解或外部分析器质量。
