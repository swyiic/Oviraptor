# Source Broker 实际原入口回归审计（2026-10-04）

本批解决六项旧无原始Source子任务权限的回归夹具，生产代码/预算准入/Native JSON/allowlist零变化；不是新增整套Source能力或删除功能验收。变更前scope九项3过/6失败，六项均tool_identity_binding_denied（临时SQLite）。已有未提交585行tests_source_broker_scope.rs的Git diff为空是因为该文件本来未跟踪，已保存完整文件前像，不以空Git diff作为无改动证据。

原测试创建普通Coordinator/旧测试context后直接调用源码工具；新六项必须经过实际签名工作台发布、冻结snapshot/view/results/current NativeSourcePlan、prepare_native_source_coordinator新生原Root及原财务凭证，再run_native_source_assessments/WorkerSupervisor/独立角色模型SDK/SourceRound/SourceBroker/回执/Reviewer/coverage/final exit。没有手造Root/补grant/扩C期限/零费运行。分析器execute是测试回调而非实际Semgrep/CodeQL，SDK是localhost脚本而非真实推理模型。

| 原具名测试 | 当前覆盖 |
|---|---|
| source_broker_scope_real_entry_never_reads_unselected_source | diff只暴露app.py；untouched读/候选拒绝，搜索不泄漏，候选0；实际SDK7 |
| source_broker_scope_full_and_auto_keep_authorized_files_available | full/auto失效base/full回退及auto有效base/diff保持原选择；实际SDK每例7；SDK能力精确匹配Source角色原合同，禁止replay_http/callgraph；移除旧17项Source/Web混合正向预期 |
| source_broker_scope_exact_paths_and_selected_dependency_manifests | .env.example、前导空格/中文/换行的准确路径；五非法路径拒绝，选中package.json可读、未选Cargo.toml不可读，原manifest/language断言保持；实际SDK7 |
| source_broker_scope_candidate_write_cannot_commit_after_stop_or_identity_change | 插入候选后scan paused→agent_attempt_not_active；cancel→coordinator_not_executable；foreign Root→assignment_attempt_run_binding:Query returned no rows；attempt paused→source_assignment_scope_unavailable:Query returned no rows；候选0、view1、scan和全部run撤权变化回滚，SDK5且无下一阶段 |
| source_broker_scope_candidate_write_rolls_back_if_view_receipt_is_lost | 部分写入/回执丢失由实际Broker保存source_analysis_integrity拒绝回执，候选0/view1，后续正常独立coverage闭合，SDK7 |
| source_broker_scope_candidates_bind_real_locations_and_distinct_content | 四非法位置拒绝；两真实文件不同id；同候选跨轮同id；原内容hash/manifest/attempt/scan/candidate状态绑定保持，候选2、实际独立Reviewer，SDK9 |

写入后撤权迁移从旧facade的source_authorization_changed改为实际SourceRound/assignment/Rust边界准确错误，先用原入口nocapture取得四精确错误再收紧断言，不用任意Err代替负向。实际Root保留每次已付SDK及10输入/10输出token；target_requests/browser_actions/controlled_writes/upload_bytes消费0、不确定0，成功无预留，失败不虚构释放；三个原预算合同/硬限额/时钟来源表typed rows/rowid始终相同，原coordinator身份及FinalClock原退出有效。没有宣称十维动态预算已经完成。

每个成功实际Root闭合后再调用同入口，调用拒绝、原全应用typed rows/rowid完全相同、SDK不追加；不是完整进程重启验收。工具结果从原SourceAnalyst已完成receipt按原Root/round/index读取，不直接调用facade给正向授权。initial/RepoMapper工具/SourceAnalyst工具/独立review及coverage分别真实模型调用；没有候选时不伪造candidate Reviewer。

三个原scope拒绝测试（deleted_receipt、stopped_replaced、unavailable_diff）逻辑完整移动到历史叶，尚无实际SDK生产者，仍需迁移；不是新入口全负向覆盖。Source结果15项函数及历史context前像完整保持，仅文件末尾空白裁剪；其本批核验8过/7失败，未ignore、改断言或修权限。失败具名如下：

- source_result_same_bytes_keep_both_engines_without_duplicate_candidates
- source_result_distinct_reports_keep_every_merged_candidate_origin
- source_result_receipt_late_history_cannot_replace_actual_attempt_results
- source_result_receipt_missing_or_changed_never_falls_back_to_history
- source_result_receipt_new_attempt_owns_results_even_when_revisions_are_reused
- source_result_receipt_out_of_view_locations_are_gaps_not_active_claims
- source_result_receipt_keeps_failed_analyzers_visible

失败集中在旧无原子任务权限context调用，后三类JSON空值/工具拒绝不代表原归属业务断言已验证。下一步必须经有原子任务授权的实际入口保留多引擎来源/唯一候选/材料缺失/历史不能覆盖/轮次原归属/分析器失败缺口，并与原费用/退出证明一起核验；若实际入口暴露生产问题，再先复现后最小修改。不能把这七项视为项目仅剩七项。

最终具名关联24/24，选择集合=报告集合=通过集合，无ignored；测试151.59秒、含编译阶段167.62秒。严格offline/locked/all-features/all-targets Clippy -D warnings退出0（10.21秒）；退役46/46（5.41秒），含literal exact及当前Native JSON原字节回环。三个新执行叶局部rustfmt检查0，scope diff--check0。变更前scope3/6和首轮迁移scope9/9（85.73秒）、四撤权exact1/1（8.69秒）、未迁结果组8/7（8.52秒）均留日志；一次误用短名配--exact筛到0项的日志不算测试证明。前批原14独立14/14属前批快照，本批不重复报当前原14运行。未全量Rust/UI/build/fmt/安装/外测。

6代码路径职责与体量：原scope聚合585→387行；历史context20行；历史拒绝58行；实际producer/财务/回执/重放夹具192行；read/dependency叶118行；候选/事务叶174行，均≤400。全部24原具名测试保持，六迁移对应关系如上；没有删除独有失败回归。6最终增量全文审查，1329原范围外SHA、HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b保持；1335源码集合SHA8fe4fec11d331b32ff0f97b7faab9be66c1fc0ecd35337e2cbe1372c3f0181fa。本批前像及已有Git diff逐文件保存，文档追加前备份且历史全文后缀原字节保持。不reset/批量覆盖/自动提交。

证据：/tmp/oviraptor-source-broker-original-{baseline.json,before.json,prior-diffs/,initial.patch,generate.py,baseline.log,first.log,revoke.log,revoke-exact.log,results-audit.log,scope-final.json,code-snapshot.json,reviewed-merge-diffs/,semantic-review.json,selected.json,checks.py,checks.json,related.log,clippy.log,retirement.log,docs-final/,docs-proof.json}。

仅临时Git/SQLite/CAS/localhost，未写真实数据库或CAS/资产，未安装/访问两URL。真实Nest写入/CAS已授权，但真实清理须先精确盘点备份，资产不得删除。Master全部14类剩余见最新顶部；Goal active。候选/来源回归、删除/残余路径、动态预算/全通道恢复、六监督/全角色/真正并行、聊天/逐路日志/整体UI、非Web及知识资产仍未全部完成；最后才完整门禁、同源安装App打开、两授权URL匿名只读和真实模型质量。自动化、只读诊断和脚本SDK都不替代整体验收。
