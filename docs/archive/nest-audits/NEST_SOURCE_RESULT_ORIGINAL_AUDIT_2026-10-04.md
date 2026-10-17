# 四项Source结果实际原入口回归审计（2026-10-04）

接六Source Broker迁移后的同字节集合1335/SHA8fe4fec11d331b32ff0f97b7faab9be66c1fc0ecd35337e2cbe1372c3f0181fa。未迁结果15此前8过/7失败，四项本批从旧普通Coordinator/no original child context调用迁到实际Source执行链。生产代码/预算准入/权限/Native JSON/allowlist零变化，未ignore、删业务或放宽授权。

复用上一批真实签名发布→冻结snapshot/view/results/current NativeSourcePlan→新生原Root/C1/预算/时钟来源→run_native_source_assessments/WorkerSupervisor→initial与独立RepoMapper/SourceAnalyst tools SDK→原SourceRound/SourceBroker/工具回执→独立candidate Reviewer/coverage→原FinalClock退出。共享测试helper只增加可配置分析器回调及分析结果生成后选择实际原key的轮次准备，不发Root/子grant或改费用。既有scope调用仍使用原API委托默认分析器，保留其六scope全部业务/负向。

| 原测试 | 保留及新增覆盖 | 实际SDK |
|---|---|---|
| source_result_same_bytes_keep_both_engines_without_duplicate_candidates | 原Semgrep→CodeQL回调顺序保持；同字节SARIF两实际文件/engine来源、同canonical revision、claim1/source2；默认/semgrep/codeql三列表唯一候选、limit1不truncated、engine选择及source2；一次原get_result与三列表逐一核验key/revisionHash/source2，独立candidate Reviewer | 8 |
| source_result_distinct_reports_keep_every_merged_candidate_origin | 两报告hash不同、canonical候选1/source2、两原artifact id及revision归属保持；原envelope未追加contributingArtifacts；claim1/source2，两engine原列表保持来源，独立candidate Reviewer | 8 |
| source_result_receipt_out_of_view_locations_are_gaps_not_active_claims | 冻结diff不暴露untouched.py；sourceClaims空、analysis_result_location_outside_view缺口保持；历史import revision仍存在、原工具receipt候选空 | 7 |
| source_result_receipt_keeps_failed_analyzers_visible | 分析器exit2的Failed输入，原工具receipt候选空、run failed、semgrep缺口可见；不把失败当干净完成 | 7 |

原同字节测试在每种engine筛选后重复调用相同key的get_result，实际SourceRound禁止同SDK重复相同工具参数。现提交一次get_result，从真实receipt与三原list receipt逐一比对key/revisionHash和两来源，保存原语义而不绕过工具去重。断言不是直接调用broker或手造已完成工具结果。

每例共同验证原model_requests等于真实localhost SDK次数、input/output各10×次数，原已付均保留，target_requests/browser_actions/controlled_writes/upload_bytes消费0且无不确定，成功预留0；三张原财务合同/硬限额/时钟来源表typed rows/rowid保持、原coordinator身份有效、FinalClock原退出有效。闭合后再次调用同Source入口拒绝，所有应用typed rows/rowid及SDK次数不变。没有宣称十维动态预算、完整重启或全部Source删除生命周期完成。

分析器执行仍为测试回调，只证明实际冻结/导入/角色/工具/费用链；SDK是脚本provider，不能证明真实Semgrep/CodeQL进程、真实模型理解/推理/纠错、用户聊天/UI或整体功能验收。没有生产Root模型脑力声明。

首次四项4/4，编译52.03秒/测试65.52秒。关联初次21/21（149.40秒测试/180.28秒阶段）后审查发现原helper的Semgrep→CodeQL调用次序需显式保留，补回此独有断言后完整重跑：最终21/21（152.50秒测试/188.23秒阶段），选择=报告=通过集合，无ignored。严格offline/locked/all-features/all-targets Clippy -D warnings退出0（11.61秒），退役46/46（5.83秒，含exact及当前Native JSON原字节回环），两执行叶局部fmt及scope diff--check0。前次结果/Clippy/退役另保留pre-order日志，不冒充最终快照。

Source结果最终同源码分组共15：关联组含12过，另三具名exact0过/3失败（见final-remaining.log），保留失败：source_result_receipt_late_history_cannot_replace_actual_attempt_results；source_result_receipt_missing_or_changed_never_falls_back_to_history；source_result_receipt_new_attempt_owns_results_even_when_revisions_are_reused。拒绝/JSON空值仍集中在旧无子授权context；其函数/helper原字节不变。该三项必须迁实际原入口：迟到历史前后实际receipt、六种失效点/原费用/零扩权/禁止重建、真实workbench retry发布的新attempt/旧Root撤权，而非插attempt行加临时权限。三旧scope拒绝夹具也尚缺实际SDK生产者。

范围3代码路径2已有/1新：scope聚合387→304行，只删除四被等价/更强原入口替换的函数并加入include，其余函数/helper字节保持；共享producer192→220行，保持原API/费用/退出/闭合重放；新结果读取叶223行。全部15原具名Source结果测试保持，结合scope9原24测试名称保持，均≤400行。三最终增量全文审查及逐文件前像/Git差异保留，1333原范围外SHA和HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b保持；1336源码集合SHA127ed29cc8e119251d721f76413fb7acc7c0b781bdd5e67c6041f9ade825f9ac。文档追加前备份，原历史全文后缀原字节保持。

证据：/tmp/oviraptor-source-result-original-{baseline.json,before.json,prior-diffs/,initial.patch,draft/,removed.json,first.log,selected.json,checks.py,pre-order-related.log,pre-order-clippy.log,pre-order-retirement.log,pre-order-checks.json,related.log,clippy.log,retirement.log,checks.json,remaining.log,final-remaining.log,scope-final.json,code-snapshot.json,reviewed-merge-diffs/,semantic-review.json,docs-final/,docs-proof.json}；15项原8/7日志仍在/tmp/oviraptor-source-broker-original-results-audit.log。

仅临时Git/SQLite/CAS/localhost，不写真实DB/CAS/asset，不reset/批量覆盖/提交/安装/外测。Master14类剩余全部见最新顶部，Goal active。继续三结果、三历史scope拒绝及其余删除/残余活路径，然后动态预算/恢复、六监督/全角色/并行、用户聊天/逐路日志/整体UI、非Web及知识资产；最后全量门禁、同源安装App打开、两授权URL匿名只读、真实模型质量。自动化或只读诊断不作为整体验收。
