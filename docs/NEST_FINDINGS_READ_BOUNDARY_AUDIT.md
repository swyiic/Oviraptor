# Findings 历史读取边界审计（2026-09-27）

本页是历史快照 IO 后续增量，只覆盖普通 Findings 详情、漏洞任务索引及源码历史行数显示。不代表全部 Findings 消费统一完成，也不代表 Master Plan 整体验收。未修改生产扫描授权、模型预算或任务停止门禁。

## 1. 真实复现

此前 `list_sentinel_findings` 在保存的仓库清单缺少 `lineStats` 时，会读取任务的 `source_path`，重新扫描当前目录并写回清单。打开历史页面或轮询即可改写已保存证据，带 kind 过滤也会触发；只读数据库直接报写入错误。详情与漏洞任务索引未检查删除 tombstone。

proof 模式下，详情使用 Rust 字符串类型与 trim 判定，索引使用 SQLite JSON/字符串转换，导致数字请求 ID、去除空白后相同的请求 ID 等记录在两处资格不一致；损坏的记录 JSON 还可能让整个索引查询失败。详情原先把策略查询/解析失败默认为 breadth，可能扩大显示范围。

保留旧行为提取内部连接接缝后，先跑四项回归，**4/4 在业务断言失败**。日志 `/tmp/oviraptor-findings-read-red.log` 分别记录历史清单被覆盖、只读查询写入、删除记录仍展示、数字 ID 在索引误显示。不是用编译错误代替红测。

前端新增三项真实 SFC 渲染测试，先 **3/3 失败**：缺失统计显示 0、保存的真实统计被父级零值投影覆盖、无效统计被当成零。日志 `/tmp/oviraptor-findings-ui-red.log`；原有正式 Reviewer 展示三项保持通过。

## 2. 读取合同

- 两个公开 Tauri 命令保持原名称和参数，通过 blocking worker 执行 DB 查询，不在异步执行线程上同步读取完整集合。
- 普通详情只读数据库中已保存的行，不再读取源码路径，也不补造清单或行数。源码目录发生变化或被移动，原清单仍可读取。
- 详情的任务存在性、删除标记、策略和记录在同一 deferred 读取事务中查询；调用方已有事务时复用该快照，不提交或回滚调用方事务。未知/已删任务返回空列表，保留原始数据。
- 索引由单条 JOIN 查询提供快照，先按项目和删除标记过滤，再按 `updated_at DESC, id DESC, finding id` 确定顺序；同任务只返回一次。
- 索引和详情复用同一个类型化 proof 显示条件：对照/测试 ID 为去除空白后非空且互不相同的字符串，impact 为非空字符串。单条损坏 JSON 在 proof 模式中不可见，不令 SQLite JSON 表达式中断整个列表。
- 无历史 context 或合法对象中缺失 closure，保留旧 breadth 兼容；显式 breadth/proof 正常处理。损坏 JSON、非对象、未知或错误类型的显式 closure 返回 `findings_policy_invalid`，不静默扩大显示范围。索引中匹配范围内任一相关任务策略损坏会使本次索引读取明确失败，不悄悄丢掉该任务。
- proof 条件只过滤 vulnerability；仓库清单等非漏洞资料仍可读取。

这里的 proof 只是旧 Findings 的**显示条件**，不能证明真实请求存在、证据完整或 Reviewer 确认。正式源码裁决继续走 `native_source_findings_snapshot` 的完整集合审计；普通表、历史 JSON、人工 verdict 或具有这些字段的任意 JSON 都不能借本修改获得执行权、确认权或总体覆盖资格。

授权启动中的 `insert_source_inventory` 保留。新增启动回归使用真实临时仓库，验证发布时写入行数；发布后修改源码，再读取 Findings，原清单与全部 Native 表保持不变。不是通过取消正常清单采集来消除读取副作用。

## 3. UI 合同

源码结果页从保存的 `sourceInventory.lineStats` 取值，不信任父级旧的零值强制转换投影。计数必须是非负安全整数；缺失、字符串、布尔、负数、小数、非有限值、超出安全整数范围或其他错误类型显示“未记录”。真实零值仍显示 0。

语言列表的代码/注释/空白/物理行采用相同判定。切换清单后重算，不保留上一清单的统计或跳过文件数。页面说明查看历史不会重新读取当前源码目录；需要更新时使用新的授权任务，没有增加绕过准入的“读取时重扫”按钮。

正式审查组件、所选轮次、导出参数及历史折叠分区保持原边界。这里是实际 SFC 自定义渲染器测试，非桌面 WebView/IPC 验证。

## 4. 验证证据

所有 Cargo 命令串行，低优先级、单作业；测试使用单测试线程。未重跑约十一分钟 Rust 全量，不把之前全量当作当前全量证明。这些执行设置不是 CPU 硬限额。

| 检查 | 本次结果 / 日志 |
| --- | --- |
| Findings 扩展回归 | 7/7，`/tmp/oviraptor-findings-read-extended.log` |
| 授权启动回归（含真实清单断言） | 11/11，`/tmp/oviraptor-findings-startup.log` |
| 正式源码发现读取和导出 | 8/8，`/tmp/oviraptor-findings-source.log` |
| 历史快照导入 | 11/11，`/tmp/oviraptor-findings-import.log` |
| 结果同步只读隔离 | 7/7，`/tmp/oviraptor-findings-sync.log` |
| 源码结果页实际渲染 | 6/6，`/tmp/oviraptor-findings-ui-green.log` |
| 完整前端组件/API 测试 | 186/186，`/tmp/oviraptor-findings-ui-full.log` |
| 类型检查 / 生产构建 | exit 0，`/tmp/oviraptor-findings-build.log`；主 JS 843.35 kB，拆包警告仍在 |
| 严格 Clippy | `--all-targets --all-features -- -D warnings` exit 0，`/tmp/oviraptor-findings-clippy.log` |
| 退役字面量 / 活残留守卫 | 4/4 + 1/1，`/tmp/oviraptor-findings-retirement.log`、`/tmp/oviraptor-findings-residual.log`；59 个登记摘要匹配，无新增或扩大豁免 |

后端额外覆盖真实 `SQLITE_OPEN_READ_ONLY` 连接和调用方已有事务、项目隔离、稳定排序、去重、未知任务、删除保留字节、损坏策略及缺失策略的区别。使用所有 Native 表快照比较，不只比较 Findings 行数。

fmt、`git diff --check` 及本轮未跟踪文本的尾空白检查通过。上述命令均已收取终态，收尾进程表未发现测试、Cargo、rustc 或 Clippy 残留。本轮没有完整 CPU 峰值记录，不据少量采样承诺资源硬上限。

## 5. 剩余范围

概览 Reviewer 确认 KPI 仍只消费 Web 候选/审查账本，尚未统一正式源码裁决；其他 Findings 消费者、SARIF 无损往返和大量结果的分页/性能仍需继续审查。不能将本页的显示资格当作统一确认模型。

独立总体覆盖审查、初评/工具恢复、过期租约与未知效果人工核对、真实 OS 强杀恢复、美元用量、跨平台打包、真实 WebView/IPC、完整角色/聊天/沙箱/工具供应验收及整个 Master Plan 仍未完成。未访问用户目标 URL、外部模型或远端 Worker；未部署、启用 Host Agent、提交或推送。
