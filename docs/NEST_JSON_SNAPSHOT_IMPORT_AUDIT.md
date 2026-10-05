# 历史 JSON 公共入口隔离、真实报告往返与任务中心导入审计

## 最新追加：SARIF 专属兼容和任务 ID 别名退役

通用 SARIF 不再读取旧 coverage 属性、旧规则名前缀或 `properties.kind`，也不再向 canonical payload 提升 `coverage_outcome`。顶层 kind／level 决定分类；warning／error 的缺省或显式 fail 为未审核候选，非 fail／无效 kind 和非发现级别保持非候选。原始 properties 仅保留为不解释的扩展证据，不是历史兼容，也不能贡献执行授权或 Native 确认。

adapter version 从 3 升为 4，防止同一文件的旧签名直接返回 unchanged。测试种入 v3 的旧分类和当前 membership，再导入同一源文件，验证旧 membership 退出当前视图、新候选只读未审核、原 revision 字节不变、再次导入幂等。这里的重解析发生在**下次导入**，不是后台全库扫描或自动清理；未触碰真实用户数据库、CAS 和源报告。源码报告专有的 `<3` 归属修正阈值未机械改成 4。

导入删除判断、任务预览、目录列表和 bundle 预览均改为精确 scan-id：旧前缀不能建立关联，旧别名的删除标记不能隐藏无关当前任务；精确 ID 的删除保护不变。现有相同内容／不同任务隔离测试转为当前身份，负向测试明确拒绝旧别名，即使目录相同也不能关联。

本批最终测试、失败记录和 52 项剩余登记见实施进度顶部。以下旧批次的正向兼容描述不是恢复要求；recon reader／目录回退、skills／knowledge 格式别名、迁移和真实数据清理仍未完成。日志／对话实时链路不因导入层清理而自动完成。

## 最新追加：停止解释已持久化的旧轨迹

停止导入旧文件还不足以退役：`historical_agent_trace.rs` 现已删除旧 run／session／message／tool／usage 解释、快照用量协调和旧任务 ID 别名。当前读取要求 row 与 envelope 同为 `model_audit`／`event_trace`，且为只读、未复核、不可执行的当前模型／提示词审计；未知 trace kind 不再进入消息兜底。旧 adapter 在解析 JSON 前被拒绝，即使存量 payload 损坏也不影响当前审计读取。

attempt 上界从任务范围内的当前 memberships 与任务 attempts 确定，先于格式／claim 过滤，防止最新数据被拒绝后错误显示上一轮审计。精确 scan-id 绑定还验证路径组件归属；任务的 `reports/`／attempt 子目录可读，其他目录、同名前缀、`..` 被拒绝，不重开离线源。旧别名的删除标记不影响无关的当前任务。

删除无剩余业务调用的 6 个旧用量别名函数及旧消息提取函数；通用 token／缓存统计和私有推理过滤用当前 Hook／公共展示入口验证。当前 Native 摘要合同没有删字段，导入审计不虚构 agents／tools／targets。原始 revisions、真实用户库、CAS 与源报告均未做物理清理。剩余登记 55 项；其他旧投影别名、SARIF 属性与迁移仍未完成。当前验证证据及实时轨迹补读修复见实施进度顶部；下方已退役功能的旧批次描述不构成恢复要求。

## 最新追加：外部 SQLite trace 不再识别或导入

旧 `agents.db` reader、派发／发现／优先级和专属 scratch 接口已删除；WAL／SHM 也不进入报告清单或对象仓库。负向用例覆盖正确旧 schema、缺表、缺列、坏行、损坏文件，以及混合目录中只修改旧文件不触发重导。两个无剩余依赖的正向 SQL 样本已删除，剩余 manifest 摘要保持不变。应用内部 SQLite 不在退役范围。

当前 Hook 的坏行保留、session／请求隔离、原文不变和展示隐私继续验证，不能为凑旧消息数量恢复 SQLite 解释。另修复嵌套 JSON 凭据原文虽然加密、展示标志却误判的问题：含凭据原文不生成原始文本预览；结构化脱敏记录仍可阅读。未回填或删除真实存量记录。

本批导入 96、历史 40、展示隐私 6、当前报告 15、CI 回读 12、公共 JSON 包导入 11、导入工具 45、退役守卫 39 项通过，各集合有重叠。最终检查、失败过程和证据以实施进度顶部为准。旧别名、旧 canonical 展示、迁移和存量清理仍未完成；下文 SQLite 正向 reader 描述不再构成支持要求。

## 最新追加：旧漏洞 JSON 读取器已删除

`artifact_import/adapters/legacy_strix.rs` 及对应模块、发现锚点、派发和合并优先级已移除，不新增换名适配器。旧漏洞 JSON 在根目录／嵌套目录中的顶层数组、四种 envelope 及损坏输入均纳入负向退役测试；旧文件不能独立建包，混入当前审计 JSON／SARIF 不进入 manifest／CAS，修改旧文件不能触发当前报告重导。

原文哈希、隐私、限额、坏文件隔离、事务、幂等和公开预览测试改用当前 SARIF，不删除独有断言。HTTP method／parameter 的指纹维度通过 canonical 输入测试，不假设标准 SARIF 解析器会提升任意 properties 字段。正式产品 JSON／SARIF 往返仍由 `source_report` 覆盖，不引入第二种 CI 格式。导入模块 104 项、公开预览 2 项、历史相关 42 项、状态 1 项和相关实时 UI 70 项通过；集合有重叠，最终补测及严格检查见实施进度顶部。

临时残留登记降至 57 项，仍含 SQLite、别名和迁移等未退役依赖。本批没有删除真实库／CAS／源文件，没有外部 URL 测试，没有完整桌面与延迟验收。下文关于旧 JSON reader 或永久兼容的描述仅为历史，不得据此恢复已删除功能。

## 最新追加：旧漏洞文本入口已退役

用户不再要求历史格式兼容。旧漏洞 CSV／Markdown 读取器及 CSV 专属限额分支已经删除；下文 CSV 实现、文件行数和正向验收均为历史记录，不得按其恢复旧功能。通用记录上限、原文完整性、失败回滚及重复导入检查保留，并用当前 SARIF／模型审计格式覆盖。旧文本单独存在不能创建包，混入当前包不进入清单／对象仓库，改变旧文本不能触发当前包重导。当前独立资产 CSV 导入导出不在本次删除范围。

该批导入 105 项和 Native CI 回读 1 项通过；具体证据和限制见 `NEST_IMPLEMENTATION_PROGRESS_2026-09-23.md` 最上方。后续调用链更正：应用正式 Native 导出使用 `oviraptor-source-review-v1`，不是旧漏洞 JSON；已删除的 standalone `write_bundle` 仅被旧 CI-006 调用。复用正式导出与 `source_report` 回读，不新增重复格式。泛化旧 JSON reader、SQLite、旧字段与真实存量清理仍未完成；不要将局部退役标记为整个导入层已经清零。

更新：2026-09-27。此页记录一个增量，不代表 Master Plan 或所有质量门禁已经完成。

## CSV 输入资源边界补充（2026-09-29）

完整门禁终态补充：session **39098** 已收取 **exit 0**，CSV 修复后的主库 **1443/1443**、导入器 **30/30**，无失败或忽略项；日志 `/tmp/oviraptor-csv-sidebar-rust-full.log`。低优先级、离线、单构建作业/单测试线程执行，期间未修改 Rust。本结果补足下文专项运行时尚缺的完整 Rust 证据，不替代安装态、CPU 基准或 Master Plan 验收。

目录导入原先在 CSV 完整解析后才检查 canonical 记录总数：有效行可先分配过量记录，无标题/端点的坏行则只增加诊断、不计入结果记录数。现复用 `Limits.records`，在每个 CSV 的逻辑数据行处理前检查上限，坏行同样计数；引号内换行不额外计数。为确认超限，CSV reader 最多额外读取一行，原文件字节上限仍生效；这不是零额外分配或性能基准证明。

遇到 `csv_input_row_limit`，停止后续适配器派发，并在 source-report 解析、CAS 发布及包提交前拒绝整包，不能把已解析前缀作为完整历史替换旧投影。上限内坏行仍报告并允许健康行导入，超限包不阻塞健康兄弟包。来源登记可能已写入 `import_sources`，因此不声称整个入口没有任何数据库写入。测试验证拒绝时不创建 bundle/revision/object 或活动扫描/发现/agent run，旧投影不变，原文件内容及元数据指纹不变；恢复合法输入后可重试，重复导入保持幂等及只读无执行权限。

实现留在 `artifact_import` 业务内：CSV 适配器 **117 行**、适配器派发 **276 行**、包服务 **217 行**、限制定义 **71 行**。新增 `tests_import_csv_limits.rs` **230 行 / 6 项**，复用既有夹具和测试入口，入口 **292 行**；没有新依赖、新测试命令、全局公共类或重复夹具，未删除独有回归。静态退役审计仅更新两个已审查文件的精确摘要，未扩大例外范围。

修复前六项回归全部失败；修复后历史导入模块 **95/95**、公共导入入口 **11/11**、导入器工具 **30/30**，格式检查、全目标/全特性严格 Clippy、静态退役 **9/9** 与残留检查 **6/6** 通过。证据为 `/tmp/oviraptor-csv-limit-{red-final,green,public-import,importer,clippy,retirement,residual}.log`。完整 Rust 历史基线为 **1437/1437 + 导入器 30/30**，不能作为本次改动后的完整门禁结果；本次不涉及 UI、打包安装或外部目标测试。

边界：本补充只处理 CSV 输入行资源边界，未改变 JSON/SARIF/SQLite 分配策略或提供整个目录的原子快照。未提升 adapter version；签名相同的已导入包仍走既有 `Unchanged` 快径，不宣称重新按新限制校验。该修复不构成先前 360% CPU 的根因诊断，Master Plan 整体验收仍未完成。

## 1. 已确认的问题与修复

旧 `import_sentinel_results` 和 `import_sentinel_project` 直接把 JSON 写回正式任务、发现、验证、检查点和机会表；项目入口还会清除任务删除标记。磁盘历史目录导入器已有的只读约束没有覆盖这两个公共 IPC 入口。不能因目录导入测试通过，就声称所有旧 JSON 都已安全兼容。

先保留原逻辑提取可测试入口，两项真实红测分别复现覆盖活动任务和复活删除任务。随后将公共入口改为共用 canonical 历史快照导入：保留旧资料，不恢复任务或信任来源里的执行/确认声明。

## 2. 格式、数据与权限合同

支持明确标识的五种格式：

- `oviraptor-sentinel-v1` / `asset-atlas-sentinel-v1`：单任务历史包。
- `oviraptor-sentinel-project-v2` / `asset-atlas-sentinel-project-v2`：项目历史包。
- `oviraptor-source-review-v1`：`schemaVersion: 1` 的正式审查导出报告。

所有导入记录均为 `historical_external`、`unreviewed`、`readOnly: true`、`executionEligible: false`。导出时核验过的审查报告，重新导入后也不是当前安装的 Reviewer 权威记录。伪造 `executionEligible`、`coverageReviewCompleted` 或内部确认字段，不能生成 Native decision、assignment、预算、工具或模型回执。

任务状态保留为来源信息，当前历史投影状态为 imported。发现进入候选记录，其余 targets/checkpoints/validations/opportunities/fuseZone 是证据备注，不写回活动表。未知字段保留为 extensions；JSON 中的任务目录、源码目录等仅是数据，不据此读取文件、创建进程或恢复扫描。

多任务项目按 scan scope 处理，所有 scope 在一个数据库事务提交。无扫描的空项目仍可作为历史记录发现；没有任务归属的项目 fuseZone 使用内容摘要派生的独立历史 scope。不同位置的同名 finding key 不合并成一条。重复扫描标识、重复记录身份、跨任务引用、错误数组、负轮次和不支持的格式被拒绝。

导入不是完整数据库备份恢复；原项目/任务包也没有被悄悄升级成正式审查报告。SARIF 的公共选择文件往返入口不在本次交付范围内。

## 3. 文件、事务和并发

- 粘贴内容直接传入共用服务，不生成明文临时 JSON。文件入口接受普通文件，检查大小，Unix 使用 `O_NOFOLLOW | O_NONBLOCK` 后再次核验文件类型；读取最多 limit+1 字节，拒绝读入期间增长导致超限的文件。其他平台的原生文件选择器/竞态验收未完成。
- 默认单文件/包 32 MiB、JSON 深度 24；解析和归属校验先于 CAS 与数据库写入。内容身份按收到的字节计算，不因文件名或导入入口改变。
- 原文按字节保留。识别为含凭据的原文使用现有 sealed CAS 加密；展示文本与 canonical envelope 经过现有脱敏器处理。未宣称能识别所有未知秘密格式。
- 项目第二个 scope 写入失败时，第一个 scope、bundle 和对象账本一起回滚。CAS 文件先于数据库事务发布，失败后可能留下不被台账引用的对象；不能把数据库原子性说成跨文件系统原子提交。本次未增加孤儿对象垃圾回收。
- 同内容并发导入通过既有 CAS 发布与 SQLite 事务协调，测试验证唯一对象、唯一语义修订及投影，密钥赢家一致且可正确解密。
- 旧 attempt 不覆盖新投影；删除标记不会被清除。实际删除完成后，重导入同包、旧包或更高轮次包仍不恢复任务或历史展示。
- 返回数字是本包读取记录数，不是创建任务数，也不保证每条都出现在当前投影；已删除/旧轮次可能仅登记原文。

## 4. 入口与界面

两个 Tauri 公共命令均在 blocking worker 运行文件/数据库工作。远端 Worker 同步也调用同一历史导入服务，移除 `.worker-import-*.json` 明文中转；Worker 同步文案说明只读历史性质。未实际连接远端节点，本次没有证明其网络协议端到端可用。

任务中心“历史产物 · 只读”区域新增“导入历史 JSON”按钮：原生文件选择器、取消不导入、操作中防重复、泛化错误提示避免输出后端路径/凭据。成功仅刷新全局历史台账，不创建或启动任务。

强制刷新会使旧列表和旧预览请求失效；组件卸载后，迟到文件选择不触发导入，迟到提交不刷新已销毁视图。导入已提交但页面已卸载时不会撤销后台成功结果，下次打开历史台账可读取。

UI 测试使用真实父子 SFC 编译、render、事件与生命周期，只替换文件选择器、IPC、图标和无关子组件；不是实际 WebView/Tauri IPC 验收。

## 5. 验证账本

所有 Cargo 命令串行、`nice -n 15`、`-j 1`，测试另用 `--test-threads=1`。这些不是 CPU 硬限额。本增量未重跑约 11 分钟的完整 Rust 测试。

| 验证 | 结果与证据 |
| --- | --- |
| 原逻辑隔离红测 | 2 项失败；真实 Native 状态变化，非测试夹具错误。`/tmp/oviraptor-bundle-import-red.log` |
| 首批修复 | 7/7，`/tmp/oviraptor-bundle-import-green.log`；早于 async/Worker 接线，不用作当前版本证据 |
| async 接线首次编译 | 曾因 Worker 同步调用 async 命令产生 E0277；已改为直接调用共用字节服务。旧失败日志路径后被复测覆盖，不声称还保留独立原始失败文件 |
| 当前公共导入专项 | 11/11，`/tmp/oviraptor-bundle-import-extended.log`；含真实项目导出、真实 Reviewer 报告往返、删除后重导入、旧轮次、并发密文、事务回滚、所有 Native 表不变 |
| 共用目录导入器回归 | 主库 `artifact_import::` 61/61，`/tmp/oviraptor-bundle-import-regression.log`；共享事务代码修订后通过，不等同另一个 importer binary 的 30 项 |
| 新 UI 定向 | 6/6，`/tmp/oviraptor-bundle-import-ui.log` |
| 完整 UI | 183/183，`/tmp/oviraptor-bundle-import-ui-all.log` |
| 类型检查与生产构建 | 通过，`/tmp/oviraptor-bundle-import-build.log`；主 JS 843.08 kB，保留超过 500 kB 的拆包警告 |
| 严格 Clippy | `--all-targets --all-features -- -D warnings` exit 0，`/tmp/oviraptor-bundle-import-clippy.log` |
| fmt / 空白检查 | `cargo fmt -- --check`、`git diff --check` 均 exit 0 |
| 退役守卫 | 字面量 4/4、活残留基线 1/1；日志 `/tmp/oviraptor-bundle-import-retirement.log`、`/tmp/oviraptor-bundle-import-residual.log`。只复核更新两个实际变动文件的精确摘要及说明，没有新增宽泛豁免 |

## 6. 未完成项与下一步

1. 后续已修复旧任务/项目导出的覆盖命名和写入边界：统一 UUID 命名、完整写入后不覆盖发布、Unix 私有文件权限及 SQLite 读取事务。真实任务/项目文件往返已验证，见 `NEST_SNAPSHOT_IO_AUDIT.md`；跨平台 ACL、强杀残留清理与原始敏感包交付仍未完成。
2. 后续已将目录 manifest 改为一次有界读取、同字节摘要/payload，拒绝增长越界和规范化路径冲突；五项红测与九项新回归、共用导入器 70 项通过，见 `NEST_SNAPSHOT_IO_AUDIT.md`。这不等于整目录原子快照或完整祖先目录竞态防御。
3. 新入口真实 WebView、打包后三平台、Worker 网络集成与进程中断恢复尚未验证。
4. 其他 Findings 消费、SARIF 往返、总体覆盖独立审查、未完成工具/模型恢复及不确定效果人工核对仍需继续；美元预算核算、全角色/聊天/沙箱/工具供应和部署验收仍按 Master Plan 推进，不缩小原目标。
5. 本次没有访问用户提供的目标 URL、调用外部模型、部署或启用 Host Agent；没有修改生产授权门禁。
