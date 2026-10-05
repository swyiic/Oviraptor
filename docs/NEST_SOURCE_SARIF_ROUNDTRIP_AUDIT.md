# 正式源码审查 SARIF 导出与历史回导审计（2026-09-27）

## 本轮结论

源码审查页新增 SARIF 导出入口，原 JSON 默认调用保持兼容。两种格式共同使用整套账本核验，不能靠页面缓存、分页数据或导入文件里的 confirmed 字段获得导出资格。SARIF 回导保留正式审查的来源信息，但只成为历史记录，不恢复 Native 执行权或 Reviewer 权威。

同时修复 SARIF 导入器的两个真实缺陷：标准顶层 kind 被忽略导致非失败记录成为漏洞候选；标准复数 codeFlows 被忽略导致分析路径丢失。历史 properties.kind 和单数 codeFlow 仍兼容。

这不是整个 Master Plan 或完整 CI-006 的验收。未访问用户目标 URL、远端 Worker 或外部模型；没有部署、启用 Host Agent 或放宽生产授权门禁。

## 实际接线

- SourceReviewEvidence 的 JSON/SARIF 按钮共享导出锁，只向后端发送任务、轮次和格式，不发送已加载的发现。
- export_native_source_findings 增加可选 format（json / sarif）。缺省仍为 JSON；其他格式在反序列化阶段被拒绝。
- 两种格式共用 native_source_findings_snapshot 和 opaque SourceDecisionSet 的完整审计，核对整套裁决、材料、Reviewer 回执与 ACK；分页之外的损坏也拒绝导出。
- SARIF 使用同一个不覆盖发布器：UUID 文件名、同目录临时文件、完整写入并同步后发布，Unix 私有权限，扩展名为 .sarif。重复导出不覆盖已有文件。
- 导出只包含正式确认发现；已排除／证据不足的数量保留在运行摘要中。零确认仍保存 audited 摘要，不等于无审查或系统无漏洞。
- 每条结果保留原 sourceDecisionId、candidateDigest、Reviewer、材料摘要、证据引用、理由、置信度及原始路径；展示级 SARIF location 对空路径不制造位置，对无效行号不制造 region，对 URI 特殊字符编码。
- 运行摘要只保存一次，不嵌套重复 findings，避免导入时按“发现数 × 全报告大小”复制。
- 保留 verified_at_export、executionEligible=false、coverageReviewCompleted=false。此文件不是可转移执行授权，也不声明总体覆盖或 CI 已通过。

## 历史导入语义

1. 优先读取标准 kind；仅缺失时回退历史 properties.kind。pass、notApplicable、open、review、informational、未知／格式错误的 kind 均不成为漏洞候选。历史缺省 kind 的 warning/error 行为保留。
2. 优先读取标准 codeFlows，缺失时回退历史 codeFlow；完整 result 另存历史 extension，保留多位置、relatedLocations、厂商字段及未映射数据。标准字段不会被旧扩展覆盖。
3. properties.severity 仅接受 critical/high/medium/low/informational；未知值按原 level 映射。导出高危以外的严重度不会因 SARIF warning 统一降成 medium。
4. 带 properties 的运行单独保留一条历史 RunState（status=imported），包括零结果报告。tool 和运行属性不按每条发现重复复制。
5. 原文继续由公共导入服务按字节存储／在识别到凭据时密封；没有执行导出文件内的路径、命令或权限声明。
6. 真实往返验证比较所有非 import_* / artifact_objects 的表，证明导入不创建或改写 Native 任务、租约、授权、回执与正式裁决；历史展示仍为 readOnly / executionEligible=false / unreviewed。

## 已收取的验证

| 检查 | 结果 | 本地日志 |
| --- | --- | --- |
| 标准 kind/codeFlows 红测 | 2 项按预期失败；错误候选数 6 而非 1，路径错误选中旧扩展 | /tmp/oviraptor-sarif-standard-red.log |
| 正式源码读取／JSON／SARIF | 11 项通过；含真实多条正式发现往返、零确认、损坏拒绝、无覆盖写入、格式和 URI 序列化 | /tmp/oviraptor-source-sarif-final.log |
| 历史导入器 | 63 项通过；含原有历史兼容及新增 3 项标准语义／摘要测试 | /tmp/oviraptor-sarif-import-final.log |
| 共用不覆盖发布器 | 5 项通过 | /tmp/oviraptor-sarif-writer.log |
| 实际 SFC 导出组件／IPC | 12 项通过 | /tmp/oviraptor-source-sarif-ui.log |
| 完整 UI 测试 | 192 项通过 | /tmp/oviraptor-sarif-ui-full.log |
| 类型检查与生产构建 | 通过；主 JS 845.67 kB，超过 500 kB 的拆包警告仍在 | /tmp/oviraptor-sarif-build.log |

收尾已收取：严格 Clippy（all-targets/all-features、-D warnings）及 fmt 通过；退役字面量 4 项、活残留基线 1 项、既有 JSON 真实源码报告往返 1 项全部通过。日志分别为 /tmp/oviraptor-sarif-clippy.log、/tmp/oviraptor-sarif-fmt.log、/tmp/oviraptor-sarif-retirement-green.log、/tmp/oviraptor-sarif-residual.log、/tmp/oviraptor-sarif-json-roundtrip.log。所有启动的工具会话均已收取终态；git diff --check 通过。

真实失败保留：新增源码测试首轮使用了私有 service 模块，次轮使用了不存在的 summary.bundles 字段，均编译失败而非业务断言失败；已改为公开 re-export 与实际 outcomes 字段。日志为 /tmp/oviraptor-source-sarif.log 和 /tmp/oviraptor-source-sarif-green.log。随后 11 项通过日志为 /tmp/oviraptor-source-sarif-green2.log；最终元数据布局调整后再次 11 项通过，以 final 日志为准。

每次仅一个低优先级 Cargo、-j 1、--test-threads=1。没有重跑约十分钟的 Rust 全量；不将旧全量或此次定向测试冒充当前全项目全绿。没有 CPU 硬限额或全程峰值监测承诺。

## 退役边界复核

仅复核并更新 SARIF 历史适配器和原有格式测试文件两个登记哈希，分别保留 3 处历史字面量以及 importer / fixture 分类；没有新增运行时依赖、启动入口或退役白名单路径。新增功能调用原生读账本与历史导入服务。退役守卫首轮 3 通过／1 失败，准确指出 tests_import_formats.rs 的登记哈希遗漏；语义复核后补齐，再跑 4/4 通过，未放宽检查。首轮日志保留在 /tmp/oviraptor-sarif-retirement.log。

## 尚未完成，后续顺序

1. **生产 CI 报告而非 Findings 快照**：当前新增的是正式源码审查报告，不是完整 CI bundle。生产 source_ci 的冻结 head/base/tree/rule-pack/analyzer 与门禁结果需要同事务接入正式 JSON/SARIF CI 输出；不能沿用仅测试入口的 Web GateReport 作为生产源码权威。
2. **完整 CI-006**：本轮验证 SARIF 的真实源码往返；既有 JSON 单独往返保留。JSON 与 SARIF 同时导入时跨格式身份归并、完整 CI 冻结字段与总体覆盖结论的无损往返，仍须单独验收，不能由旧合成 CI-006 测试替代。
3. **总体覆盖独立审查**：仍缺 source_coverage_review。候选审查完成不能据此把 CI 改成 passed。
4. **产品端到端与性能**：未验证真实 WebView 的文件操作及大规模导出峰值；当前仍整套审计／序列化。SARIF 走现有历史目录导入链路，任务中心的单文件 JSON 选择入口并未扩展为 SARIF 文件选择器。
5. **其余总目标**：其他 Findings 消费者、完整崩溃／并发恢复、费用账本、打包跨平台、部署与全 Master Plan 验收继续保留，不能因本增量通过而标记完成。
