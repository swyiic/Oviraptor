# 源码分析结果的 attempt／视图绑定审计

状态：已接通真实导入提交凭据、不可变分析结果回执、生产源码结果工具与灰盒 sourceClaims。完整顺序流水线 session 9125 已退出 0：主库 1084／历史导入器 30、严格 Clippy／fmt／空白检查通过；不代表源码多智能体、独立 Reviewer、CI 资格或整个 Master Plan 已完成。

## 1. 已复现的缺陷

真实生产入口先完成 Diff 分析并返回 `app.py` 候选，再通过正常 canonical importer 导入同扫描的历史 SARIF。旧 `analyzer.list_results` 随 scan 级 current membership 改变，返回未选中的 `untouched.py` 并丢失真实本轮候选。红测 session 82851 退出 101，日志 `/tmp/oviraptor-source-result-receipt-red.log`；初步修复 session 90240 退出 0。

历史导入器的去重意味着“原始 revision 中写了哪个 bundle”也不能代表当前接受来源。因此没有简单添加 attempt 标签或把所有历史结果过滤为空，而是从实际导入提交开始建立绑定。

## 2. 实现

1. canonical importer 在提交事务内部读取本次解析结果实际对应的不可变 revision，返回 `ImportedRecordReceipt`：revision ID、logical key、revision hash、record kind、实际存储 envelope 摘要、本次输入 artifact 摘要。相同 revision 可复用，不依赖 current membership 是否新建。事务未成功、旧 attempt 被忽略、已删除扫描不投影时不能产生接受凭据；仅命中 signature 的跳过也不冒充新提交。
2. 源码流水线只收集经过分析器 outcome 摘要验证且实际复制到新接受目录的 SARIF。最终 IMMEDIATE 投影事务复核 importer bundle 文件名／摘要，绑定 scan、attempt、analysis manifest digest、实际 artifact、具体修订和已执行分析器状态。
3. `source_analysis_results` 与分析视图外键关联；拒绝 UPDATE、REPLACE 和独立 DELETE，允许父任务级联清理。写入数量、读回、完整内容摘要和源码范围均校验。结果接收、plan、sourceClaims 与 CI 投影共同提交，提交前再次检查活动 attempt、合同、来源／视图及结果凭据。
4. 生产 `analyzer.list_results/get_result` 读取本轮回执，不再查询 scan 级 current membership。按真实接受 engine 筛选，先筛选再 limit，并提供截断标记。具体结果返回本次接受的 bundle/artifact 与 revision；原 envelope 的历史来源原样保留，不伪造“本轮确认”。失败分析器及覆盖缺口仍可见。
5. sourceClaims 使用同一回执，包含 key、revision、scan/attempt 和两个 manifest/results 摘要；历史结果的补导、撤销和可变投影不能替换它。
6. 候选的主位置与全部 locations 必须精确属于选中文件；不能归入视图的路径保留为覆盖缺口，原始导入数据仍保留。当前不猜测未解析 SARIF URI/绝对路径到源码路径的映射。
7. 已有 source plan 的恢复必须保留原结果凭据；缺失或损坏不能重新生成。相同输入与修订幂等复用，新的分析结果不能覆盖既有接受回执。

## 3. 验证记录

- 初步 green：真实历史结果串入回归转绿。
- session 70058：8 项新增定向回归通过，6.55 秒，日志 `/tmp/oviraptor-source-result-receipt-focused.log`。
- session 60771 退出 0：源码相关主库 98 项、导入器匹配项 2 项、Native Pipeline 61 项、严格 Clippy 通过。日志 `/tmp/oviraptor-source-result-regressions.log`、`/tmp/oviraptor-source-result-pipeline.log`、`/tmp/oviraptor-source-result-clippy.log`。
- 跨 attempt 补测首版 session 92703：8 passed／1 failed。fixture 在切换 attempt_count 后读取旧的活动授权，被现有检查正确拒绝；修复为切换前取得原合同，没有放宽生产检查。日志 `/tmp/oviraptor-source-result-receipt-focused-v2.log`；该失败使后续 Clippy-v2 未运行。
- 跨 attempt 补测第二版 session 6174 退出 101：8 passed／1 failed。上下文改为 attempt 2，但测试创建的冻结执行计划仍是 attempt 1，现有逐工具权限检查正确返回 `tool_execution_surface_denied`。修复测试初始化，使上下文、run 和冻结计划指向同一次执行；未放宽生产授权。后续 `&&` 导入器／Clippy 未执行，不能算通过。
- session 87196 退出 0：新增 9 项回归全部通过（7.88 秒），独立导入器 30 项通过，retirement 守卫 4 项通过，严格 Clippy 通过。日志 `/tmp/oviraptor-source-result-receipt-focused-v4.log`、`/tmp/oviraptor-source-result-importer-v4.log`、`/tmp/oviraptor-source-result-retirement-v4.log`、`/tmp/oviraptor-source-result-clippy-v4.log`。
- 新增 9 项涵盖：历史结果迟到、灰盒与 engine 筛选、缺失/篡改/原 envelope 改变、跨 attempt 真实修订复用、越界位置、投影写入 ABORT/IGNORE/停任务、不可变与级联清理、重复/更换分析、输入 artifact 被替换、失败分析器可见。
- 最新完整顺序流水线 session 9125 已退出 0：主库 **1084 passed／0 failed**（553.55 秒）、历史导入器 **30 passed／0 failed**（2.12 秒），随后严格 Clippy、fmt、`git diff --check` 均通过。日志 `/tmp/oviraptor-source-result-full.log`、`/tmp/oviraptor-source-result-clippy-final.log`、`/tmp/oviraptor-source-result-fmt-final.log`、`/tmp/oviraptor-source-result-diff-final.log`。运行期间冻结 Rust 输入，没有因静默或预计时长而重启测试。

前端本增量未修改。补跑七组实际界面回归 session 66424 已退出 0：147 passed／0 failed，日志 `/tmp/oviraptor-source-result-ui.log`。localhost 浏览器回环 session 60798 已退出 0：`passed=true`、匿名／比较采集 complete、8 个观察请求、身份隔离 true，日志 `/tmp/oviraptor-source-result-loopback.log`。前端生产构建 session 69516 已退出 0，vue-tsc／Vite 通过，日志 `/tmp/oviraptor-source-result-build.log`；既有主 JS 超过 500 kB 的拆包警告仍保留。未部署、未访问外部授权 URL、未新增 Host Agent。

## 4. 仍必须继续

- source 专用 assignment → child-run → mailbox →独立 Reviewer 执行链；assignment.finish 仍缺对应持久收口。
- 真实 source root／当前 candidate revision／独立 review 与 CI 通过条件，不能用 scan ID 或“最新决定”冒充。
- 源码/灰盒完整派发冻结、在途撤权、凭据生命周期、沙箱/工具供应、学习治理、资产与 UI、真实安装包及授权环境验收。

本次只读复核进一步定位：

- 当时 `commands/native_source_scan.rs` 将 `scan_id` 传给 `ci::evaluate` 的 `root_run_id` 参数。后续源码专家合同增量已移除这一调用：生产明确输出未复核缺口，不从错误 root 或历史决定推断通过；真实 Source Coordinator/Reviewer 编排仍待接通。
- 原 root-wide CI 决定读取器现在仅用于阈值测试；生产 `evaluate_unreviewed` 不把零决定当作已完成独立审查。真正可通过 CI 的本轮候选/revision 与独立 review delivery 资格仍未实现，不能把这一真实性修正当作完整审查链。
- `commands/agent_tools_source.rs` 仍取 root 图的 `MAX(revision)`，`SourceBroker::finish` 仍用内存状态和图计数返回 finished；后续应绑定真实 source assignment、candidate 集合与 revision，并原子提交 completion、预算结算、mailbox 和真实时间线事件。
- 后续 `multi_agent/scheduler.rs` 和 `specialist.rs` 已加入 RepoMapper/SourceAnalyst 的只读初评合同、不可变材料绑定与真实模型回执/mailbox 交付；localhost transport 已覆盖。但生产源码启动器尚未调用这些角色，独立 Reviewer 未接通。具体边界与验证见 `NEST_SOURCE_SPECIALIST_CONTRACT_AUDIT.md`。
- 接受回执当前通过输入 hash 选择 engine。多个 engine 接受完全相同字节时会命中第一个 artifact；多引擎合并来源及精确归属仍需专门回归和修复，不能将目前单引擎过滤回归扩张为全场景证明。

本回执是后续专家与 Reviewer 使用真实本轮材料的前提，不是这些执行链已经交付的声明。
