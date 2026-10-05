# 原生源码 JSON/SARIF 联合历史导入审计（2026-09-27）

## 结论与范围

真实 Reviewer 导出的 JSON/SARIF 共同导入已补齐：两份原文保留，按原生报告身份归并，不能提升为 Native 执行或确认权。当前是跨格式历史导入增量，不是完整 CI bundle 发布、源码总体覆盖审查或 Master Plan 全部完成。

## 实现

- 新增 `artifact_import/adapters/source_report.rs`，直接 JSON 与目录导入共用原生报告解析。精确识别 `oviraptor-source-review-v1`；目录仅发现保留的 `source-review.json` / `source-review-*.json` 名称，不泛化任意 JSON。
- 校验 schema、scan/attempt/root/material、候选与裁决身份、确认数量、重复 ID、审查状态；SARIF 同时验证 kind/level/rule/fingerprint 与完整 sourceFinding 一致。错误原生报告使整包失败，不回退通用路径。
- 共同身份绑定格式、scan、attempt、root、material、decision；跨格式同一裁决保留多来源，内容不同保留冲突。每来源保留运行摘要；SARIF 原始扩展与 CI 信息保留。
- 混合目录的通用 SARIF 与原生报告分流；原生同 scan/attempt 一次 reconcile，新 attempt 优先，旧 attempt 不复活已撤销记录。输入上限在去重前检查。
- Adapter 版本升到 3；CAS 发布与投影遵循共同导入事务边界。所有导入固定只读历史属性，文件中的授权/执行字段无权签发 Native lease 或审查资格。
- 删除 `sentinel_bundle` 中已经被共同解析器提前返回覆盖的死分支，旧 task/project 路径继续保留。

## 旧 SARIF 归属迁移边界

仅对 adapterVersion < 3、旧 SARIF 当前 membership、同原始文件 hash 且完整原生身份匹配的记录，在共同事务内撤销旧 membership；保留不可变 revision 与原始文件。相同字节改名或移动可识别，不依赖相同路径。

旧 envelope 已有冲突时返回 `source_report_legacy_merge_ambiguous`，整事务回滚，不把可能混入其他记录的旧合并静默撤掉。不同内容 hash 的旧来源、已有歧义的旧库不属于已验收全局迁移范围，需另案处理。

## 红测、修复与验证

以下日志位于本机 `/tmp`，不是永久 CI 归档；发布前应将需要保留的证据存入正式制品。

| 验证 | 结果 | 本机日志 |
| --- | --- | --- |
| 真实 JSON+SARIF 初始红测 | 失败：目录只发现 1/2 文件 | `/tmp/oviraptor-joint-import-red.log` |
| 真实联合导入复测 | 1 通过 | `/tmp/oviraptor-joint-import-green.log` |
| 导入套件中间失败 | 测试误用 Diagnostic.message；改为 detail | `/tmp/oviraptor-source-report-import-suite.log` |
| 迁移歧义红测 | 68 通过/1 失败；暴露 recordKind 读取错误 | `/tmp/oviraptor-source-report-import-suite-final.log` |
| 修正后历史导入 | 69 通过，19.08 秒 | `/tmp/oviraptor-source-report-import-suite-green.log` |
| 真实源码发现/导出 | 14 通过，53.68 秒 | `/tmp/oviraptor-source-report-real-exports.log` |
| 公共 bundle 导入（死分支清理后） | 11 通过，4.58 秒 | `/tmp/oviraptor-source-report-bundle-import.log` |
| 全 targets/features 严格 Clippy | exit 0 | `/tmp/oviraptor-source-report-clippy.log` |
| 退役字面量守卫 | 4 通过，session 70841 exit 0 已收取 | `/tmp/oviraptor-source-report-retirement.log` |
| 残留基线精确 filter `strix_residual_` | 1 通过，session 51665 exit 0 | `/tmp/oviraptor-source-report-residual-final.log` |

新增六项原生报告测试覆盖多任务/尝试/根隔离、普通 SARIF 混合、十类坏输入无写入、去重前 cap、同身份冲突、四类旧归属迁移、删除不复活及零发现摘要。不要把嵌套场景数算成独立 Rust 测试数，也不要把此次 residual 的 1 项说成以前轮次的 3 项。

退役登记仅更新四个既有文件的审核摘要和说明，未扩大白名单范围，未增加 Strix 运行路径。没有修改 UI，未重跑 UI/build 或 Rust 全量；此前全量不代表本次版本全量验收。

Cargo 验证采用 `nice -n 15`、`-j 1`、`--test-threads=1`，不是 CPU 硬限制。未访问外部目标 URL、模型或远端 Worker；未部署、启用主机 Agent、提交或推送。

## 仍需继续

后续整体发布已补齐：`NEST_SOURCE_ATOMIC_BUNDLE_AUDIT.md` 新增同快照 JSON/SARIF 单容器、不覆盖原子发布和公共回导验收。下段多文件发布是本审计形成时的状态；其余缺口继续保留。

多文件 bundle 整体发布、总体 source_coverage_review、完整阶段恢复/未知结果/过期 lease/进程强杀与并发恢复、美元账本、规模化性能、其他 Findings 消费者、Strix 全引用/跨平台打包，以及 Master Plan 其余验收仍未完成。
