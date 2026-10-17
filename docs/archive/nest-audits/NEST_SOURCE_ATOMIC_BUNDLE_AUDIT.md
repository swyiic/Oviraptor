# 源码 JSON/SARIF 整体发布与回导审计（2026-09-27）

## 1. 已交付范围

源码独立审查页新增「导出完整审查包（JSON + SARIF）」。后端通过一次数据库读取事务重新核验本轮材料、裁决、冻结 CI 策略和门禁，生成一份 JSON 报告及其 SARIF 表示，装入单个 JSON 容器后整体发布。保留原有单独 JSON/SARIF 导出。

历史 JSON 文件选择入口、显式 JSON 导入服务和历史目录导入均能回读该容器；发现归并为一条候选，两个报告摘要与原始容器保留。导入仍是 `historical_external/unreviewed/readOnly`，不创建 Native runs、Reviewer decisions、lease 或执行权限。

这是正式审查快照的**单容器整体发布**，不是两个按钮连续调用，也不是目录里两个文件分别写成功。没有新增远端 CI 制品上传或自动部署。未完成整个 CI、总体 coverage Reviewer 或 Master Plan。

## 2. 格式合同

文件名为 `source-review-bundle-<UUID>.json`，名称不包含扫描标识、项目名或用户输入。格式为：

```json
{
  "format": "oviraptor-source-review-bundle-v1",
  "schemaVersion": 1,
  "digestEncoding": "oviraptor-canonical-json-v1",
  "executionEligible": false,
  "manifest": {
    "json": {"name": "source-review.json", "bytes": 0, "sha256": "<digest>"},
    "sarif": {"name": "source-review.sarif", "bytes": 0, "sha256": "<digest>"}
  },
  "documents": {
    "json": {"format": "oviraptor-source-review-v1"},
    "sarif": {"version": "2.1.0"}
  }
}
```

上例仅表示结构，省略必要报告内容，`bytes: 0` 和摘要占位符不可导入。

摘要定义：按仓库 `canonical_json` 的稳定紧凑 JSON 编码分别序列化两份内嵌文档，UTF-8 字节长度写入 `bytes`、SHA-256 写入 `sha256`。对象键按当前序列化实现排序，数组顺序保留；不宣称符合其他 JSON canonicalization 标准，也不是对容器缩进后的字节计算。原始完整容器仍按收到的字节保存。

`documents` 与 `manifest` 各只接受 `json`、`sarif` 两项；内部名称固定，不是允许写入任意路径的解压指令。没有压缩、脚本、任意文件解包或联网读取。

摘要只检查内容完整性，不证明来源可信。即使文件写 `executionEligible: true` 或伪造 confirmed，仍不取得当前确认资格和执行权。

## 3. 共同快照与一致性

- JSON/SARIF 来自同一个已核验 `review` 对象和同一个导出时间，不分别调用两次数据库快照读取。
- SARIF 只接受一个原生 run，其报告摘要必须与 JSON 去除 findings 后完全相同：包括 scan/attempt/root/material、导出时间、冻结策略、CI gate 和覆盖缺口。
- 两份候选数组必须同数量、同顺序、同完整 sourceFinding；不允许重算摘要后混入另一任务或另一个版本的候选。
- 之后仍分别经过原生 source report 的 schema、裁决 ID、行归属、confirmed 数量、SARIF kind/level/rule/fingerprint 等校验。容器摘要验证不能替代报告语义校验。
- JSON 和 SARIF 来源指针分别为 `/documents/json/...`、`/documents/sarif/...`，可以在原始容器中定位；候选沿用现有完整逻辑身份归并，两份 RunState 摘要单独保留。
- 记录上限计算包含两份输入，先计数再归并；目录中的普通 SARIF、其他原生报告和多个容器共同占用总记录预算。

## 4. 发布与失败边界

共用 `snapshot_export::write_json`：先生成完整容器，独占创建同目录临时文件，完整写入并同步后通过不覆盖的 hard link 发布，清理本次临时文件并同步目录。Unix 新文件权限为 `0600`。

写入失败时不发布部分报告；已有普通文件或符号链接不被覆盖。发布后同步/清理失败返回 `durability_unknown`，不删除已完整发布的最终文件，也不承诺没有生成文件。该机制沿用原发布器；Windows ACL、跨平台硬链接支持和真实掉电/强杀恢复仍需另行验收，失败不能回退到覆盖写。

输出容器在发布前检查默认导入上限：32 MiB 文件、JSON 深度及两份输入的记录总量。超限受控报错，不把不能按默认合同回读的完整包静默交付。单独导出按钮继续保留，不自动切换来掩盖整包失败。

输入缺成员、超版本、摘要/大小/名称不符、内容不一致、原生报告语义错误或截断时，整个容器在 CAS/投影写入前失败。目录中其他文件不因为自身有效而让不完整原生容器被接受。

整包发布不消除 `source_coverage_review` 缺口。完成任务、旧 attempt、过期租约可以只读导出；其权限不会被续期或恢复。损坏裁决、冻结材料、缺策略或删除记录必须拒绝发布。

## 5. 实际接线

- `src-tauri/src/commands/native_source_findings.rs`：IPC 新增 `format: "bundle"`，同快照构建、上限检查和发布。
- `src-tauri/src/artifact_import/report_bundle.rs`：版本化容器、摘要与双文档一致性。
- `src-tauri/src/artifact_import/adapters/source_report.rs`：成员来源指针、共同身份、合并前计数，目录识别。
- `src-tauri/src/artifact_import/adapters/sentinel_bundle.rs`：公共显式 JSON 导入分流。
- `src-tauri/src/snapshot_export.rs`：新增固定文件种类，复用原子不覆盖发布器。
- `src/features/sentinel/api.ts` 与 `SourceReviewEvidence.vue`：真实格式参数、同一导出锁、仅审查可核验时提供按钮。
- `HistoricalJsonImport.vue`：支持格式与失败提示更新。回读走原公共文件选择及后端文件读取路径。

本次没有扩大退役白名单，也没有恢复 Strix 运行时。Adapter version 保持 3：新格式此前无法成功导入，不存在旧成功签名需要失效的情况；原有报告身份与历史解释保持不变。

## 6. 红绿测试与最终证据

日志为本机 `/tmp` 文件，不替代发布时的永久 CI 制品归档。

| 检查 | 结果 | 日志 |
| --- | --- | --- |
| 后端初始红测 | 失败：IPC enum 只接受 json/sarif，不接受 bundle | `/tmp/oviraptor-source-bundle-red.log` |
| 实际组件初始红测 | 12 过/1 失败：没有整包按钮 | `/tmp/oviraptor-source-bundle-ui-red.log` |
| 真实 Reviewer 导出→两类回导 | 1 通过 | `/tmp/oviraptor-source-bundle-green.log` |
| 历史导入器 | 72 通过，22.75 秒 | `/tmp/oviraptor-source-bundle-import.log` |
| 源码结果/导出 | 15 通过，56.78 秒 | `/tmp/oviraptor-source-bundle-findings.log` |
| 补充公共文件路径导入复测 | 1 通过，3.30 秒 | `/tmp/oviraptor-source-bundle-public-import.log` |
| 共同原子发布器 | 5 通过 | `/tmp/oviraptor-source-bundle-publication.log` |
| 实际组件/API 专项 | 19 通过 | `/tmp/oviraptor-source-bundle-ui-green.log` |
| 全部现有 UI 测试 | 193 通过 | `/tmp/oviraptor-source-bundle-ui-full.log` |
| vue-tsc 与 Vite build | 通过，主 JS 846.32 kB 分包告警保留 | `/tmp/oviraptor-source-bundle-build.log` |
| 全 target/all features 严格 Clippy | exit 0 | `/tmp/oviraptor-source-bundle-clippy.log` |
| 退役字面量守卫 | 4 通过 | `/tmp/oviraptor-source-bundle-retirement.log` |
| residual 基线 | 1 通过，不是此前其他轮次的 3 项 | `/tmp/oviraptor-source-bundle-residual.log` |
| fmt | exit 0 | `/tmp/oviraptor-source-bundle-fmt.log` |

新增三项导入测试分别覆盖：14 类异常在显式/目录两条路径整包无写入；多容器/普通 SARIF 混合的去重前上限；非空/零发现摘要、来源指针、历史权限与幂等。真实源码回归增加 completed/historical attempt 整包导出、六类账本故障拒绝整包发布、真实零确认整包输出。嵌套场景不是独立测试数量。

测试始终单 Cargo、`nice -n 15`、`-j 1`、`--test-threads=1`；不是 CPU 硬限制。所有本轮后台会话已收取终态，检查时无 cargo/rustc/oviraptor_lib 残留。未跑整个 Rust 全量、真实 WebView/IPC、Windows/macOS 分发包或外部目标验收；不能用前轮全量代替。未访问用户 URL、外部模型或远端 Worker，未部署、启用主机执行、提交或推送。

## 7. 剩余工作

下一项应继续处理独立总体源码覆盖审查：真实 assignment/child-run、冻结覆盖材料、独立 Reviewer 模型调用、mailbox/ACK 和不可变裁决消费，不得直接将当前 `coverageReviewCompleted: false` 改成 true。

完整阶段恢复、未知调用/租约过期核对、真实强杀重启/并发、美元账本、大规模审计性能、其他 Findings 消费者、用户指令/聊天端到端、Strix 全引用与跨平台发布，以及 Master Plan 其余要求仍待验收。本增量不勾选整个项目完成。
