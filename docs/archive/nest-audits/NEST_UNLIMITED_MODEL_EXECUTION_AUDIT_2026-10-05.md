# 2026-10-05 Native 完全无上限原模型执行审计

Master/Goal均未完成；本报告不是整体功能验收。

实际原creator (0,0) 先被utility拒绝；仅新建Native5冻结有限原额度占比规则后，又被原request=0准入挡住。现无有限额度不产生稀缺权重，但费用仍独立入账。每次原Web SDK按原消息/工具字节与有界输出估算预留，原worker/轮次/请求/C/模型账本绑定；有限维度保持原硬限额，缺估算、陈旧调用、未知费用或失效授权不获得新调用。原Native1—4 JSON与规则保留，不升级已有Root、不恢复Strix或旧格式正向兼容。

实际临时生产链creator→Root/Mapper→预算Root→Web SDK/Broker GET→Client→Root：14次HTTP SDK（Root4、Mapper1、原max_turns=8的Web8、Client1）、localhost GET1；原账本已知input140/output380/cached0/request14，已关闭后四维预留和未知均0。这520 tokens是脚本响应声明的费用事实，不证明真实供应商推理/美元或整体功能。混合无上限遵守有限原硬限额。Root/Web实际503保留未知请求和token义务，停止目标工作/重放，不退款换代。

SQL触发器业务逃逸先实际失败，再限定新增预留writer只直接写主库预算entries；ABORT/FAIL/IGNORE/业务写入四种故障均拒绝且全库typed rows/rowid回滚，没有后续SDK/目标。缺估算、陈旧轮次、原政策损坏及预算建议越权亦拒绝。原最大轮数未调低或放开。

扩大首轮145项130通过/15失败：九项旧Web财务/取消夹具直接scheduler创建worker，缺原付费Mapper/派发事实，生产入口在SDK前以root_budget_original_mapper_missing拒绝。临时副本精确还原本轮前1386文件SHA c75b7b30581b3f0968071fd92d1e26c3dd16a120d51b24b6fc5094dae543bf13，复测15项仍九失败、六Mapper通过，证明旧九项不是本轮新增回归。九项仍为未完成，不删测试、不ignore、不计通过；下一补实际creator/Root/原派发生产者，再核费用/取消/恢复。

第二轮146项136通过/10失败，其中新增低预算负向错误读取加入预算快照前的估算；移除错误的测试前置断言，改为直接核验实际调用的拒绝与零写/零SDK，未改生产代码。所有失败日志保留。

六Mapper用例新建Native5请求变大，22,000原上限不足保守估算+Reviewer15,000留底；正向新建23,000、动态grant7,980及已付后余额22,960，守恒/撤权/故障原要求保持。新增22,000负向核验包含原预算快照的实际Root准入拒绝、全库零写/零SDK；不是扩大已有任务预算或降低估算/留底。原版本1—4不改变。

同源码146项关联回归137通过、9项原有失败（测试235.42秒/阶段236.91秒，选择=报告146，无ignore）；新8项全部通过。严格all-features/all-targets Clippy0（13.00秒），同二进制退役50/50（5.74秒，含literal/当前Native JSON），八叶局部fmt及16文件范围diff0。关联门禁整体仍失败，2389项全量、整体UI/安装IPC/授权URL及真实供应商质量/美元对账均未验收。

16代码路径11已有/5新均保存逐文件前像、原Git差异及最终增量审查；1375原范围外源码及HEAD59be3d86保持，1391源码集合SHA 1a7d36f82158b02570f5e935c9c8176b927b93fc6a2c4472ae8133c20983c5e7。只用临时Git/SQLite/CAS/localhost SDK和隔离源码副本，无真实DB/CAS/asset、UI/安装/授权URL操作或自动提交。

原有九项失败（当前及修改前同名实测）：

- `commands::agent_tests::budget_web_executor_rechecks_root_unknown_cost_before_each_model_round`
- `commands::agent_tests::budget_web_executor_unknown_transport_holds_cost_without_hidden_retry_or_secret_leak`
- `commands::agent_tests::budget_web_model_claim_faults_never_send_and_restore_authority`
- `commands::agent_tests::budget_web_model_claim_precedes_transport_and_receipt_precedes_more_work`
- `commands::agent_tests::budget_web_model_combined_token_overrun_preserves_invoice_and_stops_work`
- `commands::agent_tests::budget_web_model_failed_receipt_reopen_never_replays_or_refunds`
- `commands::agent_tests::budget_web_model_inflight_root_and_child_cancellation_close_transport_keep_cost`
- `commands::agent_tests::budget_web_model_missing_provider_usage_keeps_estimates_unknown`
- `commands::agent_tests::budget_web_model_received_bill_missing_publication_cannot_renew_or_replay`

证据保存 `/tmp/oviraptor-unlimited-execution-*`：逐文件before/prior-diffs/reviewed-merge-diffs、baseline/code-snapshot/scope-final；red、失败修复历程、first-expanded首轮门禁、prior-source完整隔离还原与15项对照；final-build JSON精确二进制/列表/选择/完整146项结果、Clippy、退役和格式结果。保留所有失败，不拼接不同源码局部结果。

下一先补九旧夹具真实生产者；十四类剩余及优先级以Master顶部为准。实际供应商、美元、并行、整体UI/安装与授权URL尚未验收；真实清理先盘点备份，不删除asset。
