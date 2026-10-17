# 源码实际分析视图与输出挂载隔离审计

状态：已实现分析器的实际 full/diff/auto 输入视图、不可变回执与 CI 实际范围绑定。早期 session 57719 的受审摘要失败已复核修复；后续完整流水线 session 93712 退出 0，主库 1075／历史导入器 30、严格 Clippy/fmt/空白检查全部通过。SourceBroker 文件范围与身份修复见 `NEST_SOURCE_BROKER_SCOPE_AUDIT.md`；分析结果范围、源码独立 Reviewer 和整体 Master Plan 尚未完成。

## 1. 先复现的真实缺陷

- 显式 Diff 有有效基线时，分析器仍能看到未选中的 `untouched.py`。
- 显式 Diff 基线无效时，仍调用整仓分析器。
- 初次隔离 `/src` 后，旧 `/out` 挂载仍是包含完整来源快照与分析视图的 scratch 父目录，既泄露整仓，又为只读 `/src` 提供可写别名。

前两项红测 session 23474 退出 101，日志 `/tmp/oviraptor-analysis-view-red.log`；初步修复后 session 25491 退出 0，日志 `/tmp/oviraptor-analysis-view-green.log`。输出挂载红测 session 94790 退出 101，日志 `/tmp/oviraptor-analysis-view-mount-red.log`。这些是调用生产源码流水线、检查实际传给分析器的目录与文件的测试，不是只断言报告标签。

## 2. 已实现的合同

`native_pipeline/analysis_view.rs` 中新增 `AnalysisManifest` 与 `SourceAnalysisView`：

1. 原 `RepositorySnapshot` 仍是完整来源记录，文件清单与 tree hash 不裁剪。另建独立分析目录，复制实际选中的文件，不使用硬链接。
2. 分析 manifest 绑定 scan/attempt、完整冻结请求、来源 JSON 摘要、来源 tree hash、HEAD/base、基线状态、实际 scope、选中文件及内容摘要、无内容的变更路径、回退原因和覆盖缺口。
3. Full 选择完整来源；Diff 只接受可用基线及已保存的已知清单，否则 unavailable，不执行整仓分析；Auto 可回退 Full，但原因明确进入报告，UI 明示语义。
4. 删除、排除或超限等路径统一记录为 `changedPathsWithoutContent`，不把所有无内容路径伪称为已确认删除。空增量不启动分析器，不声称安全通过。
5. Unix 分析根目录权限 0700、文件创建 0600，写入后同步并设只读。枚举校验拒绝软／硬链接、特殊文件、额外文件／目录、缺失、长度／摘要变化及越界根目录。
6. 读取复用 bounded、NOFOLLOW／NONBLOCK 的快照读取器；必要父目录预计算，避免对每个目录重复扫描完整清单。
7. `source_analysis_views` 按 scan/attempt 保存不可变 manifest、digest、root；UPDATE/REPLACE/独立 DELETE 禁止，父合同级联清理允许。
8. 初次来源与视图同事务保存，检查写入计数和读回；恢复只能验证已保存视图，不按当前 checkout 重建。检查来源记录恰好一条、完整来源绑定及视图回执。
9. 分析器前后、失败返回路径和最终导入／投影复核完整性；取消探针检测视图回执变化。恢复不能换 workdir 或 scratch，再取得不同输出挂载。
10. 每个 engine 独立使用 `scratch/analyzer-output/<engine>` 作为输出挂载，拒绝软链接与别名；不能传含来源快照的父目录。

新增 `analysis_policy_version`：新请求为 1；数据库迁移保留历史合同为 0，必须新建 attempt，不能用旧 Auto 请求自动取得新回退语义。task JSON 仍不是授权来源。

## 3. 分析器、CI 与 UI

生产 `AnalyzerSpec.repository` 使用真实分析视图；报告分别给出选中 `fileCount` 与完整 `sourceFileCount`。CI freeze 使用实际 scope、选中文件数与 analysis manifest digest，完整来源 tree hash 保持不变。

CodeQL 当前只在 Full 执行。Diff 明确留下 `codeql:diff_project_context_not_authorized`，不偷偷挂载整仓，也不声称残缺项目完成 CodeQL。完整项目上下文授权仍需另行设计和实现。

工作台明确说明 Auto 回退、Diff 不扩大范围、Full 不沿用隐藏旧 base，以及增量下 CodeQL 的覆盖限制。新增真实 SFC 渲染测试，不用源码字符串匹配代替 UI 行为验证。

## 4. 新增回归与验证证据

`commands/tests_source_analysis_view.rs` 共 20 项，覆盖真实输入选择、无效/空/删除 Diff、Full/Auto、写入 ABORT/IGNORE、原来源与视图变化、不可变回执及级联删除、缺失/损坏回执、Git 元数据变化后恢复、取消探针、旧版本合同、输出隔离、链接/FIFO/额外目录、隐藏索引、特殊文件名/超限、真实旧列迁移、来源写入忽略及被修改、恢复 workdir 替换。

- 最终定向日志 `/tmp/oviraptor-analysis-view-final-focused.log`：20 passed，13.73 秒。
- 完整顺序验证 session 57719 退出 101：`/tmp/oviraptor-analysis-view-full.log` 为 1065 passed／1 failed，491.06 秒；退役审核摘要失败使导入器、严格 Clippy 和 fmt 后续命令未运行。不能把未生成的日志视为通过。
- 前端 session 46646 已退出 0：实际 SFC 147 passed，vue-tsc/Vite 通过。日志 `/tmp/oviraptor-analysis-view-ui.log`、`/tmp/oviraptor-analysis-view-build.log`。主 JS 830.15 kB，既有 >500 kB 拆包警告仍在。
- localhost 浏览器回环 session 1197 已退出 0：`passed=true`，匿名/比较采集 complete，8 个观察请求，身份隔离 true。日志 `/tmp/oviraptor-analysis-view-loopback.log`。

没有部署，没有访问用户外部 URL，没有启用目标主机执行。

### 退役审核失败的复核

失败项只有 `src-tauri/src/db_initialize_columns.rs`。读取实际残留上下文后确认均为历史路径修复、废弃配置删除和假完成修复，不包含后端启动。本轮新增行仅为 `source_scope_contracts.analysis_policy_version` 的默认 0／值域 0、1 迁移。对当前文件移除这一行再计算摘要，精确得到原受审 `3f1b6dc232217f403b84cc11e926b4787ce1cb386d456a0934b497c786c96577`；当前完整文件为 `a90ef53ddf00b1fa57b292f17ac6cc54d71e9f1a1f4b31cd7746cbd23f8bfb24`。据此仅更新该条摘要与审核理由；12 个残留字面量未增加，固定扫描范围、失败断言及分类限制未修改。

## 5. 不可被本增量替代的剩余工作

1. 后续 SourceBroker 已完成 inventory/search/read/dependency/候选路径的分析视图约束，详见 `NEST_SOURCE_BROKER_SCOPE_AUDIT.md`。
2. 后续已绑定有效 run/root/attempt、事务内状态/取消与权限复核；source 专用 assignment 和分析结果跨 attempt 可见性仍需实现。
3. 源码 SARIF 仍缺真实 assignment→child-run→mailbox→独立 Reviewer 执行链。CI 调用使用 scan ID 而不是正确 root，当前 revision 资格、去重和缺少 review 时的通过语义仍待修复。
4. 完整源码/greybox 启动冻结、沙箱供应与工具链、凭据生命周期、资产/学习/UI、重启未知效果及授权环境验收继续按 Master Plan 实施。本审计不代表整体完成。
