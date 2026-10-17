# 历史目录快照与 JSON 文件发布审计（2026-09-27）

本页是 `NEST_JSON_SNAPSHOT_IMPORT_AUDIT.md` 后续增量，不代表整个 Master Plan 或跨平台验收已完成。修改只涉及本地历史资料导入和报告导出，不修改扫描授权、任务预算或模型执行门禁。

## 1. 实际复现的缺陷

目录导入的 manifest 先读取文件计算摘要，随后再次读取 payload；两次读取之间源文件变化会导致摘要与入库原文不一致，第二次读取错误还会被替换为空字节。原大小检查依赖读取前 metadata，文件增长可绕过单文件和总量限制；文件数量限制检查过晚，规范化后重复路径也未拒绝。

保留原行为提取 reader 测试接缝，先运行五项断言，五项均失败：`/tmp/oviraptor-manifest-snapshot-red.log`。它们分别证明摘要/payload 不一致、单文件增长越界、总量增长越界、文件数量检查晚于读取、规范化路径重复。

任务 JSON 导出使用 `sentinel-{scan_id}.json` 覆盖写入，既覆盖前次结果，也跟随已有同名符号链接修改其目标。项目导出使用项目名和秒级时间戳，存在碰撞及超长文件名风险。不透明的任务 ID 被当作文件路径片段。

三项导出红测均失败：`/tmp/oviraptor-snapshot-export-red.log`。真实复现重复导出覆盖、符号链接目标被改写，以及带路径片段的 ID 无法安全导出。所有红测仅操作测试专属临时目录，没有改写用户真实报告。

## 2. 目录导入修复合同

- 文件数量和全部规范化路径先检查，再读取 payload。重复规范化路径整体拒绝；路径必须无损 UTF-8 编码，不允许控制字符插入 manifest 分隔符。有效普通路径继续使用原有 NFC 规范化及内容 ID 算法。
- 每个文件只读取一次，摘要和 payload 使用同一组实际捕获字节；读取失败产生诊断，不生成空原文替代品。
- 每次读取最多单文件上限与剩余包预算较小值再加一字节。实际读取字节计入预算，额外一字节用于拒绝超限，不截断后假装成功。
- 打开前拒绝非普通文件，Unix 使用 `O_NOFOLLOW | O_NONBLOCK`；打开后再检查文件句柄类型，拒绝符号链接和 FIFO。Windows 原生文件竞态尚未验收。
- 新增九项定向测试，包括实际有界读取、读取错误、文件增长、文件数量、规范化冲突、控制字符/非 UTF-8 名称、替换符号链接及 FIFO 拒绝。

保证的是“摘要和解析描述同一批捕获字节”，不是整目录同时刻原子快照。不能声称已经防住所有祖先目录交换竞态、并发原地写入或恶意本地主机文件系统操作。

## 3. 导出修复合同

`src-tauri/src/snapshot_export.rs` 是任务包、项目包及正式源码审查报告共用的发布器：

1. 使用固定类型前缀和 UUID，文件名不使用任务 ID、项目名称、目标 URL 等输入。
2. 使用排他创建的同目录临时文件；Unix 创建权限为 `0600`。只有本次成功创建后才启用清理，创建碰撞不会删除既有文件。
3. 完整写入并同步文件后，以同目录 hard link 发布最终路径。既有普通文件或符号链接都导致发布拒绝，不用覆盖 rename 作为回退。
4. 发布后移除本次临时文件，Unix 同步目录。已知发布前写入失败会清理本次临时文件；最终报告路径不会暴露半成品。
5. 发布后清理/目录同步失败返回 `durability_unknown`，不声称成功，也不删除已经完整发布的报告。此时人工检查输出目录；重试可能产生另一份报告。

不支持 hard link 的文件系统明确失败，不偷偷退回覆盖写入。这里没有承诺跨平台 ACL 保密，也没有承诺进程强杀后的无残留恢复；崩溃可能遗留本次临时文件，尚无自动清理器。祖先目录交换和外部攻击者同时修改输出目录的完整防御也不在本轮证明内。

任务和项目导出改为 blocking worker，IPC 名称及参数保持兼容。各自全部集合在同一 SQLite 只读事务中取得，再结束事务并发布文件，避免混合不同提交版本。源码报告继续通过原有整集合审计事务取得正式裁决，不能直接导出分页缓存。

任务/项目包仍是历史格式，明确 `qualification: historical_snapshot` 和 `executionEligible: false`。正式源码报告仍为 `verified_at_export`，不声称总体覆盖完成；重新导入任何格式都只进入未复核、只读历史记录，不回写 Native 权威表。

旧任务/项目导出为兼容保留原始资料，可能包含凭据；`0600` 是访问权限，不是加密或万能脱敏。不得将这些原始包默认当作可公开分享报告。需要跨设备交换敏感原文时，后续仍需独立的加密交付和明确 UI 提示设计。

## 4. 当前验证与边界

所有 Cargo 串行运行，使用 `nice -n 15`、`-j 1`，测试另用 `--test-threads=1`；这些不是 CPU 硬限额。不重跑约十一分钟的 Rust 全量，不引用之前全量通过作为当前修改的全量证明。

| 检查 | 当前证据 |
| --- | --- |
| 导出首轮八项 | 8/8，`/tmp/oviraptor-snapshot-export-green.log` |
| 扩展真实文件往返与错误保护 | 10/10，`/tmp/oviraptor-snapshot-export-extended.log` |
| 公共历史导入 | 11/11，`/tmp/oviraptor-snapshot-io-import.log` |
| 正式源码发现读取/导出 | 8/8，`/tmp/oviraptor-snapshot-io-source.log` |
| 共用目录导入器最终回归 | 70/70，`/tmp/oviraptor-snapshot-io-manifest-final.log`；不是独立 importer binary 的全量 |
| 首轮严格 Clippy | 因新增测试的一处不必要 clone 拒绝；`/tmp/oviraptor-snapshot-io-clippy.log`，保留失败记录 |
| 修正后的严格 Clippy | `--all-targets --all-features -- -D warnings` exit 0；`/tmp/oviraptor-snapshot-io-clippy-final.log`，未加 lint 豁免 |
| 退役守卫 | 字面量 4/4、活残留基线 1/1；`/tmp/oviraptor-snapshot-io-retirement.log`、`/tmp/oviraptor-snapshot-io-residual.log` |
| 完整前端组件/API 测试 | 183/183，`/tmp/oviraptor-snapshot-io-ui.log`；不等于真实 WebView/IPC |
| 类型检查及生产构建 | exit 0，`/tmp/oviraptor-snapshot-io-build.log`；主 JS 843.08 kB，保留拆包警告 |
| 格式、空白及登记摘要 | fmt/diff 检查通过，59 个登记摘要均匹配；未增加宽泛豁免 |

十项导出测试覆盖最终路径发布前不可见半成品、写入失败清理、普通文件和符号链接碰撞、并发不同完整文件、Unix 权限、重复任务/项目导出、长名称/不透明 ID、缺失记录与输出目录被文件占用、真实任务/项目文件往返原字节不变且正式表不变。

只重新审核并更新历史字面量登记中实际变更的 `commands/appsec_validation.rs` 摘要和说明；其唯一旧阶段字面量仍只用于历史来源分类，没有新增活动运行时例外。

上述所有命令已收取终态；收尾进程表没有残留的测试、Cargo、rustc 或 Clippy 进程。没有以本轮少量采样推导 CPU 硬上限，也没有再启动全量 Rust 测试。

本轮未访问授权 URL、外部模型、远端 Worker，未部署或启用 Host Agent。真实 WebView/IPC、打包跨平台、OS 强杀恢复、其他 Findings/SARIF、独立总体覆盖、工具/模型恢复及未知效果人工处理、美元账本和完整聊天/沙箱/工具供应验收仍按 Master Plan 继续。
