# 原 worker 原子过期撤权开发审计（2026-10-02）

Master 框架未完成。本批补生产撤权事务及真实清理调用入口，仍没有后台 Supervisor、安全重派或强杀整体恢复验收。当前 Native JSON 不变；没有访问真实 DB/CAS、资产、安装包、两个授权 URL，也没有提交或重置。

## 原问题与作用域

- `/tmp/oviraptor-atomic-expiry-cleanup-red.log` 0/2：只令原 worker 到期，实际生产清理仍将 worker 写成 paused，不能区分执行尝试过期与逻辑任务暂停。
- `/tmp/oviraptor-atomic-expiry-worker-cost-red.log` 0/2：Coordinator 仍有效时，原 worker 到期后的专家/Source 响应仍返回正常可消费回执。
- `/tmp/oviraptor-atomic-expiry-pending-admission-red.log` 0/1：到期后、晚到回调前的 executing 调用会漏过新预算准入。
- `/tmp/oviraptor-atomic-expiry-root-red.log` 0/1：child 为 Native 时，非 Native Root 仍能执行撤权写入；修复增加 Root native/multi/Coordinator 绑定，不把退役行重新激活。
- `/tmp/oviraptor-atomic-expiry-repeat-red.log` 0/1：重复撤权接受损坏的逻辑完成时间；修复核验 paused 未关闭且持有原 lane，或合法 completed 已结清并释放 lane。
- `/tmp/oviraptor-atomic-expiry-business-red.log` 0/1：最后撤权写入触发候选投影旁路改动未被识别；最终证明增加原 Root 的候选/证据图及 scan/target 当前结果投影。
- `/tmp/oviraptor-atomic-expiry-missing-worker-restored-red.log` 0/2：损坏恢复库缺少原 worker 时，专家/Source executing 调用会漏过预算准入。首次 `/tmp/oviraptor-atomic-expiry-missing-worker-red.log` 只是夹具正常删除被不可变 guard 拒绝，不作为该问题证据。新测试先证明正常删除仍被拦住，再只在临时库移除该 trigger 构造损坏恢复；生产 guard 保留。

以上均用临时 SQLite/临时任务目录；全表快照只读取这些临时库。生产证明只读指定 Root、原 assignment/child，以及对应 scan/target 的 Native 事实，不读资产或 CAS。没有新增 migration、旧行回填或自动清理。

## 当前生产合同

`attempts::try_expire_original_in_transaction` 已接入 `stop_failed_child_preserving_usage_in_transaction`；现有生产 wrapper 持有 IMMEDIATE 写锁，Reviewer 的失败收尾也使用该入口。要求当前有效的同 Root Coordinator 和活动 scan/open Root；原 child/role/lane/scan/attempt/target/Native multi 完整匹配。过期身份从原 assignment/worker 读取，不采用 actor 的新 generation。

到期时只修改：原 worker 的 expired/finished_at/failure_class；原 assignment 的 paused/failure/updated_at；原 child 的 paused/updated_at；原绑定 capability 的 revoked_at；对应的两类持久协作 outbox。原 UUID、worker fence、ordinal、heartbeat、deadline 和旧 Coordinator 归属不变。预算账本/entries/clock、模型/目标调用、lane、合同及并发槽不释放、不借用、不续租。未知费用不会因过期变成零。

事务初始捕获四类 Root 资源行及 31 类冻结查询；最后比较同一证明、旧 outbox 和本次精确 outbox，再核验 actor/Root。IGNORE、ABORT、漏 outbox、预算/调用/lane/Root/候选旁路改变均回滚全部临时库应用表。正常到期、重复到期及本地已完成后的重复检查保持原 worker 审计；坏日期/错误逻辑收尾拒绝。

同 Root 当前 successor 可撤销旧 worker 执行权，但不会继承旧财务或业务发布权。旧 actor、不同 Root actor 拒绝；非 dispatch owner 的 duplicate 调用仍不得撤权。当前接口只供撤权，不签发 ordinal 2，不更新 assignment 的 child pointer，也不自动重派。

专家 received/uncertain/typed unsent 与 Source received/uncertain 回调现在单独检查原 worker deadline。到期或已 expired 时仅保存原费用事实，received 返回拒绝，不产生业务响应/工具计划/消息/检查点或新权限；费用 owner 和 dispatch 全列证明沿用已有费用通道。已知原费用/合法 unsent 不误判为未知，未知事实继续阻断新预算。到期 executing 尚无终态费用事实，或恢复库缺原 worker 时，新预算准入只读拒绝，保留原预留。

## 并发与验证边界

两 SQLite 连接、channel 与真实 `SQLITE_BUSY` 证明生产续租/撤权的写锁顺序：续租先获得锁则撤权重读未来 deadline、返回 false 且不写；到期撤权先获得锁则续租拒绝，不能恢复原权限。没有用 sleep 模拟并发。显式到期 fixture 只改 deadline，expired 转换由生产 API 写入；它仍不是真实 Supervisor 定时扫过期或进程强杀的验收。

- 最后修改前定向 `/tmp/oviraptor-atomic-expiry-current-green.log` **14/14，exit 0，6.33 秒**。之后仅增加已完成幂等/已知费用正向控制断言及作用域格式；最新扩大回归结果待收取，不借此旧日志证明最后代码。
- 最后代码扩大预算/attempt/专家/Source/Reviewer `/tmp/oviraptor-atomic-expiry-affected.log` **201/201，exit 0，495.23 秒**；补充 Source 候选 Reviewer/保存结果/过期/工具恢复 `/tmp/oviraptor-atomic-expiry-source-recovery.log` **52/52，exit 0，446.02 秒**。两组包含既有 localhost 真实传输合同；新 14 项包含在扩大集合中，集合不相加。这是开发合同验证，不是当前全量或整体验收。
- 严格全目标全特性 Clippy `/tmp/oviraptor-atomic-expiry-clippy.log` **exit 0，20.82 秒**；退役字面量登记 `/tmp/oviraptor-atomic-expiry-retirement.log` **1/1，exit 0，0.49 秒**。15 文件作用域 fmt、git diff --check 通过；未运行当前最终完整 Rust/UI/构建、安装态或授权 URL 验收。
- 831 源码/测试/配置路径快照 `/tmp/oviraptor-atomic-expiry-code-snapshot.json`，aggregate SHA-256 `bff3f9464db2173e93b4e2d5c3c02a1b995dbde562a45e4cd4b47740147d600e`，全部最终复核与开跑时一致。相对上一批 824 路径，8 个已有路径改变、7 个新增、0 个缺失；HEAD 仍为 `59be3d86d25adda5b1f975759bf256ef3b94f32b`，当前展开 dirty paths 932（包含既有未提交成果，不当作本轮修改数）。原文本 `/tmp/oviraptor-atomic-expiry-before.json`、本轮追加保护 `/tmp/oviraptor-atomic-expiry-audit-before.json`、增量 `/tmp/oviraptor-atomic-expiry-scoped.diff`。
- 15 个代码路径均不超过 400 行：attempts 170、expiry 116、proof 266、gap_validation 328、specialist 341、receipt 282、Source dispatch 369、admission 87；新四份 attempt 测试 153/144/201/106，Source 测试 118；两个 include 文件 44/92。没有移除或删减既有测试。

## 仍未完成

安全重派需要原子签发新 child/worker/ordinal、独立 journal/grant 命名空间及逐 attempt 财务边界；当前 specialist journal 主键 assignment、Source 主键 assignment/round、capability 唯一 assignment/capability 都不能直接复用。不能删除旧行或换 child pointer 伪装重派。跨 generation 保存结果的业务发布、后台 Supervisor/真实强杀、Root/单智能体预算、动态 grant/精确人工对账、Source typed unsent、聊天完整四态与工具交付、外部/AST/浏览器逐路实时日志、旧数据精确盘点备份清理、最终完整门禁/当前安装态/授权 URL 均继续未完成。

Goal 状态核验：现有目标内容正确但 `get_goal` 返回旧 `blocked`；本批没有技术阻塞，也没有将目标 complete/paused。工具不提供恢复 active 的操作；尝试检查应用内恢复入口时，Computer Use 明确禁止控制 Codex 应用，未绕过限制。需要用户在 Goal 卡片恢复持续调度；本轮按用户继续开发指令完成上述增量，不能将历史 active 文案当作实际目标状态。
