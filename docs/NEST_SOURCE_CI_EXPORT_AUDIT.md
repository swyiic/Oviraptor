# 生产源码 CI 历史导出审计（2026-09-27）

## 结论与边界

正式源码审查 JSON/SARIF 导出已接入生产 `source_ci::evaluate`：报告包含同一尝试的冻结信息、发布策略、计数、门禁状态、退出码、原因与覆盖缺口。不读取页面 JSON、旧 Web 测试专用 GateReport 或可变任务设置来重建权威。

任务完成、租约过期、当前任务已有新尝试，不妨碍读取旧尝试的已审计报告；这条历史读取路径不续租、不派发、不补写决策、不修改原执行门禁。

**仍不是完整 CI-006 或 Master Plan 验收。** 两种格式各自的真实往返已验证；JSON 与 SARIF 联合导入的共同身份及作用域归并、多文件整体发布、独立总体覆盖审查、真实 WebView、打包和最终验收仍待完成。候选审查完成不代表 CI 通过。

## 生产接线

1. 保持已有 `export_native_source_findings` IPC：`scanId`、`attemptNumber`、可选 `format=json|sarif`。没有新增由前端传入门禁结果的参数。
2. `native_source_findings_export_snapshot` 在一个只读事务中审计整套 SourceDecisionSet，保留审计对应的 CoordinatorLease 身份。
3. 从同尝试 NativeSourcePlan 判断是否为 CI；非 CI 报告的 `review.ciGate` 为 null，不伪造 CI 门禁。
4. CI 从不可变 `source_ci_policies` 读取精确尝试的策略；历史读取允许终态/旧尝试，但仍要求真实任务、真实尝试且任务未删除。缺失策略拒绝导出，不回退默认设置。
5. `source_ci::evaluate` 再核对 opaque 决策集合、封存分析材料、源码计划和分析器回执，重建生产门禁结果。
6. 删除门禁中的重复 `findings` 和 `sourceDecisionProjection`，只保留 CI 上下文；上下文脱敏，发现仍使用原有逐字段脱敏投影。
7. JSON 存在 `review.ciGate`；SARIF 存在 `runs[0].properties.oviraptorSourceReview.review.ciGate`。运行摘要不会按每条发现重复。
8. 继续使用 UUID、不覆盖、完整写入后发布的单文件导出器。任一资格检查失败，不创建导出目录，不生成部分报告。

历史策略读取函数与执行策略读取函数分开：`load_workbench_ci_policy` 的 scanning/current-attempt 条件保持原样，不能为了历史导出而放宽执行权限。

## 保留的字段

- `freeze`：headSha、baseSha、diffBase、treeHash、rulePackDigest、analyzers、scope、fileCount、analysisManifestDigest 及原 schemaVersion。
- `policy`：实际尝试发布的 releaseLimits 等生产策略。
- 门禁：status、exitCode、reasons、gaps、counts。
- 身份：scanId、attemptNumber、rootRunId。
- 审查状态：independentCandidateReviewCompleted 保留；independentReviewCompleted 仍为 false。`source_coverage_review` 不被删除。
- 整份导出仍声明 `qualification=verified_at_export`、`executionEligible=false`、`coverageReviewCompleted=false`。

这些字段是导出快照的历史证据，不是可移植的执行/发布授权，也不是保证导出后目标仍安全的证明。

## 真实回归与失败记录

- 红测 `/tmp/oviraptor-ci-export-red.log`：新增完成任务历史导出回归实际失败，freeze 字段为 null，证明原有报告缺少生产 CI 上下文。
- 第一轮源码绿测 `/tmp/oviraptor-ci-export-green.log`：12 项通过。
- 补充故障与回导断言后 `/tmp/oviraptor-ci-export-final.log`：13 项通过，49.58 秒。
- JSON 实际导出文件回导 `/tmp/oviraptor-ci-export-json-roundtrip.log`：1 项通过，2.88 秒；CI 上下文在历史 RunState 扩展中完整保留，原文件及 CAS 原文不变，Native 表不变。
- 策略回归 `/tmp/oviraptor-ci-export-policy.log`：8 项通过，1.89 秒；执行入口仍严格绑定当前活跃尝试。
- 概览回归 `/tmp/oviraptor-ci-export-overview.log`：6 项通过，13.83 秒；共享审计结构调整没有把历史来源升级为当前确认。
- 完整 UI `/tmp/oviraptor-ci-export-ui.log`：192 项通过；增加导出包含 CI 上下文的界面说明。
- 类型检查与构建 `/tmp/oviraptor-ci-export-build.log`：通过；主 JS 845.83 kB，超过 500 kB 的拆包告警仍保留，未改阈值掩盖。
- 首轮严格 Clippy `/tmp/oviraptor-ci-export-clippy.log` 失败：新增租约字段使 SourceFindingsAudit 出现 large_enum_variant。改为装箱租约，未关闭规则；`/tmp/oviraptor-ci-export-clippy-green.log` 已通过。
- 装箱后源码复测、格式与退役检查的最终状态见下方收尾记录，不能把零匹配测试命令视为通过验收。

新增故障回归对 JSON/SARIF 分别验证：缺少策略、head/base 被修改、清单损坏、末条裁决损坏、任务删除均拒绝发布；不创建目录，不修复/改写数据库。正常路径验证旧尝试保留原策略，当前新尝试使用自己的策略，过期租约不被续期。

SARIF 的真实回导增加 CI 上下文全等断言；完整发现、细分严重度、材料身份和只读历史 claim 的原回归继续保留。导入不生成 Native runs、决策、租约或工具授权。

## 下一阶段必须完成的工作

2026-09-27 整体发布后续：第 3 项已以单 JSON 容器实现，不是两个独立按钮连调。真实公共文件/目录回导、双文档一致性与原子发布证据见 `NEST_SOURCE_ATOMIC_BUNDLE_AUDIT.md`；仍不代表远端制品上传或整个 CI 完成。第 4/5 项继续保留。

2026-09-27 后续增量：下列第 1/2 项已经由 `NEST_SOURCE_JOINT_IMPORT_AUDIT.md` 记录的共同解析、作用域归并及真实回归补齐；旧来源迁移只覆盖精确原始字节/身份匹配，歧义仍拒绝。第 3～5 项继续保留。以下清单保留本审计形成时的状态，不应据此重复实现共同导入。

1. **跨格式身份和作用域**：当前 JSON 显式快照导入使用记录身份及任务/尝试；普通 SARIF 目录导入使用通用 fingerprint 和目录作用域。不能只让 ruleId 看起来相同就宣称联合导入归并完成。
2. **共同导入验收**：使用真实 Reviewer 执行输出，验证同一发现 JSON+SARIF 只出现一次，不同任务/尝试/裁决不误合并；来源、冲突、原文全部保留；重排/重复导入幂等；新旧尝试不相互复活已撤销发现；伪造来源不能获得 Native 权限。
3. **多文件 bundle 发布**：如果增加“一次导出 JSON+SARIF”，需要整体完成后发布的目录或容器格式；当前两个独立按钮的单文件原子写入不能冒充多文件事务。
4. **总体覆盖独立审查**：完成实际 coverage Reviewer 执行及证据闭环后再消除缺口。不能为门禁变绿而直接删除 gap。
5. **其他总目标**：费用账本、未知调用和崩溃/并发恢复、其他 Findings 消费者、大规模审计性能、Strix 全引用和跨平台打包证明、用户指令/聊天/沙箱及完整 Master Plan 验收继续保留。

本轮只使用本地数据库、冻结文件和 localhost 模型夹具；没有访问用户目标 URL、外部模型或远端 Worker，没有部署或启用 Host Agent，没有删除用户文件、提交或推送。

## 收尾记录

- 装箱后源码复测 `/tmp/oviraptor-ci-export-final2.log`：13 项通过，48.78 秒。
- 退役检查首次误用 `backend_retirement` 过滤器，匹配零项，不计入验收；改用 `retirement_literal_`，`/tmp/oviraptor-ci-export-retirement-final.log` 实际 4 项通过，0.42 秒。本轮未修改或扩展退役白名单。
- residual `/tmp/oviraptor-ci-export-residual.log`：3 项通过，0.69 秒，含基线与扫描器夹具。
- 最终全 target/all features 严格 Clippy `/tmp/oviraptor-ci-export-clippy-final.log`：exit 0；fmt `/tmp/oviraptor-ci-export-fmt-final.log`：exit 0；git diff --check：通过。
- 所有后台验证会话均已收取终态；收尾进程表无 cargo/rustc/oviraptor_lib 残留。
- Cargo 始终单作业、测试单线程、nice 15；这不是 CPU 硬限制。本轮未运行 Rust 全量，不能用此前全量替代当前版本全量验收。
