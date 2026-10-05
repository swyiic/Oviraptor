# 概览正式源码确认与读取竞态审计（2026-09-27）

## 结论与范围

概览已经消费当前轮次经过完整审计的正式源码裁决，不再只统计 Web 发布表。旧 Findings 表、历史 JSON 和手工验证保留各自口径，不被写回或升级成正式源码确认。此增量不是全项目完成，也不授予执行、重试或 CI 通过资格。

本轮修改涉及 `commands/appsec_validation.rs`、`commands/native_source_findings.rs`、`models_sentinel.rs`、前端类型及 `SentinelBoard.vue`。未访问用户目标 URL、远端 Worker 或外部模型，未部署，未启用 Host Agent，未放宽生产授权门禁。测试模型交互仅限 localhost 夹具。

## 后端统计与证据边界

`sentinel_overview_stats` 转到 blocking worker；同一次概览使用一个 deferred SQLite 读事务。如果调用方已经有事务，则复用并保持它的生命周期，不自行提交或回滚。

正式源码详情、导出和概览共享 `native_source_findings_audit`。该入口要求现有读事务，并返回 `NotAvailable`、`Unverified` 或包含不透明 `SourceDecisionSet` 的 `Audited`。不能通过导入 JSON、标签或 SQL 中的 verdict 字段伪造该对象。只有经过完整裁决集、四阶段交付、独立 Reviewer 回执、ACK 和冻结文件核验后才计数；不是仅检查当前分页或第一条记录。

### 当前轮次

- `sentinel_scans.attempt_count` 必须对应真实且最新的 attempt 记录。
- 新轮次尚未建立、尚未产生可用源码审查，计入“尚无可用审查”，不借用旧轮次确认。
- 指针与较新的 attempt 记录矛盾，计入“无法核验”，不自动选取旧结论。
- 根不唯一、租约缺失、裁决/模型回执/ACK 损坏、冻结字节丢失时，不提供确认数，保留无法核验任务计数。
- 旧轮次仍可通过显式任务/轮次详情入口读取；概览不改变历史数据。
- 有有效审查而零确认的任务，仍算“已核验”，不与“尚无审查”混淆；零确认不等于无漏洞。

源码任务通过 code 类型、保留的 source_path 或源码 root/assignment/裁决迹象识别；后两种方式覆盖灰盒及有源码分支的其他任务。仅 Web 的任务不增加源码审查缺口数。

### 字段口径

| 字段 | 本次含义 |
| --- | --- |
| `reviewerConfirmedCount` | 当前轮次 Web 合格发布数 + 当前轮次源码完整审计确认数 |
| `reviewerHighRiskCount` | 上述正式确认中 high/critical 数；不使用手工验证覆盖级别 |
| `sourceReviewerConfirmedCount` | 上述总数中的源码部分 |
| `sourceReviewAuditedTaskCount` | 本轮源码裁决集核验成功的任务数，包含零确认 |
| `sourceReviewUnavailableTaskCount` | 本轮尚无可用源码审查的任务数 |
| `sourceReviewUnverifiedTaskCount` | 本轮存在矛盾或不可核验材料的源码任务数 |
| `vulnerabilityCount` | 旧 Findings 表的漏洞记录数；不向该表复制源码裁决 |
| `otherVulnerabilityCount` | 旧表记录数减去 Web 合格发布数；不能减去源码账本条目 |

旧 `highRiskCount` 继续保留手工验证影响的历史口径，与正式确认高危数明确分开。任务、目标、指纹、接口、端点、漏洞、验证、熔断和机会统计均排除对应删除标记；底层记录不删除。

Web 资格继续要求投影与候选精确一致、同 revision 的 confirmed decision/request、Native 独立 Reviewer 已完成及 assignment 已完成；本次另外绑定 root/Reviewer 的 scan、当前 attempt 和 Reviewer parent，并要求真实最新 attempt，排除删除任务。

## UI 与异步读取

- 原生审核确认卡显示源码确认条数、已核验源码任务数、尚无可用审查及无法核验任务数。
- 明示“仅统计当前轮次；候选审查不代表整体覆盖完成或系统无漏洞”。
- 读取中显示“核验中”；读取失败显示“暂不可用”，隐藏该卡旧的细分确认数，不把失败显示成已核验零发现。
- 统计入口统一为带 project/generation/unmount 校验的刷新函数。普通加载、周期同步、机会更新、验证保存均不再直接接收无保护的旧统计响应。
- 项目变化立即清理上个项目的 KPI；同项目多次刷新只接受最后启动的请求；旧项目失败不会覆盖当前成功状态或弹出过期错误。
- 完整加载也校验 generation/project，避免旧列表响应覆盖新项目。验证后的详情写入增加所选任务/详情 generation 校验。
- 周期同步在加载或统计审计未结束时跳过，不每 12 秒继续叠加审计请求。

这里没有把完整源码核验伪装成便宜查询：每次仍审计冻结材料，不使用未经重新核验的成功缓存。此轮没有证明大量历史项目下的性能上限，也没有加入全局 IPC 并发硬限额或可靠取消正在执行的后台核验；这些仍应继续优化和压力验证。读取事务一致性只涉及数据库；冻结文件按核验时的实际字节审查。

## 红测与验证证据

先前未实现时的 4 项源码概览测试全部失败，真实结果保留在 `/tmp/oviraptor-overview-source-red.log`：确认数为 0、新状态字段缺失。前端 3 项真实 Board setup/template 红测复现了缺少源码分项、项目切换保留旧数值和同项目旧响应覆盖新响应，日志 `/tmp/oviraptor-overview-ui-red.log`。

扩展删除覆盖的首次测试因夹具缺少必填 `stage` 失败，见 `/tmp/oviraptor-overview-source-extended.log`；已补齐夹具字段，并重新通过全部源码概览测试。这不是被忽略的产品断言失败。

最终验证均已收取退出状态：

- 源码概览 6/6：`/tmp/oviraptor-overview-source-final.log`。包含 Web+源码混合计数、完整集损坏矩阵、文件丢失、只读连接、调用方事务、零确认、缺失/矛盾轮次、项目隔离、删除标记和无 Native 表写入。
- Web 溯源 1/1：`/tmp/oviraptor-overview-web.log`。包含跨 scan/attempt、缺失 parent/attempt、矛盾指针和删除标记的拒绝矩阵。
- 共用源码详情/导出 8/8：`/tmp/oviraptor-overview-source-findings.log`。
- 实际 Board 测试 15/15：`/tmp/oviraptor-overview-ui-green.log`。包括新增 5 项概览展示、竞态、失败状态及防周期叠加检查。
- 完整 UI 191/191：`/tmp/oviraptor-overview-ui-full.log`。
- 类型检查与生产构建通过：`/tmp/oviraptor-overview-build.log`。主 JS 845.39 kB，仍有 >500 kB 体积告警，未通过隐藏告警宣称解决。
- all-targets/all-features 严格 Clippy 通过：`/tmp/oviraptor-overview-clippy.log`。
- fmt 检查通过：`/tmp/oviraptor-overview-fmt.log`；`git diff --check` 通过。
- 退役字面量守卫 4/4、活残留基线 1/1：`/tmp/oviraptor-overview-retirement.log`、`/tmp/oviraptor-overview-residual.log`。

退役清单仅更新本轮实际改动且复核过的三个条目：`appsec_validation.rs` 的一个历史来源字面量、`SentinelBoard.vue` 的两个只读展示字面量、`tests_overview_provenance.rs` 的四个历史夹具字面量。未新增执行豁免或批量重新认可其他文件。

本轮 Cargo 验证一次一个，使用 `nice -n 15`、`-j 1`、测试 `--test-threads=1`；没有再跑耗时约十一分钟的 Rust 全量。它们不是 CPU 硬限额。没有进行当前真实 WebView/IPC、跨平台安装包或大量任务性能验收。

## 未完成项与后续顺序

1. 其他 Findings 消费者和 SARIF 导出/往返仍须对齐正式审查资格，不能把历史显示条件当确认权。
2. 独立总体覆盖审查仍有 `source_coverage_review` 缺口，候选审查完成不能使 CI 自动通过。
3. 顶层初评/工具恢复、过期租约与未知副作用的人工处理、真实 OS 强杀/重启及并发恢复仍须完成。
4. 大候选集分批/输出上限、Reviewer 取消/配置/期限、显式美元上限下的实际费用账本，以及规模化只读核验性能仍需推进。
5. Master Plan 的角色、指令、聊天、工具、历史、沙箱、离线供应、部署及完整门禁仍需逐项最终验收；本轮不标记整个目标完成。
