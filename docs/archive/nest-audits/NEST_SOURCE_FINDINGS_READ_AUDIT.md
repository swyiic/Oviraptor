# 源码正式裁决读取与任务证据页审计（2026-09-27）

后续原源码结果页、独立审查 JSON 导出及父页面竞态修复见 `NEST_SOURCE_RESULT_EXPORT_AUDIT.md`。本页 1195/30 完整回归是后续改动前的基线，不能作为后续版本已全量验证的证明；下面保留本阶段的真实交付记录。

## 交付范围

新增 `get_native_source_findings` Tauri 命令与任务中心“证据”页的源码审查区域。Code/Greybox/CI 可选择执行轮次，读取该轮正式裁决的确认发现。旧 JSON 的只读历史预览和原结果页入口保留，未复制一套可绕过裁决审计的 Finding 数据。

这不是整个 Master Plan 完成：旧结果页、其他 Findings 消费方和源码导出尚未统一；总体覆盖独立审查也未完成。界面和接口仍明确区分“候选已经独立审查”与“整个任务已充分覆盖 / CI 已通过”。

## 后端数据合同

- `scanId`、`attemptNumber` 精确定位；拒绝不存在的轮次、其他任务和已删除任务。轮次必须正数，offset 非负，limit 为 1–100。
- 以 SQLite 读取事务核对唯一源码根及其原 Coordinator 绑定。多个根不任意选取；缺少租约记录或绑定变化不接受。历史租约到期不影响只读审计，但读取绝不续权或恢复执行。
- 只调用类型化的 `source_decisions::read_audited`，复核四阶段交付、冻结文件、独立 Reviewer、真实模型回执、ACK、精确裁决集合；不从报告 JSON、模型自述或旧导入结果取得“已确认”资格。
- **完整集合审计在分页之前**。即使损坏位于后续页，第一页也不能展示确认发现。
- `not_available` 表示没有原生源码根；`unverified` 表示当前不能核验完整正式裁决；两者均返回空发现和 `counts: null`。只有 `audited` 才返回正式统计，包括确认、排除、证据不足；核验后零确认与尚未核验不是同一状态。
- 仅返回已确认发现，绑定 scan/attempt/root、Reviewer、裁决 ID、材料摘要与证据引用。人类可读标题、路径、CWE、理由再次脱敏，不改写不透明来源 ID。
- 命令在 blocking worker 中完成数据库和文件核验，避免同步阻塞桌面 UI 线程。命令不修复裁决、不回填旧记录、不更新资产清单、不修改任务 gate。

## 前端行为

`SourceReviewEvidence.vue` 使用真实 API，位于 `SentinelTaskCenter.vue` 的“证据”页。

- 显示轮次、已审计统计、确认发现标题 / 级别 / 路径 / 行号 / CWE / 理由；折叠展开裁决、Reviewer 和证据引用。
- 固定声明总体覆盖审查尚未完成，不将候选审查成功显示成全任务安全或 CI 成功。
- 校验返回版本、任务/轮次、根和材料绑定、统计一致性、Reviewer 独立身份、裁决 ID、证据引用、分页进度和重复项。
- 切换任务/轮次或重新核验先清除旧发现；忽略迟到请求。分页串行，只合并相同根、材料和统计的结果。
- 后续页核验失败也撤销前面已展示的确认列表；错误不暴露后端原始异常。
- 使用 Vue 文本插值，不把模型/历史字段当 HTML 或可执行链接。
- 历史导入区域仍标记“未审核 / 只读”，不会获得当前 Reviewer 资格。
- 证据页选轮次复用统一 `chooseAttempt`，同时撤销并重读日志、通信和工具缓存，不能将旧轮次的记录显示在新轮次标题下。为长路径、标题和证据 ID 保留自动换行。

## 故障测试与真实失败记录

后端新增五项测试，经过真实 localhost 模型传输和磁盘 SQLite，比较读取前后的全业务表快照：

1. 确认 / 图候选确认 / 排除 / 证据不足投影；参数边界、跨任务、旧轮次及过期历史租约。
2. 决策 JSON 损坏、决策缺失、ACK 缺失、模型回执损坏、租约缺失或 fence 不一致，以及软删除任务。
3. 至少两个真实确认候选的分页；后续页损坏使第一页失效。
4. 旧版 v2 和 Reviewer 尚未交付的结果不能升级为正式发现。
5. 多源码根歧义、冻结文件消失；恢复同一冻结文件后才恢复原核验结果。

故障注入仅在测试 savepoint 内撤去不可变触发器，回滚恢复触发器和数据；生产保护不变。首轮夹具更新被不可变保护拒绝，后续又被模型回执保护拒绝。多确认夹具最初没有文件读取证据，生成了无效引用。修复夹具采用真实 `repo.read_slice` 回执，未放宽生产合同。

严格 Clippy 首轮报告 `manual_range_contains`，已改用区间判断，未添加 allow。

页面接线复核实际发现跨标签页串轮次：新选择器最初只更新 `selectedAttempt`，没有清掉执行页缓存。新增实际 SFC 事件回归在 `/tmp/oviraptor-source-findings-attempt-red.log` 复现旧轮次工具记录仍保留；改走 `chooseAttempt` 后 `/tmp/oviraptor-source-findings-attempt-green.log` 的七项通过，并同时核验日志 / mailbox / tool 三类轮次及重复请求数量。

## CPU 事故后续

重新阅读线程采样，另两条残留线程分别来自重定向服务和出网哨兵，不是两条相同的重定向服务。它们原先阻塞在无退出条件的 `accept`，不能仅凭这个样本认定为高 CPU 来源。

- 共用 HTTP 夹具增加原始响应构造入口，保留调用方弱引用生命周期；重定向 fixture 通过它生成 Location 响应。
- 出网哨兵改用相同生命周期服务，记录请求并返回 502，绝不建立 CONNECT 隧道。原正向自检、Native/历史导入路径及被禁域名断言保留。
- 增加两个 fixture 释放后可重新绑定监听端口的测试；原五项共用 HTTP 生命周期测试保留。
- 仅重新审核并更新已登记 `agent_tests_backend_residual.rs` 的证据哈希和理由；16 处历史字面量不变，没有添加文件豁免、活动符号基线或修改冻结历史 JSON。

## 验证台账

- 新接口定向回归：`/tmp/oviraptor-source-findings-api-final.log`，主库 5/5，exit 0。另两个 target 过滤后零测试，不算导入器全量通过。
- Reviewer 定向回归：`/tmp/oviraptor-source-findings-reviewer-regression.log`，23/23，exit 0。该运行先于后续 fixture 生命周期改动，完整组合需以下新回归证明。
- 实际 API / SFC 测试：早期 163/163 及相关 14 项通过记录保留；补齐父页面接线、跨标签页缓存回归后，最终 `/tmp/oviraptor-source-findings-ui-final.log` 为 165/165，exit 0，耗时 4436.955 ms。
- 前端类型检查及生产构建：最终 `/tmp/oviraptor-source-findings-build-final.log`，exit 0，覆盖最后的选择器及换行样式修改。主 JS 837.62 kB 的拆包警告保留，没有提高阈值掩盖。
- 全 targets/features 严格 Clippy：`/tmp/oviraptor-source-findings-clippy-final.log`，exit 0。
- 完整 Rust 组合 session 66703 已收取 exit 0；日志 `/tmp/oviraptor-source-findings-full.log`。主库 1195/1195（676.89 s），历史导入器 30/30（2.27 s），主程序 target 为零测试。此轮统一覆盖最新 Rust 改动，包括 Reviewer 夹具、源证据读取、历史兼容、退役守卫与监听释放。
- 该完整运行每两秒采样 Cargo 可见后代，共 358 次；测试进程最大读数 100%，包含编译阶段的进程树总和最大约 247.1%。采用低优先级、单作业、单测试线程，不是 CPU 硬上限，也不能排除采样间隔内峰值。进程于 2026-09-27 18:46 左右正常结束；收尾进程检查未见残留 Cargo test、测试二进制或 rustc。
- 最终 Rust fmt 检查和 `git diff --check` 通过；登记文件内容哈希另行只读核验，不因全量通过而扩大豁免。
- 尚未运行本增量的真实桌面 WebView / Tauri IPC 交互验收；SFC 渲染测试和编译不能替代该项。

## 后续必须继续

统一其他 Findings 消费和源码/SARIF 导出；总体覆盖独立审查；早期阶段恢复、过期租约/未知效果人工核对、强杀重启及并发恢复；大候选集批次与输出预算、在途取消、美元记账；全部历史引用分类和打包跨平台验证；聊天/用户指令/沙箱/离线工具与部署的 Master Plan 完整验收。

本轮未部署、未访问用户提供的外部 URL、未请求外部模型、未新增目标主机操作或移除生产授权门禁。
