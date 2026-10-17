# 原 Root 时限与父续租审计（2026-10-02）

Master 继续开发，未完成。本批只使用隔离库；没有真实业务库、资产/CAS、安装包、git 提交或授权 URL 操作。此前真实 Nest 精确59行清理以独立清理审计为准。

## 问题、失败和作用域

- 原 Root 的冻结硬时限已耗尽，但没有 worker 回调时后台父仍报告健康。先冻结合法3秒计划，再签发真实持久 worker/dispatch claim，自然等待原 Root 时间；实际红测0/1，3.19秒：`/tmp/oviraptor-parent-governance-clock-red.log`。没有真正发出该模型请求，不能把 claim 当提供方未知账单。
- 当前父 Coordinator 将到期时，没有 worker 回调便不续租。将原有效租约设为未来15秒，等待后台服务，实际红测0/1，0.85秒：`/tmp/oviraptor-parent-governance-renewal-red.log`。
- 第一次 clock 实现已经满足时限/凭证断言，测试却误认为仍有效的 executing claim 必须被 require_determinate 拒绝；改为核验原预留/槽位不变且新 grant 拒绝。组合回归48/49中的另一个失败，是既有 billing successor 夹具无冻结计划；仅给测试中新 Root 初始化其独立计划，生产继续拒绝无计划父，未修复旧 Root 或降低门禁。所有失败日志保留。

## 最小实现

- clock 共用正向原时间计算，后台观察原 Root 的冻结期限。完整十维配置沿用原 origin/limit；未签发的新 Root 仅在全部用量/预留/child/assignment/entry/origin均空时只读观察原 created_at+plan。部分维度、历史用量、未来原点或损坏计划拒绝，不回填账本或权限。
- 健康原父仅在自身 Coordinator 临近到期时续租；不 acquire、不接管过期或新 generation，不续 worker、不重置 Root 时间。候选绝对截止时间取当前600秒与原 Root硬期限的较早值，已有更晚元数据不缩短，执行仍受原 Root clock 阻断。
- IMMEDIATE 事务内核验原身份、可执行状态和活父；SQLite authorizer仅允许 Coordinator 三个时间列变化，禁止资产/预算/业务/其他表触发器副作用。typed rowid 全行快照要求所有 Coordinator 行只有原作用域三个时间值按预期变化，最后复核后提交。短暂 writer 竞争等待下一轮，其他故障撤活并回滚。
- authorizer 在续租后清除，原 worker 到期撤权仍能执行。续租和随后 worker 撤权是两个事务；后者失败不代表已经提交的健康心跳也回滚。

## 合同与门禁

新增8项，主库1720→1728：原硬期限自然停止、原续租后新签发且旧 worker 全列不变、续租只改原父、pristine观察及七类坏历史、configured原点冲突、十类续租故障、writer竞争、续租后撤权。

- 最后逻辑版本合同49/49，51.84秒：`/tmp/oviraptor-parent-governance-final-contracts-fixed-fixture.log`。
- 受影响扩大172/172，119.71秒：`/tmp/oviraptor-parent-governance-final-affected.log`；严格全目标全特性 Clippy exit0，12.63秒；导入39/39，2.36秒；严格字面量1/1，0.49秒。集合重叠不相加，不是最终全量或整体功能验收。
- 此后仅格式化上述 successor 测试新增夹具语句（无逻辑修改），最后该实际合同1/1，0.55秒；9文件 scoped fmt、差异检查及873路径复核通过。扩大/Clippy属于该测试语句格式调整前快照，不能隐瞒快照差异。

## 文件与保护

6已有+3新增，无删除。已有 clock224、limits189、mod32、supervisor204、测试入口57、worker_supervisor286行；新增 coordinator_heartbeat158、parent_governance271、parent_governance_faults147行。共享预算/监督留在 runtime，测试留在多智能体模块，无超400的新手写文件。

基线870路径；逻辑版本873路径摘要`99b98961423df2b28fdfed5174e00a729989b6600700129a9cc3575133a5a0b7`。最后格式版本`/tmp/oviraptor-parent-governance-formatted-code-snapshot.json`，摘要`b5ab25de456c33ebe51e19ee24a4d7dd08e6473eaf2b2dd4d051b079d45b74e7`，HEAD仍`59be3d86d25adda5b1f975759bf256ef3b94f32b`。修改前文本、逐文件差异和作用域见`/tmp/oviraptor-parent-governance-*`，不覆盖其他未提交成果。

## 未完成和风险

同 Root 可重复创建监督服务、持久父实例/OS worker 归属和强杀完整恢复矩阵继续处理。Weak 凭证只保证旧捕获 context 不被新父复活，尚不证明新父不能给旧 worker 新凭证。状态页仍主要读取持久租约，不能据此推定活父。

自然期限合同是合法短冻结 Root，续租合同使用未来15秒 Coordinator；不代替正式600秒全生命周期或安装 App 强杀。Root/Single十维预算、动态 grant/人工对账、Source/Reviewer安全重派与canonical请求、跨Coordinator财务恢复、剩余角色、聊天闭环/逐路实时日志及最后完整门禁、安装态、授权URL未完成。继续框架开发，不标 Goal 或 Master 完成。
