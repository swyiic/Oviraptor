# Source ToolPending 恢复耗尽后的原 Root 收口（2026-10-05）

Master 框架未完成，Goal active。本批仅收口具有完整原回执和已知费用的恢复耗尽路径，不能作为真实模型质量或整体功能验收。

## 问题和数据作用域

签名 Source 发布、原 Root/当前 C、实际分支领取和 SDK checkpoint 的临时库回归：两个初始请求及 Mapper 第一个工具请求真实返回；本地工具交付被临时忽略，留下 received/planned 原事实。实际顶层恢复入口继续至第三轮，失败子任务正确结清，但 Root 仍 running。工具调用 ID 修正后，业务负向 0/1，编译18.69秒、测试2.38秒，失败是 Root状态 (running,空) 与 (terminal,paused) 的差异。

最初夹具重复使用工具调用 ID，被生产正确拒绝 source_round_tool_id_reused，未算业务复现。只给两新增模式使用按转录长度区分的 ID，其他原模式和 helper 原字节保持。所有库、Git、CAS 和 SDK endpoint 均是临时测试资源；本批没有真实 DB/CAS/资产写入、UI修改、安装、URL访问或提交。

## 最小生产修改

原 ToolPending 未知继续保持早退；只有精确 round_budget_exhausted 错误才选择更严格的同事务审计。错误字符串和选择布尔值不构成执行权或失败证明。

审计从原数据库发现唯一实际失败子任务，不假设保存的 Mapper 仍是失败方；核验原角色/任务/Root/attempt/target/C/fence、原 worker失败事实、三轮模型/工具/事件/预算/checkpoint、完整前序回执与冻结材料、粗预算精确已知费用且无预留、十维无未知和预留，拒绝额外 worker、消息、审核、Root/Web历史或活lane/权限。Root写入前后重复证明并核对同一原子任务身份。

使用既有私有 financial closure writer、当前授权和 OriginalFinalClock/实际 SDK退出核验，不创建新 Root、租约、模型费用或执行授权；wall 费用和原退出证明由既有 writer 基于原事实生成并核验。结果仍 paused/partial，不发送失败子任务成功邮箱，不制造独立审核。原成功/恢复派发逻辑原字节保持。

## 三个具名开发回归

- source_recovered_exhaustion_original_paid_rounds_close_root_without_resending：实际 SDK总5，保留100 tokens/5请求、输入输出各50；预留与未知0。Root paused、分支 partial、原退出证明有效；原费用/原回执和rowid保留，终态重入全库零变更、零追加 SDK。
- source_recovered_mapper_finish_then_analyst_exhaustion_closes_actual_failed_child_root：保存的 Mapper finish 本地交付后完成，后续 Analyst真实三轮耗尽；实际 SDK总7，保留140 tokens/7请求、输入输出各70；精确认出失败 Analyst，Mapper仅有原成功消息，其余同上。
- source_recovered_exhausted_root_rejects_cutover_corruption_busy_and_silent_writes：七场景各实际 SDK5。原Root撤权、前序邮箱损坏、第三轮状态损坏、粗账不符、worker失败事实损坏、Root终态写入被忽略、原SDK inode实际忙锁，均拒绝 Root收口。以故障后前像比较全表 typed rows/rowid，业务/权限/原费用/回执不改；仅允许原 finally 的合法 wall费用或 exceptional elapsed 事实。第三轮损坏仅在临时库显式移除不可变触发器后模拟；生产不可变规则未改。

第一正向单项1/1（编译36.80秒/测试2.70秒）；扩展三项首轮2/3，新增负向取消拒绝码预期失准。另有新增测试模块路径编译错误和尝试损坏不可变回执被触发器正确拒绝的夹具错误，日志均保留。收紧为实际 coordinator_not_executable，损坏仅限定临时库后，负向exact1/1（编译16.51秒/测试16.73秒）。不改已有测试，不放宽生产规则，不ignore。

## 最终同源码验证

- 关联 56/56，选择=报告=通过；测试527.65秒，阶段528.17秒。包含上一批34项、完整 Source reentry 18项、原暂停切换1项、本批3项；集合不与历史相加。
- 严格 all-targets/all-features Clippy退出0（18.20秒编译日志）；退役过滤 50/50（5.50秒），含 literal allowlist登记与当前 Native JSON原字节回环。
- 四执行/测试叶局部 rustfmt --check、范围 git diff --check退出0；所有修改文件小于400行。未运行全项目所有测试、整套UI门禁或安装态验收。

6代码路径3已有/3新，逐文件前像、先前git差异及最终增量保存在 /tmp/oviraptor-source-recovered-exhausted-*。1345原范围外源码SHA和HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b保持；1351源码集合SHA 504d78ad86dd9bfcc2af25a6426e3ff435da490a849090f87ca7683453ac69c2。原 fixture其他模式/helper、原测试清单（仅两个新include）、原执行/成功与恢复派发闭包字节比对保持。

## 剩余义务和下一范围

失败/paused Source的删除和冷审计仍精确拒绝 deleted_audit_original_source_completion_required，未扩删除权限。Root收口因撤权、材料损坏、忙锁或清理未确认而被拒绝后，保留的 pending Root仍需要原事实重新核验和恢复路径；不能借本批一次成功的收口宣称全恢复完成。旧attempt、保护/取消/未知费用、活动取消IPC、远端/浏览器/进程后代退出、实际分析器与真实模型质量均待完成。

Master的十维动态分配、六监督、15角色/真实推理、实际并行、聊天、逐路日志、整体UI、回归债、非Web、数据知识及最后全量/安装/授权URL验收仍在范围内。真实清理先精确盘点备份，不删除asset，不自动提交。脚本 SDK不证明供应商美元对账或真实模型脑力，分析器回调不等于真实Docker执行。
