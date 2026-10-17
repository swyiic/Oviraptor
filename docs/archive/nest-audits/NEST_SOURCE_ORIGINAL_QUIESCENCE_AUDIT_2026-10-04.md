# Source原调用退出与暂停审计（2026-10-04）

本批完成Source暂停生产缺口及原调用证明负向。Master全部框架仍未完成，Goal active；本审计不替代完整功能/安装/真实模型验收。

## 问题与数据作用域

实际工作台原发布和真实`launch_native_source_pipeline`启动Source分析，缺少固定分析器能力保持真实覆盖缺口，实际localhost SDK已到达。生产Source外层持有branch/source，内部持有source-model/source；没有Web target/Source-key调用。原暂停核验仅凭财务Root策略multi要求Web原target证明，线程退出后仍停pausing。首个原入口红例准确失败`scan_quiescence_original_target_exit_missing`，不是旧夹具伪造权限或模拟界面。红例日志 `/tmp/oviraptor-source-quiescence-launcher-red.log`。工具外壳以tail返回0的退出码不作为Cargo成功依据；原Cargo日志明确0过/1失败。

本批只改：

- `src-tauri/src/commands/scan_quiescence.rs`：原金融控制列出的历史attempt/target参与核验；RootOwner纯读核对原UUID/control/plan/scope/limits/clock，不续C或授予权限；Source冻结阶段及零target/host动作合同与Web原mode互斥，等待原Source模型inode；Web仍probe既存原target inode，缺失拒绝。
- `src-tauri/src/commands/tests.rs`：新增测试登记，原登记保持。
- `src-tauri/src/commands/tests_source_quiescence_original.rs`：真实外层启动器/SDK/暂停/费用，以及精确原证明缺失、6种plan/scope篡改、未出生及旧attempt锁负向。

没有修改Source费用/权限writer、模型调度器或Web准入。已付/未知费用、原Source计划保留；暂停不伪造Root/分支完成、mailbox结果、退款、续跑或额外SDK。重复收尾、拒绝和late response保持全部原行/rowid。锁证明仅说明本地调用者退出，不能推导外部请求效果回滚。

## 最终核验

| 范围 | 结果 |
|---|---|
| 直接关联暂停/Source财务退出/原证明 | 精确23/23；阶段162.53秒，实际75.51秒；选择集合=通过集合。 |
| 严格Clippy | all-features/all-targets，-D warnings，退出0，17.20秒。 |
| 退役 | 46/46；阶段5.34秒，实际4.95秒；含exact、当前Native JSON原字节roundtrip。 |
| 原14具名债 | 精确9过/5失败；阶段31.72秒；无ignore。 |
| 扩大325 | 主动SIGINT保留部分结果：91过/6失败/228未得结果，926.64秒；没有全跑/全绿，不能与其他范围加总冒充整体验收。 |
| 付费Web/Multi原删除保护 | 扩大部分结果中同一源码11项具名通过，含原target缺失不补造、原owner在用拒绝、审计/锚点/费用/commit故障。未另称11项是全量删除验收。 |
| 6项Source Broker基线 | 隔离1323文件SHA逐个与本批前相同；精确0过/6失败（95.44秒含编译，实际3.14秒），名称集合=本轮观察的6项失败；均提前身份授权拒绝。 |
| 格式与差异 | 新leaf及新增helper局部rustfmt通过；逐文件增量全文审查及diff--check0。 |

当前lib2314已编译，未全量运行。首3/3通过属于增加最终负向之前的中间状态，不作为最终完整集合；最终23/23才是当前直接关联证据。扩大325并未跑到全部新用例，之后在最终定向中完成它们。没有回滚/恢复旧正向兼容来让旧Broker通过。

原14的剩余5项：

- `commands::agent_tests::bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure`
- `commands::agent_tests::e2e_missing_side_stops_at_insufficient_evidence`
- `commands::agent_tests::every_target_ends_in_exactly_one_terminal_state`
- `commands::agent_tests::policy_native_coverage_only_is_ready_without_a_reviewer`
- `commands::agent_tests::policy_native_routes_without_runtime_adapter`

Source Broker既有6项：

- `commands::tests::source_broker_scope_candidate_write_cannot_commit_after_stop_or_identity_change`
- `commands::tests::source_broker_scope_candidate_write_rolls_back_if_view_receipt_is_lost`
- `commands::tests::source_broker_scope_candidates_bind_real_locations_and_distinct_content`
- `commands::tests::source_broker_scope_exact_paths_and_selected_dependency_manifests`
- `commands::tests::source_broker_scope_full_and_auto_keep_authorized_files_available`
- `commands::tests::source_broker_scope_real_entry_never_reads_unselected_source`

这些Broker夹具直接创建无真实Source子任务权限的Coordinator行，所有6项在本批前源码已被`tool_identity_binding_denied`提前拒绝。不能直接改期望为拒绝来抹掉原路径/内容/候选/撤权断言，亦不能放宽权限；下一批须迁真实Source原任务/工具调用再证明原语义。还有其他未执行或未知回归债；不是全项目只剩11项。

## 工作树、文件与证据

逐文件前像、已有Gitdiff和SHA已保护。本批3路径2已有/1新，1321原范围外文件SHA保持；1324源码集合SHA `1de26dbdbe22c381841af6bd24d4df99e3c6887b9d36685951d54f37550b3cf1`，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 保持。完整原1323文件隔离副本SHA `2bd96feb09b6801d5ff7f540549e9217233e4a83a2becbab8afbf28284882a57`；只在临时副本中恢复本批前像，未改变/重置当前工作树。Cargo顺序单作业/offline/locked/all-features、nice15运行，隔离基线复用编译缓存后已在当前工作树重新编译并完成最终门禁。

审计定位：

- `/tmp/oviraptor-source-quiescence-original-before.json`、`-baseline.json`、`-scope-final.json`、`-code-snapshot.json`、`-prior-diffs/`、`-reviewed-merge-diffs/`。
- `/tmp/oviraptor-source-quiescence-targeted-final-names.json`、`-final-checks.json`及逐项.log。
- `/tmp/oviraptor-source-quiescence-selected.json`、`-checks.json`、`-broad-interrupted.json`、`-related-final.log`（非完整325结果）。
- `/tmp/oviraptor-source-quiescence-baseline-source-proof.json`、`-baseline-broker-debt.json`及.log；隔离副本在`/tmp/oviraptor-source-quiescence-baseline-source`。
- `/tmp/oviraptor-source-quiescence-docs-before/manifest.json`与三文档完整前像；新章节前置，原全文作为精确suffix保持。

全程临时SQLite/localhost/合成会话；没有写真实DB、CAS、资产或业务记录，没有安装App、外部授权URL访问、Git自动提交/推送。当前Source分析器缺能力仍是覆盖缺口；没证明真实容器/Windows/模型推理质量。预算动态分配/对账/恢复、Root六触发/全角色/真正并行、聊天/逐路日志/UI、余下删除与活路径、完整门禁/安装/URL最后验收均按Master最新14项继续。InputParser原拒绝不得绕过。
