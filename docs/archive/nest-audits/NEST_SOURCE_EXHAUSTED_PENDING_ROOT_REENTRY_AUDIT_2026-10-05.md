# Source 已知失败 pending Root 的原事实恢复（2026-10-05）

Master 框架未完成，Goal active。本批继续 REM-A01/A07 原执行义务，范围为已知三轮耗尽且子任务已结清、Root尚未终态、当前原C仍有效的恢复。所有生产者使用签名发布、真实Source入口和本地脚本SDK，不能作为真实模型质量或全框架验收。

## 先证明问题和数据作用域

实际Source工具恢复入口运行至耗尽，原子任务failed且费用已结清。测试在Root写入前持有实际原SDK inode锁，既有退出检查正确拒绝Root终态；随后释放该原锁，以同一Source顶层入口重入。修改前仍报source_tool_reentry_child_binding_invalid，Root保持running，业务负向0/1（编译17.45秒/测试2.63秒）。没有手工INSERT failed子任务、用量、租约或退出事实。

最初新夹具把exceptional elapsed事实当作必然存在，Option::unwrap失败（编译44.10秒/测试2.57秒）；这不是业务复现。实际finally也可仅记录合法wall费用，因此按原事实是否存在核验，未补造elapsed或退出。错误日志保留。

## 最小生产修改

原owned入口增加已知失败Root的收口分支，在工具续跑前处理已经结清的失败；不会再把failed worker送入工具恢复。选择查询和布尔返回仅用于选择严格审计，不构成执行权、失败证明或退出证明。

复用原私有Root writer，核验当前原C、活attempt、原运行时与冻结材料、唯一实际失败worker、三轮SDK/工具/事件/checkpoint/费用、全部前序回执、精确粗账和无未知/预留的十维余额。原失败审计、SDK实际退出/原inode核验、事务内前后证明和OriginalFinalClock保持原实现；拒绝缺失材料、撤权、忙锁及未确认原义务，不补Root/C、模型请求、费用或成功消息。wall费用与原退出仍由既有金融路径按事实生成和核验。

原执行/恢复派发和失败writer全字节保持，原耗尽审计函数全字节保持；只有五行入口及一个新辅助函数，测试清单仅一个新include。既有测试与fixture/helper未改。

## 两个具名开发回归

1. source_exhausted_pending_root_reentry_closes_original_failure_after_actual_sdk_exit_without_resending：两个实际生产者分别为Mapper耗尽（SDK总5）和Mapper完成后Analyst耗尽（SDK总7）。锁仍忙的重入不写业务终态；原锁释放后收口同一Root为paused、Source分支partial，模型用量保留5请求/输入输出各50和7请求/输入输出各70，预留/未知0。原SDK/工具/邮箱/费用、Root金融身份/C及原assignment/worker的typed rows/rowid保持，不重跑、不续租。原退出证明有效；若已有原exceptional cutoff则保持原值，不虚构缺失事实。终态重入全库零变化，实际HTTP总量不增加。
2. source_exhausted_pending_root_reentry_rechecks_original_proof_and_pause_cutover_atomically：六场景各实际SDK5后进入同一pending Root。选择收口后、写入前通过真实request_sentinel_pause撤权，或者损坏前序邮箱、第三轮原回执、粗账、worker失败事实、静默忽略Root终态写入。逐一精确拒绝，Root仍running，无原退出凭据补写；费用请求5保留。故障后全表typed rows/rowid前像比较，仅允许原finally合法wall/elapsed附加，业务、权限和原回执不改。回执损坏仅在临时库移除不可变触发器后模拟，生产不可变规则未改。

首次生产修复正向exact1/1（编译22.75秒/测试7.13秒）；新增负向和局部fmt后两项2/2（编译41.51秒/测试22.22秒）。场景数不计测试数，没有修改已有断言、ignore或放宽生产审计。

## 最终同源码验证和工作树保护

- 关联58/58，选择=报告=通过；测试569.90秒/阶段570.77秒。含上一批56项及本批2项，集合不与历史结果拼接。
- 严格all-targets/all-features Clippy退出0（24.03秒日志）；退役50/50（8.21秒），包括精确literal登记和当前Native JSON原字节回环。
- 三个执行/测试叶局部fmt --check及范围git diff --check退出0，四修改文件均小于400行；未运行全项目所有测试、全UI或安装态门禁。

4代码路径3已有/1新，每个前像和先前git差异保存在/tmp/oviraptor-source-exhausted-root-reentry-*。1348原范围外源码SHA及HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b保持；1352源码集合SHA 0ad25f13dc4a5ad9c0faa3f03ad9512b17e3ee3bf7f78462711a464fb06f38ae。全增量及原字节保持证明已核对，不重置、不批量覆盖、不自动提交。

仅临时Git/SQLite/CAS和脚本SDK；未写真实DB/CAS/资产、未改UI、未安装或访问授权URL。分析器回调仍不是实际Docker执行，脚本SDK不是模型脑力/供应商美元对账验收。

## 仍未完成及下一步

失败/paused Source删除冷审计仍拒绝deleted_audit_original_source_completion_required；下一步先以原failed/paused任务证明删除拒绝和全部材料/财务/退出/受保护业务作用域，再实现独立失败冷审计，不降低完成审计。

本批只支持当前原C有效、原已知失败材料完整且实际SDK阻塞解除的Root恢复。撤权、取消、过期/替换C、未知费用、缺原事实、旧attempt、保护或清理未确认的pending恢复仍未完成；活动取消IPC、全HTTP/浏览器/进程后代和SIGKILL/重启仍待完。十维动态分配/美元对账、六监督、全15角色/真实模型及并行、聊天/实时日志/整体UI、回归债、非Web、数据知识和最后完整门禁/安装App打开/两URL验收仍保持原范围。

真实清理先精确盘点备份，不删除asset。Goal不暂停、不标完成。
