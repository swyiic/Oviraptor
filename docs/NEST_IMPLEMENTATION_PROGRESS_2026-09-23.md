# Nest Native 与联合多智能体实施状态（更新于 2026-09-30）

## 1. 当前结论

### 最新追加（Loop2，2026-09-30）：旧 stage 特权分支持续退役

**整体仍未完成；本批仅删除 `appsec_validation` 中的旧来源特权分支。** 空 `source_types` 回退统一为中性 `scanner`，`ai_validation` 保留给人工 Repeater 验证；`source_key` 仍保留原始 stage 可追溯。新测试 + 关联回归 3/3 通过，fmt 与 lib Clippy 通过。旧 stage 清理分支、技能/知识迁移、安装日志持久回放等仍未完成。详情见 `NEST_HISTORICAL_SOURCE_BRANCH_RETIREMENT_AUDIT_2026-09-30.md`。

### 最新追加（Loop1，2026-09-30）：历史来源专用 UI 展示标签中性化

**整体仍未完成；本批仅清除展示层品牌专用标签，不增强执行能力。** `execution/presentation.ts` 与 `sentinel/presentation.ts` 的历史后端展示统一改为 `历史封存（只读）`，`strix` 条件仅作旧数据兼容保留并注释待 DB 清理后拆除；`test_execution_presentation.cjs` 同步改为中性断言并新增品牌字面量负向用例。执行展示+详情+Fuse 定向 52/52 通过，展示品牌 0 命中，`git diff --check` 通过。`ReleaseNotesDialog.vue` 归档文案、旧 `stage === 'strix'` 数据分支、recon/技能/迁移别名、安装日志持久回放、外部脚本通知、真实桌面验收仍未完成。详情见 `NEST_HISTORICAL_LABEL_NEUTRALIZATION_AUDIT_2026-09-30.md`。

### 最新追加：SARIF 专属兼容、导入层及展示层任务 ID 别名退役

**整体仍未完成；本批完成导入语义和只读展示清理，不增强目标执行能力。** 保留当前 Native JSON／源码报告合同；未修改授权、Reviewer、工具门禁，未访问外部目标，未物理删除真实用户数据库、CAS、不可变 revisions 或源报告。

- `adapters/sarif.rs` 删除旧厂商 coverage 字段读取／提升、旧规则名前缀分类及 `properties.kind` 回退；共享 canonical 字段列表删除 `coverage_outcome`。分类只依赖顶层 kind／level，缺省 kind 为 fail；未知、空或非字符串 kind 不成为候选。任意 properties 仅作为不解释的原始扩展证据，不生成执行权限、Native 确认或覆盖信用。移除特殊判断后，warning／error 且缺省或显式 fail 的旧标记结果可能成为**未审核候选**；这是公开的分类变更，不再承诺旧厂商语义。
- `ADAPTER_VERSION` 从 3 升为 4，旧签名在下次导入时重解析。新增测试真实种入旧 Coverage revision／membership，验证重导后旧当前投影退出、当前候选进入、原 revision 字节不变；重复导入幂等。没有把 Native source-report 专有的 `<3` 归属修正阈值机械改成 4。没有后台重扫或自动清理尚未重导的真实存量。
- `store::scan_marked_deleted`、任务预览、导入列表和 bundle 预览统一只认精确 scan-id，删除旧前缀关联及别名删除语义。相同目录不能证明两个 ID 属于同一任务；旧别名删除不能隐藏无关当前任务；精确 ID 删除仍生效。既有相同内容／不同任务测试改用当前身份，负向测试覆盖旧关联拒绝，读取前后 Native 状态相同。
- 新增 SARIF 专项测试文件 116 行，复用现有夹具和真实数据库入口；展示专项文件 179 行。生产 SARIF adapter 缩至 292 行。没有新依赖、新执行器、重复格式或全局公共类。
- 残留登记现为 **52 文件：fixture 39、importer 4、migration 5、historical_label 4**。移除已无字面量的源文件登记，只更新实际审查过的测试文件和入口摘要，最终退役守卫通过。此数仅是登记范围中的文件数，不是完成比例；测试负向样本和真实生产残留必须区分。

最终验证（集合有重叠，不相加）：

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| 导入模块 | 99/99 | `/tmp/oviraptor-sarif-retirement-final-import.log` |
| 历史相关 | 44/44 | `/tmp/oviraptor-sarif-retirement-historical.log` |
| 退役守卫 | 46/46 | `/tmp/oviraptor-sarif-retirement-final-retirement_.log` |
| source_report／source_findings_ci | 7/7、2/2 | `/tmp/oviraptor-sarif-retirement-final-source_report.log`；`/tmp/oviraptor-sarif-retirement-final-source_findings_ci.log` |
| 当前 JSON 包导入／展示隐私 | 11/11、6/6 | `/tmp/oviraptor-sarif-retirement-final-bundle_import.log`；`/tmp/oviraptor-sarif-retirement-final-trace_display_privacy.log` |
| 导入工具（启用 import-tools） | 45/45 | `/tmp/oviraptor-sarif-retirement-cli-green.log` |
| 轨迹／聊天事件／安装／runner／资产日志 UI | 71/71 | `/tmp/oviraptor-sarif-retirement-realtime-ui.log` |
| 全目标／全特性严格 Clippy | 通过 | `/tmp/oviraptor-sarif-retirement-clippy.log` |
| 前端类型／Rust 格式／diff 空白 | 通过 | `/tmp/oviraptor-sarif-retirement-typecheck.log`；`/tmp/oviraptor-sarif-retirement-fmt.log`；`git diff --check` |

失败过程保留：SARIF 两项红测复现旧属性影响分类和 v3 缓存不重解析（`/tmp/oviraptor-sarif-retirement-red.log`）；导入别名红测、三展示入口红测分别复现无关删除标记遮蔽当前记录（`/tmp/oviraptor-sarif-retirement-alias-red.log`、`/tmp/oviraptor-sarif-retirement-view-red.log`）。首次退役守卫 44/45，遗漏的是新增 test include 后入口文件的审查摘要，复核该入口后精确更新，最终 46/46。导入工具首次因未启用必需 feature 未运行测试（`/tmp/oviraptor-sarif-retirement-cli.log`），显式启用后 45/45。格式首次仅发现一处插入语句换行，调整后通过；不将中间失败隐藏或当作通过。

Cargo 全部离线、低优先级、单构建作业／单测试线程，UI 在 Cargo 完成后串行执行。未运行完整主库／UI 全集、生产桌面包或实机延迟基准；没有宣称解决此前 360% CPU 的根因。

**剩余项及实时状态：** recon reader／旧目录回退、skills／knowledge 旧格式别名、迁移字段、历史 UI 标签和真实旧数据的安全清理仍未完成。当前对话／轨迹以已提交事件通知为主，低频补读用于漏通知；安装日志仍为瞬时事件、前端保留 300 条，无持久回放。其他进程或未接入统一提交钩子的写入仍可能等待 15 秒兜底，外部脚本链路和真实桌面断线／重连延迟仍需验收。不能通过删除范围授权或 Reviewer 来替代这些工程工作，也不能把定向 UI 测试通过写成“所有日志完全实时”。下面旧批次的 SARIF／scan-id 待处理描述，以本节的已完成范围为准。

### 最新追加：旧 canonical 轨迹解释退役，终态轨迹漏通知可补读

**整体仍未完成；本批是导入展示清理和只读 UI 修复，不涉及目标执行能力增强。** 未修改授权／Reviewer 边界，未访问外部测试 URL，未删除真实用户数据库、CAS、原始 revisions 或源报告。

- `historical_agent_trace.rs` 删除旧任务 ID 别名绑定，以及旧 run／session／message／tool／usage 展示与快照用量协调。仅允许当前 `model_audit`／`event_trace`，同时验证 envelope 来源、kind 与只读未复核 claim；未知 trace kind 不进入通用消息兜底，退役 adapter 在解析 payload 前即被拒绝。
- 精确任务归属按路径组件验证，允许任务内 `reports/`／attempt 子目录，拒绝同名前缀、其他目录及 `..`；不访问已离线源。attempt 上界先于格式／claim 过滤计算，防止最新旧记录被拒绝后回退到上一轮。旧别名的删除标记不能隐藏当前任务。
- 删除 `scan_execution_metrics.rs` 中 6 个无剩余业务调用的旧用量别名函数，删除 `trace_message_detail` 旧提取函数。旧正向用量测试改为当前 Hook provider token／cache 合同，旧格式拒绝由新持久化负向测试覆盖。Native 公共摘要字段保留，当前审计不虚构 agent／message／tool／target；公共展示层的私有推理过滤继续验证。
- `useLiveTrace.ts` 修复终态或首次详情失败后，丢失通知可能使视图永久过期的问题。15 秒兜底同样读取当前已提交状态，事件仍经 50ms 合并即时刷新；隐藏／卸载停止读取、单次在途请求及任务切换隔离不变。订阅恢复时保留“补查 + 注册成功后补读”，关闭订阅窗口的数据缺口；没有改成高频扫文件。
- 残留登记 **55 文件：fixture 39、importer 7、migration 5、historical_label 4**。删除两个已无字面量文件的登记，仅更新实际审查过的历史轨迹负向夹具摘要；全部 SHA-256／出现次数／路径证据校验通过。该数字不是剩余兼容分支数量，更不是清零或整体验收。

最终验证（各集合有重叠，不相加）：

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| 历史相关 | 43/43 | `/tmp/oviraptor-canonical-retirement-final-historical.log` |
| 展示隐私 | 6/6 | `/tmp/oviraptor-canonical-retirement-final-trace_display_privacy.log` |
| 导入模块 | 96/96 | `/tmp/oviraptor-canonical-retirement-final-artifact_import_tests.log` |
| 退役守卫 | 42/42 | `/tmp/oviraptor-canonical-retirement-final-retirement_.log` |
| Native trace／当前 Hook provider 用量 | 1/1、1/1 | `/tmp/oviraptor-canonical-retirement-final-native_trace.log`；`/tmp/oviraptor-canonical-retirement-final-current_hook_usage_keeps_provider.log` |
| 轨迹／聊天事件／安装／runner／资产日志 UI | 71/71 | `/tmp/oviraptor-canonical-retirement-realtime-ui-green.log` |
| 全目标／全特性严格 Clippy | 通过 | `/tmp/oviraptor-canonical-retirement-clippy-green.log` |
| 前端类型／Rust 格式／diff 空白 | 通过 | `/tmp/oviraptor-canonical-retirement-typecheck.log`；`/tmp/oviraptor-canonical-retirement-fmt.log`；`git diff --check` |

失败过程保留：退役红测为 4 通过／3 预期失败（`/tmp/oviraptor-canonical-retirement-red.log`）；历史首次 41/42，定位为正常嵌套报告目录被过严路径相等判断拒绝，改为组件归属并补专项后通过。UI 红测 27/30（`/tmp/oviraptor-trace-reconciliation-red.log`）；首次修复后 70/71，剩余断言仍要求订阅恢复仅一次读取，按关闭订阅缺口的两次有序读取更新后全绿。严格 Clippy 首次报告上述 6 个旧函数 dead-code，删除而非增加 allow 属性后通过。中间结果不是最终验收。

所有 Cargo 检查离线、低优先级、单作业，测试单线程；UI 串行运行。这不是 CPU 硬限额或端到端延迟证明。本批未运行完整 Rust／UI 全集、桌面生产包或实机回放验证。

**剩余工作不能漏掉：** SARIF 旧 coverage 属性、其他导入投影中的 recon／scan-id 别名、迁移字段和经预览确认的真实旧数据清理；安装日志持久回放、未接入钩子的外部进程／脚本通知、真实桌面延迟与断线补读验收。安装日志目前仍是临时事件、UI 留存 300 条；未接入统一 WAL 钩子的写入仍可能等待 15 秒兜底。以下旧批次“旧 canonical 轨迹待退役”和“终态不补查”的状态以本节为准，不得恢复已删除兼容分支。

### 最新追加：删除外部 SQLite trace reader 与专属快照接口

**整体仍未完成。此批退役的是外部旧轨迹数据库读取，不是删除应用自身 SQLite，也没有清空真实任务数据。** 当前 Native JSON／SARIF、Hook JSONL 和模型审计继续使用既有合同；不恢复旧格式兼容，不扩展目标执行能力。

- 删除 `artifact_import/adapters/sqlite_trace.rs`、模块声明、发现锚点、派发和合并优先级。根目录／嵌套 `.state/agents.db`、WAL／SHM 不加入 manifest／CAS；独立旧目录不建包，混入当前报告时其变化不触发重导。
- 删除 `ImportContext.scratch_dir`、`ParseContext.scratch` 和仅供旧数据库使用的 `ScratchFile`。调整公开导入、源码报告回读及测试 helper 调用；保留源码分析本身所需的独立快照目录。移除轨迹展示里的 `agents.db` 父目录特判。
- 删除两份没有剩余读取依赖的正向 `agents.db.sql` 测试样本及 manifest 条目。其余 30 份样本摘要未改，库存测试明确为“待退役输入”而不是“受支持格式”。负向数据库用例独立生成，未删除用户数据库、CAS 或源报告。
- 将有效行保留、源文件不变、请求／session 隔离、pending 用量及展示隐私断言迁到当前 Hook；审计不能凭空生成 agent／工具消息计数。私有推理隐藏通过实际展示入口验证，不以内部读取结构代替 UI 结果。
- 新测试发现并修复凭据预览不一致：原文加密已检测到转义 JSON 凭据，但展示路径曾重新用较弱检测覆盖 secret 标志。现在加密、标志和原文预览使用同一判定；含凭据原文只展示占位说明，结构化脱敏记录仍可阅读，密文解密保持源字节完整。该修复不回填已经存在的真实库记录，存量清理仍待独立审计。
- 残留登记仍为 **57 文件**，不能解释为剥离完成。本批只更新十份实际审阅文件的摘要与理由，不新增豁免；全部摘要／字面量数量／路径匹配检查通过。

本批验证（集合重叠，不相加）：

| 范围 | 结果 | 本地证据 |
| --- | --- | --- |
| SQLite 退役红测 | 旧 reader 上 3 项真实失败，非编译失败 | `/tmp/oviraptor-sqlite-retirement-red.log` |
| 导入模块 | 96/96 | `/tmp/oviraptor-sqlite-retirement-imports-green.log` |
| 历史相关／展示隐私 | 40/40、6/6 | `/tmp/oviraptor-sqlite-retirement-{historical,trace_display_privacy}-green.log` |
| 当前源码报告／CI 回读／公共 JSON 包导入 | 15/15、12/12、11/11 | `/tmp/oviraptor-sqlite-retirement-{source_findings_,tests_native_ci,bundle_import}-green.log` |
| 剩余样本清单 | 2/2 | `/tmp/oviraptor-sqlite-retirement-legacy_fixture-green.log` |
| 导入工具 | 45/45 | `/tmp/oviraptor-sqlite-retirement-cli.log` |
| 退役守卫 | 39/39 | `/tmp/oviraptor-sqlite-retirement-guards.log` |
| 轨迹／聊天事件／安装／runner／资产日志 UI | 70/70 | `/tmp/oviraptor-sqlite-retirement-realtime-ui.log` |
| 全目标／全特性严格 Clippy | 通过 | `/tmp/oviraptor-sqlite-retirement-clippy.log` |
| Rust 格式／工作树 diff 空白检查 | 通过 | `/tmp/oviraptor-sqlite-retirement-fmt.log`；`git diff --check` |

中间证据：导入首次为 95 通过／1 失败，定位到凭据预览问题；修复后 96 全绿。历史首次为 39 通过／1 失败，原因是新增私有推理断言调用了内部读取层；改为真实展示入口后 40 全绿。不得将中间结果当作最终验收。

Cargo 离线、低优先级、单作业、单测试线程运行，UI 回归在 Cargo 结束后串行运行；这不是 CPU 硬限额。本批没有重跑完整 Rust／UI 全集、生产桌面打包或外部 URL 测试；70 项 UI 是本地组件／模拟事件测试，不是实时延迟的端到端证明。

**实时状态本批仅复核，没有新实现：** 协作消息仍是 WAL 提交唤醒→25ms 合并→通知→UI 补读，15s 是漏通知兜底；未接入钩子的外部写入仍可能等待兜底。安装 stdout／stderr 按记录推送，但 UI 仅留 300 条、无持久回放；未完成全部外部脚本接线和真实桌面延迟验收。

**必须继续：** SARIF 旧 coverage 属性、recon／scan-id 别名、旧 canonical 轨迹展示分支、迁移字段及真实旧数据清理；安装日志持久回放、外部写入通知覆盖和桌面端端到端验收。以下旧批次中“SQLite reader 待删除”的状态已被本节覆盖，其余未完成项不能视为自动完成。

### 最新追加：删除旧漏洞 JSON reader，纠正旧配置提升测试与执行规范

**整体仍未完成。本批确实删除旧 reader，不是重命名或保留历史兼容；实时链路本批只复核与回归，没有新增持久化功能。** 未修改目标执行、授权或 Reviewer 边界，未探测外部 URL，未删除用户真实数据库、CAS 或源报告。

- 删除 `artifact_import/adapters/legacy_strix.rs` 及模块声明、旧漏洞 JSON 派发／发现／合并优先级。`vulnerabilities.json` 不再生成结果，也不进入当前包清单。没有替代 reader、新运行依赖或第二种 CI 导出格式；现有正式 `oviraptor-source-review-v1` JSON／SARIF 继续独立使用。
- 退役回归移至 `tests_result_retirement.rs`：根目录／嵌套目录中的顶层数组、四种 envelope 与损坏 JSON 均不能解析；旧文件独立存在不能创建包或 CAS，混入当前报告不被收录，其变化不能触发重导，源字节和 Native 状态不变。已有 CSV／Markdown 拒绝回归继续保留。
- 原文哈希、隐私、坏文件隔离、限额、事务、幂等、revision 和公开预览断言迁至现有 SARIF／审计格式。完整 HTTP method／parameter 指纹通过 canonical 输入验证，不让 SARIF properties 获得额外语义。当前 JSON／SARIF 的真实产品往返仍由正式导出测试覆盖。
- CLI 补测发现 `db_tests.rs` 仍要求旧模型配置自动提升；现改为证明旧凭据／profile 不激活 Native、重复初始化幂等、显式当前配置重启后保留且不借用旧密钥。没有为旧测试恢复已移除的配置提升实现。
- 精确残留登记为 **57 文件：fixture 40、importer 8、migration 5、historical_label 4**。删除无剩余字面量的登记，仅对实际审查的变更更新摘要／理由；57 项 SHA-256、出现次数及路径匹配均核验通过。该数字不是业务兼容分支数量，也不是清零证据。
- Master Plan §2.1、§9、§11.4、§13 已把永久正向兼容改为退役合同，移除目标目录树中的已删 reader，并改用当前 SARIF envelope 示例。后续执行者不得按下方旧批次记录重新添加已删除功能。

验证结果（集合重叠，不相加）：

| 范围 | 结果 | 本地日志 |
| --- | --- | --- |
| 导入模块 | 104/104 | `/tmp/oviraptor-json-retirement-imports.log` |
| 公开预览／历史相关／状态视图 | 2/2、42/42、1/1 | `/tmp/oviraptor-json-retirement-{previews,history,status}.log` |
| 导入工具，含初始化与通知模块 | 45/45 | `/tmp/oviraptor-json-retirement-cli-green.log` |
| 正式源码报告 JSON／SARIF | 15/15 | `/tmp/oviraptor-json-retirement-source.log` |
| 退役检查，最终版本 | 35/35 | `/tmp/oviraptor-json-retirement-guards-final.log` |
| 轨迹／聊天事件／安装／runner／资产日志 UI | 70/70 | `/tmp/oviraptor-json-retirement-realtime-ui.log` |
| 全目标／全特性严格 Clippy | 通过 | `/tmp/oviraptor-json-retirement-clippy.log` |

初始三项退役测试在旧 reader 上确实失败，证据 `/tmp/oviraptor-json-retirement-red.log`。中途修正了测试借用编译问题、通过 JSON 断言 serde 跳过字段的问题；CLI 首次漏开 `import-tools`，补开后暴露旧提升断言（44 通过／1 失败），修正后的 45 项全绿。不能把这些中间失败或前一批 Clippy 当作最终证据。Cargo 均离线、低优先级、单作业与单测试线程运行；本批 Cargo 与 UI 进程已收取退出码 0。

实时链路现状与必须继续完成项：

1. `collaboration_events/pump.rs` 已是提交唤醒、25ms 合并、成功发送后推进游标；前端事件提示触发权威数据补读。15 秒补查用于漏通知，不能描述为所有生产者均即时推送，也不能把 25ms 合并窗口当成端到端延迟承诺。
2. `useInstallLogPanel.ts` 明确仍为瞬时日志流，最多保留 300 条，订阅失败标记可能缺失。必须补安装批次身份、持久序号、脱敏后存储、按游标补读及保留策略，再验证重连无重复／漏行；目前未实现。
3. 外部脚本／未接提交钩子的写入仍可能依赖 15 秒补查。逐项确认 stdout／stderr、文件追加、重命名／轮转／截断、子进程退出与异常路径的通知及补读，再做真实桌面延迟、突发输出背压和退出释放验收；不能仅缩短轮询周期。
4. 继续退役 SQLite reader、SARIF 旧 coverage 属性、旧 ID／路径别名、adoption／数据库迁移及旧 fixture 清单；先保留当前数据与拒绝旧状态的安全回归，不清空残留登记来掩盖代码。
5. 真实存量清理需要精确对象预览、备份和明确确认。本轮未跑完整 Rust／UI 套件、生产构建、桌面安装包、CPU 耐久或实际端到端延迟测试，不能宣布 Strix 全量剥离或所有日志／对话实时可靠交付。

### 最新追加：删除无业务调用的旧 CI JSON writer，核验正式导出合同

**整体仍未完成。** 调用链核对纠正了上一批的依赖判断：`GateReport::write_bundle` 仅被旧 CI-006 测试调用，应用真正的 JSON／SARIF／bundle 导出位于 `commands/native_source_findings.rs`，已经使用 `oviraptor-source-review-v1` 与 `source_report` 适配器，不需要再设计第二套 CI JSON 格式。

- 删除旧 standalone writer 和仅供它使用的路径导入；原门禁评估与独立 SARIF 视图保留。CI-006 改为标准 SARIF 回读，继续核验指纹、严重程度、原文字节和只读／无执行权限，不再要求旧 producer 来源。
- 在既有正式导出测试中增加版本号、格式、文件名前缀以及混合导入后 `source_report` 来源／字段来源断言。仍验证冻结的 CI 策略不受后续任务配置影响，导出不续租、不改变 Native 状态，重新导入不生成 Reviewer 权威。
- 删除退役登记中已不含旧名称的 CI 测试文件条目。登记为 **61 个文件：41 fixture／11 importer／5 migration／4 historical_label**；所有剩余 SHA-256 摘要一致，没有扩大例外。没有新建业务模块、适配器、依赖或测试入口。中途试建的第二套 CI 格式已完整撤回，其日志不作为交付证据。

最终相关验证（集合不相加）：正式 source-findings **15/15**、Native CI **12/12**、真实 JSON 报告往返 **1/1**、导入模块 **105/105**、退役检查 **35/35**；全目标／全特性严格 Clippy、Cargo fmt 与 `git diff --check` 通过。证据为 `/tmp/oviraptor-ci-writer-retirement-{source,ci,roundtrip,imports,guards,clippy}.log`。第一组运行暴露的未使用测试导入已移除，后续 CI 与严格 Clippy 均通过。单 Cargo 作业、单测试线程、离线、低优先级运行，进程均已退出。

没有重跑完整 Rust／UI 套件，没有桌面包、真实延迟或 CPU 耐久验收，未连接用户给出的外部 URL，未清理真实数据库／CAS。下一项删除对象仍是 `artifact_import/adapters/legacy_strix.rs` 及泛化漏洞 JSON 的发现／派发／合并优先级；关联的原文、隐私、限额、事务、幂等测试需要迁到现有正式格式，不能直接删掉独有断言。SQLite reader、SARIF 旧属性、旧字段及真实存量清理也仍在待办。日志方面，本次只核实剩余缺口，未新增实时实现：安装日志无持久回放、部分外部写入仍依赖 15 秒补查，不可宣称所有日志和 Agent 对话已完成实时可靠交付。

### 最新追加：旧漏洞 CSV／Markdown 入口退役与当前格式回归

**整体仍未完成。本轮完成的是两个旧文本结果格式的退役，不是所有 Strix reader、字段、存量数据或实时日志的最终验收。** 未修改扫描执行和授权门禁，未运行外部目标，未删除用户真实数据库、CAS 或源报告。

- 删除 `artifact_import/adapters/csv.rs`、`adapters/markdown.rs` 及其派发、发现、合并优先级分支。`vulnerabilities.csv` 与 `vulnerabilities/*.md` 不再生成记录、触发结果包发现或进入混合包清单；修改这些旧文件也不触发当前包重导。
- 删除旧 CSV 专属限额／坏行正向测试及 CSV 专属拒绝码分支。通用合并前记录上限仍在 CAS 发布和包提交前执行，不因重复项最终可合并而放宽。以当前 SARIF 覆盖整包拒绝、零限额、失败保留旧投影、合法重试幂等及健康相邻包独立导入。
- 原文逐字节不变、中文／引号／换行／空字段、不可信消息按纯文本保留、独立记录不丢和单文件变更重导，迁到当前审计 JSON、Hook JSONL 与标准 SARIF；不为旧测试恢复已删除的读取器。保留独立的资产 CSV 业务，不把所有 CSV 功能误删。
- 新增退役测试 3 项、当前限额测试 4 项，分别位于 131 行和 175 行的业务测试文件。残留登记仅更新 4 个已审查文件的摘要和理由；登记仍为 62 个文件，因为本轮删除的模块本来不含旧名称字面量，不能据字面量数量不变否认语义入口已删除，也不能据测试通过宣布清零。

验证（集合有重叠，不能相加）：

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| 删除前新退役回归 | 3 项均按预期断言失败 | `/tmp/oviraptor-text-retirement-red.log` |
| 导入模块 | 105 项通过 | `/tmp/oviraptor-text-retirement-imports.log` |
| 退役／残留登记及相关拒绝测试 | 35 项通过 | `/tmp/oviraptor-text-retirement-regression.log` |
| backend 相关检查 | 21 项通过 | `/tmp/oviraptor-text-retirement-guards.log` |
| 历史展示、只读来源与隐私等相关回归 | 41 项通过 | `/tmp/oviraptor-text-retirement-history.log` |
| 当前 Native CI 导出再导入 | 1 项通过 | `/tmp/oviraptor-text-retirement-ci.log` |
| 全目标／全特性严格 Clippy | 通过 | `/tmp/oviraptor-text-retirement-clippy.log` |

修改前第一次测试编译曾使用不存在的枚举变体，已修正；该编译失败不算删除前的有效失败证据。上表 red 日志是在旧生产代码上实际运行、得到三项行为断言失败后的结果。本轮未修改前端，未重跑 UI 或生产构建，上一节的 UI 结果仅代表上一轮。完整 1496 项 Rust、安装包、长时间 CPU 和端到端可见延迟仍未重验。

依赖判断更正：继续核对调用链发现，`native_pipeline/ci.rs::write_bundle` 仅由旧 CI-006 测试调用，不是应用正式导出入口。产品早已通过 `commands/native_source_findings.rs` 导出 `oviraptor-source-review-v1` JSON、SARIF 和完整 bundle，包含冻结的 CI 信息，并由 `source_report` 适配器回读；无需新建第二套 Native 合同或适配器。旧 standalone writer 已删除，CI-006 只保留标准 SARIF 回读。泛化旧 JSON envelope 和旧 producer 仍待退役，但不能再以当前产品导出依赖为理由保留。随后继续处理 SQLite 轨迹、SARIF 旧属性、旧 ID／字段与真实存量清理。日志未完成项沿用下一节：安装日志持久回放、外部脚本通知和全链路验收仍未完成。

### 最新追加：协作事件提交唤醒、游标可靠性与退出清理

**整体仍未完成。本节仅覆盖下文“聊天后端仍每 250ms 轮询”的旧状态，不代表全部日志已实时化，也不代表 Strix 字段／导入器／存量数据已清零。** 本轮只修改展示通知基础设施，不修改扫描执行、授权、范围、预算或 Reviewer 门禁；没有部署、接触外部测试目标、删除真实用户数据库或原件。

实际改动：

- 在统一 `db::open` 连接安装 WAL 提交通知，将桌面协作事件泵改为提交唤醒。回调只唤醒展示读取，不派发任务、不接触消息正文。因为该回调会替换 SQLite 默认自动检查点，显式保留当前 bundled SQLite 的 1000 页 PASSIVE 检查点行为；检查点暂时繁忙不能把已经成功的提交报告成失败。
- 提交信号使用世代计数与条件变量；**先取世代再读数据库**，避免提交恰好发生在读完到等待之间而漏唤醒。进程级通知可能被其他数据库的提交唤醒，但读取始终限定在自己的固定数据库路径，不跨库广播正文或事件身份。
- 正常提交唤醒后有 25ms 固定合并窗口，控制密集、无关提交引起的读取频率；有积压时按 256 条连续补读并让出线程。不再固定睡眠 250ms。该合并窗口不是端到端延迟保证。
- 启动前记录既有水位；初次水位读取失败时保留从零恢复提示的能力，不在稍后的初始化中跳过新消息。只读连接不创建数据库，busy timeout 为 250ms。
- 只有事件发送成功才推进游标；数据库／发送错误保留未发送位置，按 1 秒退避重试，连续失败只输出一次不含正文、SQL 或路径的错误提示。事件仍只有序号、任务／尝试、实体和事件类型；发送成功**不等于浏览器已经消费**，前端自己的持久游标补读仍必要。
- 应用真正 `RunEvent::Exit` 时通知线程退出；关闭窗口隐藏到托盘不会停泵。退出等待可以被唤醒，不在 UI 线程同步 join 正在发送事件的线程。
- 通知线程、提交信号、测试夹具与回归分别放在 `src-tauri/src/collaboration_events/`。离线导入工具的测试引用共享数据库模块，补齐显式模块路径；该工具不启动桌面通知线程。本轮新增生产文件各不超过 200 行。

验证（集合有重叠，不相加）：

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| 最终后端专项：15 项新通知回归、退役守卫、现有时间线／事务回滚 | 50 项通过 | `/tmp/oviraptor-commit-events-regression-final.log` |
| 前端完整 UI 回归 | 765 项通过 | `/tmp/oviraptor-commit-events-ui.log` |
| Vue 类型检查与生产构建 | 通过 | `/tmp/oviraptor-commit-events-build-final.log` |
| 离线单作业、全目标／全特性严格 Clippy | 通过 | `/tmp/oviraptor-commit-events-clippy-final.log` |
| Cargo fmt、diff 空白及残留登记摘要检查 | 通过 | `/tmp/oviraptor-commit-events-static-final.log` |

新回归覆盖：提交早于等待、多次提交合并、无通知时超时、停止打断等待／退避、提交风暴不绕过退避、统一连接真实提交唤醒、检查点阈值与读者占用、600 条分页和发送失败后续传、未提交／回滚不可见、通知不含正文、只读不建库、跨库隔离、批中停止，以及发送期间新提交和线程实际退出。只使用本地合成记录，未调用模型或扫描目标。

证据限制：本轮未重跑完整 1496 项 Rust 测试，未做桌面安装包、端到端可见延迟或长时间 CPU 验收。最初 14 项专项与 49 项相关回归通过后，又增加了合并窗口测试；以上最终 50 项和最终 Clippy 才覆盖最终后端版本。沿用下文中止的旧全量日志仅作为历史失败证据，不能用它宣布全量通过。

当前日志通路与剩余工作：

- **协作聊天**：本进程统一连接写入后提交唤醒；其他进程或未安装钩子的写入仍依赖 15 秒补查。前端保留事件优先、错误重试、自己的增量水位和可见性恢复。
- **运行日志／资产日志**：已有各自提交后提示及 `useCommittedRefresh` 的 15 秒补查。本轮没有重写这些生产者；765 项 UI 回归不构成所有外部脚本生产者的端到端验收。
- **轨迹页**：沿用上一节已实现的订阅注册失败重试与恢复补读，不能保证识别一切静默断线。
- **安装日志**：仍是有界、脱敏的瞬时输出流，缺少批次持久化及按游标回放；关闭页面期间的日志不能以“重新订阅”补回来。
- **脚本链路**：仍需逐条确认日志从子进程 stdout/stderr、文件或数据库写入到 UI 的路径。此次没有为外部文件写入增加监听，也没有改动脚本执行或自主扫描能力。
- **退役清理**：剩余登记仍是 62 个文件（fixture 42、importer 11、migration 5、historical_label 4）；仅复核更新了 `db.rs` 的摘要和理由，没有扩大残留许可。旧漏洞、SQLite/SARIF 属性等 reader、旧 ID／删除别名及真实存量清理仍须完成，禁止恢复历史兼容。

### 最新追加：旧 run 入口退役，现行格式夹具修复，轨迹事件订阅恢复

**整体仍未完成，不能宣布 Strix 字段／存量数据清零，也不能宣布所有日志已经实时化。** 本节覆盖下文“旧 run 解析仍在”的旧状态；其余明确未完项仍有效。本轮没有恢复历史格式兼容，没有删除真实数据库或原件，没有访问外部测试 URL 或部署。

实际改动与边界：

- 删除 `run.json` 的发现锚点、解析分派、旧状态别名和用量解析、producer 版本／schema 推断；scope 不再从旧 run 文件推导任务身份。保留当前 `meta.json`、`model-prompt-audit.json`、`llm-hook.jsonl` 和标准 SARIF 路径。`adapters/legacy_strix.rs` 仍保留漏洞 JSON 解析，**不是整个适配器已经删除**。
- 删除测试专用 `commands/legacy_artifact_compat.rs`、include、旧目录 walker 和旧 run 常量。新增三个退役拒绝回归，覆盖直接分派、独占目录、混合／嵌套目录及绕过发现层直接传入旧字节：不产生旧状态／用量／活动运行，不改变源文件，当前数据仍可导入且重复导入幂等。
- 将通用完整性、篡改、事务、来源隔离、Native 任务不变性、历史展示与隐私测试迁到现行审计 JSON／标准 SARIF；当前 Hook 用量按来源隔离计数，不再依赖旧 run 快照。提示词审计夹具改用完整 `ModelPromptAudit` 序列化，未给生产 reader 添加缺字段默认值。
- 全量中间运行暴露 17 项失败后，定位到旧格式夹具以及未显式冻结计划的终态夹具。后者只在八个相关合成测试中补齐当前 Native 计划前置数据；生产运行／拒绝逻辑、WAF 停止、证据与权限断言不变。没有用恢复“缺失计划自动当作 Native”来满足测试。
- 轨迹页 `useLiveTrace.ts` 首次事件订阅失败后，现在会在可见状态下每 15 秒或页面重新可见时重试；订阅单飞，恢复后补读终态最后一批数据，连续失败只提示一次，卸载期间晚到的订阅立即释放。新增三项测试修改前均失败、修改后通过。该修复只解决**注册失败后的恢复**，不代表能侦测所有传输层静默断连。

本轮验证（集合重叠，不能直接相加）：

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| 退役、当前导入、历史展示、隐私、配置与隔离 | 151 项通过 | `/tmp/oviraptor-run-retirement-focused-final.log` |
| 八项计划夹具修复及退役／残留守卫 | 40 项通过 | `/tmp/oviraptor-run-retirement-fixture-final.log` |
| 轨迹页事件与读写竞态 | 29 项通过 | `/tmp/oviraptor-trace-subscription-final.log` |
| 前端完整 UI 回归（含上述轨迹页测试） | 765 项通过 | `/tmp/oviraptor-run-retirement-ui-final.log` |
| Vue 类型检查与生产构建 | 通过 | `/tmp/oviraptor-run-retirement-build-final.log` |
| 全目标／全特性严格 Clippy | 通过 | `/tmp/oviraptor-run-retirement-clippy.log` |
| Cargo fmt／diff 空白检查 | 通过 | `/tmp/oviraptor-run-retirement-fmt-final.log`、`/tmp/oviraptor-run-retirement-static-final.log` |

失败与中止证据保留：`/tmp/oviraptor-run-retirement-lib.log` 是旧测试二进制的**中止全量运行**，计划 1481 项，停止前记录 1031 通过／17 失败，不能作全量结论。为避免继续占用 CPU，已停止本轮启动的那个测试进程。随后第一次定向运行 150 通过／9 失败，见 `/tmp/oviraptor-run-retirement-focused.log`；这些失败已在上述最终定向集合复测通过。旧 run 三项拒绝测试和轨迹恢复三项测试的失败证据分别保留在 `/tmp/oviraptor-run-retirement-red.log`、`/tmp/oviraptor-trace-subscription-red.log`。**修复后未重跑完整 1481 项 Rust 用例。**

仍须继续的工作，不可用本轮通过项替代：

1. **旧解析和存量数据退役。** 漏洞 JSON、CSV／Markdown、SQLite 轨迹、SARIF 专属旧属性、旧 ID／删除标记、数据库封存与历史展示分支仍需逐项拆除。当前精确残留登记有 62 个文件（42 个测试、11 个导入相关、5 个迁移相关、4 个历史标签）；这是待审查清单，不是永久兼容豁免，也不能覆盖没有旧字面量的旧语义。真实库／CAS 清理前仍需给出具体位置、来源、数量、共享引用和可恢复备份。
2. **聊天后端真正事件驱动。** `collaboration_events.rs` 仍每 250ms 查询数据库，前端事件优先并不等于后端无轮询。后续只在提交后通知展示层，保留持久化序号、游标补读、失败重试、批量合并与退出释放；通知不能授予执行权限，也不能提前展示回滚消息。
3. **安装日志可靠性。** 现有 stdout／stderr 流和脱敏有界展示保留，但缺安装批次身份、持久化、游标回放；晚订阅／断连仍可能漏行。页面不得把瞬时日志当成安装成功凭证。
4. **脚本生产者及桌面验收。** 尚未逐个确认所有脚本写入都经过统一通知；短周期读文件的残余路径、最终输出 flush、完成后最后一批、轮次切换、长时间运行、打包与安装态仍需检查。不得宣称用户提供的 URL 已完成验收。

### 最新追加：旧 coverage 入口退役，执行状态与活动聊天改为事件优先

**整体仍未完成。** 本节覆盖下方“旧 coverage 解析仍在”“Native 状态仍每 3s 查询”“连接正常的活动聊天仍每 3s 查询”的旧状态。旧运行/漏洞适配器、存量数据和端到端日志可靠性仍有明确未完项，不能据此宣布彻底剥离或多智能体整体验收通过。

本轮实际修改：

- 删除 `coverage.json` 的目录发现锚点、嵌套收录、直接解析调度及专属 `coverage_records` 函数，不仅是隐藏 UI。新增三个拒绝回归：不同 schema/损坏 JSON 直接调用不识别、旧文件独占目录不入库、混合目录只保留当前审计 JSON 与标准 SARIF。源文件不变、CAS/导入行数、重复导入和不可执行约束均有断言。
- 原有深度预算、损坏文件及历史台账测试迁到当前审计格式或标准 SARIF `kind=pass/open`。删除旧 coverage 接受测试和测试专用文件名常量，保留“覆盖记录不能提升为 Finding”的验证。精确残留登记只更新五个已人工审查的受影响条目，不恢复旧解析来满足旧断言。
- `NativeRunStatus.vue` 去掉 3s 自循环，复用公共 `useCommittedRefresh`：已提交协作事件按任务/尝试筛选、50ms 合并、读取期间积压一次补读、15s 丢通知补查、页面重新可见时补读、卸载释放资源。事件只提示读取，正文来自后端状态快照；返回的任务/尝试不匹配时清空状态并报错，不能据此允许人工恢复或结案。
- 活动聊天不再因为任务处于 scanning/pausing 或存在活动后续任务，就在正常事件连接之外重复 3s 查询。连接正常时同样使用事件优先及 15s 水位补查；读取失败/订阅不可用保留 3s 重试，已有分页积压仍用 50ms 补读。本轮不宣称解决了聊天订阅生命周期的全部恢复场景。
- 操作审批、目标范围、预算、恢复与结案回执校验未放宽。测试仅使用本地合成数据/替代 IPC，没有调用真实模型或远程目标，没有删除真实用户库/CAS。

最新验证（集合重叠，不能相加；不是 Rust 全量或安装包验收）：

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| 旧 coverage 入口拒绝，修改前 | 0/3，按预期失败 | `/tmp/oviraptor-coverage-retirement-red.log` |
| 导入模块 | 104 项通过 | `/tmp/oviraptor-coverage-retirement-imports.log` |
| 退役与残留登记守卫 | 29 项通过 | `/tmp/oviraptor-coverage-retirement-guards.log` |
| 当前格式台账、来源与成员关系 | 2 项通过 | `/tmp/oviraptor-coverage-retirement-history.log` |
| 执行状态事件及既有操作回归 | 49 项通过；随后纳入全量再次执行 | `/tmp/oviraptor-native-status-events.log` |
| 聊天事件、活动任务补查 | 17 项通过 | `/tmp/oviraptor-chat-events-backoff-final.log` |
| 最新前端全量 | 762 项通过 | `/tmp/oviraptor-coverage-status-ui-final.log` |
| Vue 类型检查及生产构建 | 通过 | `/tmp/oviraptor-coverage-status-build-final.log` |
| 离线、单作业、全目标/全特性严格 Clippy | 通过 | `/tmp/oviraptor-coverage-status-clippy.log` |
| Cargo fmt / diff 空白检查 | 通过 | `/tmp/oviraptor-coverage-status-fmt.log`、`/tmp/oviraptor-coverage-status-static.log` |

中间证据保留：执行状态新增事件测试在修改前失败；活动聊天的三项新回归在修改前因 3s 定时失败。首次改后聊天测试对零水位请求形式作了错误假设，已改成非零水位夹具明确验证增量请求，未更改生产游标合同。初次 759 项 UI 结果早于聊天修改，以最新 762 项为准。

主要日志/对话通路核查（不是“全仓库已实时化”的证明）：

| 通路 | 当前可确认的机制 | 尚未完成的边界 |
| --- | --- | --- |
| 任务详情 / 全局 runner 日志 | 写入后发身份提示，界面有界脱敏快照补读 | 绕开统一写入函数的脚本生产者仍须逐个核验；未证明所有文件变化即时通知 |
| 资产日志 | 数据库提交后身份通知，复用公共刷新 hook | 真实桌面断连/压力验收仍缺 |
| 安装日志 | 双管道读取、有界分帧与即时 UI 事件 | 仍是瞬时流；缺安装批次身份、持久日志、游标和重连回放 |
| Native 执行状态 | 本轮改为事件优先及低频补查 | 后端事件泵仍查询数据库，并非事务提交直接推送 |
| Agent 聊天 / 轨迹 | 已有协作事件与数据库水位读取；本轮减少正常活动聊天轮询 | 后端仍 250ms 事件泵，订阅失败后的完整重连/积压/后台页面验收尚未全部结束 |

继续顺序：先移除旧 run/vulnerability、SQLite 轨迹、SARIF 旧属性/规则前缀、来源/ID 别名与相应格式分支（不得把旧覆盖结果误升为漏洞）；再按核验过的来源和引用清单定向处理存量数据库/CAS；实时链继续补持久安装日志、脚本生产者通知、协作事件提交唤醒和桌面端断线/压力测试。不能通过删授权门禁或把旧状态默认为 Native 来完成退役。

体量：本轮新增 coverage 拒绝测试 77 行、Native 状态事件测试 74 行；状态组件 355 行、状态测试 harness 102 行，聊天事件测试 324 行。未新增通用工具大文件，也未批量删除独有回归。

### 最新追加：停止导入旧事件流，约束安装日志后端缓冲与失败语义

**整体仍未完成。** 本节覆盖下方“旧事件流仍被识别”“安装管道/单行无界”的旧状态，不代表整个旧适配器已删除、存量数据已清理或全部日志已经持久化实时展示。

本轮落实：

- `events.jsonl` / `events.ndjson` 从目录发现锚点、嵌套 bundle 文件收录及直接解析调度中移除，旧 `event_records` 函数删除。当前模型审计生产者使用 `llm-hook.jsonl`；独立的 `oast-events.jsonl` 不在本次修改范围内。三项拒绝测试先 **0/3**，再全部通过；仅旧文件的目录不入库，混合目录只收录当前审计文件，源树指纹不变。
- 截断末行、坏中间行及单行上限测试迁到 `llm-hook.jsonl`。原文一致性测试使用当前两种审计文件，检查文件总数、哈希、字节数、CAS 原文及导入前后源树不变。权限测试逐一要求六种记录类别非空后验证只读/不可执行，不再用总行数近似覆盖率；没有恢复旧入口来满足旧数量。
- 安装输出从 `commands/environment.rs` 抽出为独立 `installation_logs.rs`（95 行）及 `installation_logs/process.rs`（143 行），分帧测试独立 184 行。stdout/stderr 并行读取，共用 **64 条消息的有界通道**；单条原始记录上限 **16 KiB**。消息入队前统一脱敏、清理 ANSI 并限制为 1200 字符；失败尾部最多 24 条，最终错误最多 8000 字符。限制仅针对本模块缓冲，未对 Tauri 或系统管道证明全局内存上界。
- 超长记录整条丢弃并发出省略标记，不把原始凭据拆成多个片段展示；无换行的超长输出不会无限扩张单行内存。支持 CR 进度、CRLF、末尾无换行、Unicode 分片、正常重复行与中断读重试；保留记录边界，不宣称逐字节无缓冲推送。
- 管道读取错误不再静默吞掉；读取线程失败或读取未完成，即使子进程退出码为 0 也不能报告安装步骤成功。启动/等待错误经 UI 包装层发出脱敏错误；线程启动失败时尝试终止并回收本步骤子进程。管理员准备流程和禁止活动任务期间安装的限制未改变。
- 精确残留登记仅审查更新四个实际受影响文件。旧适配器的运行、漏洞与覆盖率分支仍是删除债务，不是永久例外。没有执行真实用户数据库/CAS 清理。

验证证据（集合重叠，不相加；不是全项目 Rust/安装包验收）：

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| 旧事件入口拒绝测试修改前 | 0/3，按预期失败 | `/tmp/oviraptor-event-retirement-red.log` |
| 导入模块完整集合最终结果 | 102 项通过 | `/tmp/oviraptor-event-retirement-imports-final.log` |
| 退役守卫及专项 | 26 项通过 | `/tmp/oviraptor-event-retirement-guards.log` |
| 安装日志资源边界、失败与本地子进程 | 10 项通过 | `/tmp/oviraptor-installation-logs-bounds-final.log` |
| 轨迹、安装禁止活动任务、源目录不变、公共脱敏 | 分别 6/1/1/2 项通过 | `/tmp/oviraptor-retirement-installation-related.log` |
| 安装日志面板、轨迹与 JSON 导入 UI | 45 项通过 | `/tmp/oviraptor-retirement-installation-ui.log` |
| 离线单作业、全目标/全特性严格 Clippy | 通过 | `/tmp/oviraptor-retirement-installation-clippy.log` |
| Cargo fmt / diff 空白检查 | 通过 | `/tmp/oviraptor-retirement-installation-fmt.log`、`/tmp/oviraptor-retirement-installation-static.log` |

中间失败记录保留：首次导入扩展回归 99 通过/3 失败，原因是旧文件数量与当前审计路径的夹具假设；首次安装日志编译有 Rust 可见性错误，已修复。最终结果使用表中 `final` 日志，不能混用中间输出。本轮没有重跑 Vue 生产构建或 UI 全量；此前结果仍只是此前证据。

下一步未完成项：

1. 旧 run/vulnerability/coverage 解析、历史 SQLite 轨迹、SARIF 旧属性及旧来源/ID 别名仍在。继续将共用完整性测试迁到当前数据后删除旧分支；禁止把覆盖记录错误提升为漏洞。
2. 存量数据库、旧投影及 CAS 原文未清理；停止新导入不等于已有旧记录被删除。需明确来源、逐表数量、引用与备份恢复范围后定向清理。
3. 安装日志仍是瞬时 UI 流，缺批次身份、持久化、游标回放及发送失败后的补读。本轮不保证订阅断开期间无日志丢失。安装前置检查等非步骤级错误尚未全部统一审计。
4. Agent 协作后端 250ms 数据库事件泵、Native 状态 3s 查询及脚本文件生产者通知本轮未改动；用户指令、多智能体联合链、真实桌面/安装态/长时间压力验收尚不能判定完成。

本轮仅本地修改与合成数据/本地打印子进程测试；没有运行真实安装器、下载工具、访问外部 URL、部署或削弱授权/范围/审阅限制。

### 本轮追加：解除当前审计日志对旧适配器的依赖，关闭旧审计文件入口

**整体仍未完成。** 本节只确认审计导入边界的退役与当前解析保留，不是整个旧结果导入器删除，也不是实时聊天链路完成。

- 公共有界 JSON/JSONL 解析从 `adapters/legacy_strix.rs` 提取至独立 `adapters/json_records.rs`（123 行）。`model_audit.rs`（57 行）不再引用旧适配器；调度中的旧 producer 解析改为惰性求值，当前审计路径不调用它。旧格式专属解析仍在原模块，没有通过改名掩盖残留。
- 旧提示词审计文件名不再作为发现锚点、不再进入当前 bundle 的文件清单，直接调用 `parse_bundle` 也不能解析它。两项拒绝测试修改前 **0/2**、修改后通过；与当前文件并存时，仅当前文件入库，源文件树逐字节/元数据指纹不变。
- 当前 `model-prompt-audit.json` 与 `llm-hook.jsonl` 保留；核对实际写入入口使用的也是当前文件名。原审计与命令层夹具改用当前名称，保留脱敏、来源、用量、只读和不可执行断言，没有恢复旧别名来满足测试。
- JSONL 去掉整份字符串的额外复制和全量行引用数组，改为逐行带预读迭代；测试覆盖重复消息、Unicode、物理行号、会话隔离、坏中间行、截断末行、单行字节上限及 JSON 深度。没有做内存或时延基准，不据此声称量化性能提升。
- `model_audit.rs` 与 `discovery.rs` 的旧字面量归零并从残留登记移除；仅逐文件更新实际修改的登记项。`discovery.rs` 仍识别其他旧格式文件名，字面量归零不代表该模块已经完成全部退役。

验证证据（集合重叠，不相加）：

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| 导入模块完整集合 | 99 项通过 | `/tmp/oviraptor-audit-import-retirement-imports.log` |
| 退役专项及残留守卫 | 23 项通过 | `/tmp/oviraptor-audit-import-retirement-guards.log` |
| 轨迹读取相关 | 6 项通过 | `/tmp/oviraptor-audit-import-retirement-trace.log` |
| 轨迹展示隐私 | 6 项通过 | `/tmp/oviraptor-audit-import-retirement-privacy.log` |
| 轨迹/导入 UI 相关 | 39 项通过 | `/tmp/oviraptor-audit-import-retirement-ui.log` |
| 全目标/全特性严格 Clippy、Cargo fmt、diff 空白检查 | 通过 | `/tmp/oviraptor-audit-import-retirement-clippy.log`、`/tmp/oviraptor-audit-import-retirement-fmt.log`、`/tmp/oviraptor-audit-import-retirement-static.log` |

本轮没有重跑全部 Rust/UI 或重新打包；上轮的全量 UI 与构建证据不冒充本轮完整验收。旧 `run.json`、漏洞/覆盖率/事件格式、SQLite 历史轨迹、来源/删除别名、SARIF 专属字段和存量库迁移仍需继续拆除；现有旧格式测试通过仅说明本轮未产生额外破坏，不是必须保留这些旧功能的要求。

没有删除真实原件或已导入记录，没有外部测试、部署、模型/目标调用或放宽授权约束。聊天 250ms 事件泵、状态查询、安装日志持久化和脚本通知的剩余问题在本轮没有解决，整体目标保持未完成。

### 本轮追加：停止解析旧后端别名，拒绝损坏记录的 Native 兜底

**整体仍未完成。** 本节覆盖下方“contract 仍接受旧 backend 别名”的旧状态；没有把旧导入器、存量库迁移或所有日志实时链路标为完成。

已落实：

- `agent_runtime/contract.rs` 不再通过字符串解析或 serde 接受旧后端名称及其容器别名。当前 Native 表达保留；`legacy_backend_removed` 仅保留为不可执行的拒绝标记，不能算作最终旧类型清零。
- `agent_runtime/store.rs` 读取未知后端返回通用错误，不再默认为 Native，也不在错误中回显原始字段。`runtime_adapter` 拒绝非 Native 报告及已有非 Native 运行记录，不新建活动记录、不改写原有状态/预算。
- 矩阵解析不再丢弃无效目标后接受剩余目标。新的 `commands/agent_backend_storage.rs` 区分不存在和读取/解析失败，核验 scan、attempt、目标唯一性以及目标行与计划 JSON 的后端/轮次/URL 一致性。权威矩阵损坏时，不得回退到有效 checkpoint；Native 矩阵也不能覆盖损坏的目标行。
- `agent_backend_choice` 与 `runtime_report` 不再把无法验证的已存后端/计划转换为 Native。续跑缺少可验证父计划时拒绝继承；当前 Native JSON 的存储、报告和读取往返不改写原值。
- 专属测试初始 **0/9 通过**，确认原缺陷可复现；修复及增加当前格式/作用域测试后 **12/12 通过**。扩大续跑测试首次 **12/13**：原暂停夹具未冻结当前轮次计划，依赖了已删除的默认行为。按真实流程补齐夹具后 **13/13**，保留暂停状态、预算继承和请求数量断言；没有恢复生产兜底。
- 两个生产文件的旧字面量已归零并从残留登记移除；仅按逐文件审查更新相关负向测试条目，未批量放宽登记。该登记通过不等于所有旧代码已删除。

验证证据（集合重叠，不相加；不是全部 Rust 测试通过声明）：

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| 本轮退役专项 | 12 项通过 | `/tmp/oviraptor-backend-alias-retirement-focused-final.log` |
| 运行时基础回归 | 55 项通过 | `/tmp/oviraptor-backend-alias-retirement-runtime.log` |
| 退役登记及拒绝守卫 | 21 项通过 | `/tmp/oviraptor-backend-alias-retirement-guards.log` |
| 后端选择/冻结/恢复相关 | 22 项通过 | `/tmp/oviraptor-backend-alias-retirement-backends.log` |
| 续跑、fresh、冻结计划、默认策略、报告 | 分别 13/4/2/1/1 项通过 | `/tmp/oviraptor-backend-alias-retirement-related-final.log` |
| UI 全量 | 752 项通过 | `/tmp/oviraptor-backend-alias-retirement-ui.log` |
| 离线、单作业、全目标/全特性严格 Clippy | 通过 | `/tmp/oviraptor-backend-alias-retirement-clippy.log` |
| Vue 类型检查与生产构建 | 通过 | `/tmp/oviraptor-backend-alias-retirement-build.log` |
| Cargo fmt / diff 空白检查 | 通过 | `/tmp/oviraptor-backend-alias-retirement-fmt.log`、`/tmp/oviraptor-backend-alias-retirement-static.log` |

模块体量：持久化校验独立文件 111 行；两份退役测试分别 126、227 行。没有删除现有独立覆盖的测试，也没有宣称既存大文件已全部拆分。

仍须继续：

1. `artifact_import/adapters/legacy_strix.rs`、其调度、旧文件名发现和来源分支仍在；其中公共解析与当前 Native 导入共用，须拆分引用后删除旧格式，不能只改名冒充退役。
2. `db_neutral.rs` 等旧库迁移、旧默认值修复及中性拒绝标记仍在。真实数据库/目录未清理，清理前仍需明确来源、数量、引用、备份与恢复范围。
3. 聊天后端 `collaboration_events.rs` 仍是 250ms 事件泵；Native 状态卡仍有 3s 查询。runner/资产的提交后通知不能作为全部日志、脚本和 Agent 对话已实时化的证明。
4. 安装日志仍缺批次持久化/游标补读，子进程管道的无界缓冲与部分早期错误路径待审计。真实桌面、安装态、长时间运行验收尚未完成。

本轮仅修改退役/拒绝逻辑和本地回归夹具；未部署、未请求外部目标、未删除用户数据库，也未放宽授权、范围或审阅约束。

### 本轮追加：资产日志提交通知、公共脱敏与安装日志缓冲

**整体仍未完成。** 本节覆盖下方“资产日志尚未事件化”“安装日志无界缓存/订阅阻塞初始化”的旧状态；不是 Strix 零残留证明，也不是全部聊天/脚本输出实时化或安装日志完整持久化验收。

已落实：

- 资产日志唯一生产写入入口 `jobs::log_line` 改用 `asset_logs::append`：在事务内写入脱敏日志及读取所属项目，成功提交后才发送 `asset-log-committed` 身份提示，不广播日志正文。插入或延迟外键提交失败不通知；双连接测试确认回调发生时另一连接已能读取记录。
- 资产日志查询保持项目与 run 过滤交集及真实 ID 倒序；服务端默认 500 条、上限 2000 条，UI 请求 300 条。旧原始行读取时脱敏，不改写其存储。不把 `job-progress` 事件拼成负 ID 假日志，不按文本去重真实重复记录。
- `useCommittedRefresh` 由 runner 与资产日志实际共用：50ms 合并事件、15s 兜底补查/订阅重试，后台或非活动页面暂停自动读取，重新进入补查。快照有代次及项目隔离；错误保留上次快照，不显示原始传输异常。增加响应结构、ID 严格倒序及容量校验。
- 新测试复现同一 Vue tick 内 A→B→A 导致清空后漏补读的问题；共享 scope watcher 改为同步观察每次变化。专项初跑 **43/44**，修复后 **44/44**，没有弱化测试预期。
- 公共 `log_display` 供 runner、资产和安装日志复用：先去除现有支持的 ANSI/回车、按已知字段脱敏，再限制字符数。新增 FOFA key 字段识别；安装事件正文、进程错误尾部及组合错误使用该边界。不是对所有任意格式秘密的识别保证，也没有宣称所有安装早期错误返回路径均已审计。
- 安装日志订阅抽到独立 `useInstallLogPanel` 生命周期，不再阻塞 App 初始化。保留最近 300 条、消息最多 1200 字符，报告移出视图数量，保留相同消息并用本地单调 ID 标识；读取旧日志时不强制滚到末尾。订阅失败独立提示并重试；卸载和晚到订阅释放资源。
- 安装日志是**瞬时事件流**，尚无持久化快照或安装批次 ID。曾发生订阅错误就持续提示可能缺失，重连/清空显示不伪造补齐。`complete/success` 事件不再决定 App 的安装成功状态；本窗口 API 调用及环境校验返回才决定。安装调用失败显示通用错误，避免直接渲染原始 IPC 异常。

验证证据（各集合重叠，不能相加）：

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| Rust 资产/公共脱敏/runner/退役守卫 | 29 项通过 | `/tmp/oviraptor-asset-install-log-rust-final.log` |
| UI 全量，含新增 9 项资产及 6 项安装日志 | 752 项通过 | `/tmp/oviraptor-asset-install-log-ui-final.log` |
| Vue 类型检查与生产构建 | 通过 | `/tmp/oviraptor-asset-install-log-build.log` |
| 离线、单作业、全目标/全特性严格 Clippy | 通过 | `/tmp/oviraptor-asset-install-log-clippy.log` |
| Cargo fmt / diff 空白检查 | 通过 | `/tmp/oviraptor-asset-install-log-fmt.log`、`/tmp/oviraptor-asset-install-log-static-final.log` |

代码组织：新增 `asset_logs.rs` 85 行、其测试 137 行、`log_display.rs` 50 行、公共刷新 hook 63 行、资产 hook 43 行、安装 hook 49 行；runner 订阅 wrapper 24 行，复用公共刷新而非再复制一套。资产/安装测试分别 116/90 行，已接入现有 UI 门禁，不新增重复执行。既存 `App.vue` 704 行、`jobs.rs` 1284 行、`commands/environment.rs` 624 行仍属结构债；本轮独立状态/查询/脱敏职责已外移，没有声称整个工程已符合体量要求。未删除具有独立覆盖的测试。

当前仍须继续，不能勾选完成：

1. `agent_runtime/contract.rs` 仍接受旧 backend serde/字符串别名；`db_neutral.rs` 仍有旧表与旧记录封存迁移；旧格式 reader、来源展示、发布说明等仍有旧后端内容。删除前须保证旧状态不会因默认分支变成 Native 可执行任务。退役登记通过不等于文本或兼容分支清零。
2. `collaboration_events.rs` 仍有 250ms 数据库事件泵；`NativeRunStatus.vue` 仍有 3s 查询。详情 mailbox/tool history、列表状态和脚本直接写文件的生产者通知覆盖未全部完成。15s fallback 是丢通知补查，不是正常实时路径的延迟承诺。
3. 安装日志尚未做持久化、批次关联、游标补读；后端 stdout/stderr 管道队列仍无界，单行读取也尚未做流式容量控制。瞬时 IPC 事件丢失且不报订阅错误时，当前 UI 不能检测或恢复缺失；不得称为完整可靠日志系统。
4. 真正的旧数据删除尚未进行；需要明确库/目录、来源条件、引用与数量、备份及恢复清单。不删真实数据库、不按关键词无差别清空当前任务/资产/证据。
5. 真实桌面、安装包、长期运行和全部日志端到端延迟尚未验收。本轮没有安装/部署、外部 URL 探测、工具自动下载、扫描执行能力增强或授权/范围/审阅门禁放宽。

### 本轮追加：配置提升链退役与全局日志页事件化

本节覆盖下方“旧配置仍提升为当前字段”“全局 runner 日志尚未处理订阅/快照竞态”的状态。**整体仍未完成：不等于仓库所有 Strix 字段、旧记录和全部日志来源已清零/实时化。**

已落实的配置边界：

- 删除 `db_migrations/legacy_strix_to_native.rs`，改用 61 行 `db_settings.rs`。不再提升旧模型、密钥、profile、策略、预算或目录别名；顶层旧后端命名前缀（不区分大小写）及过渡目录/策略字段直接丢弃，不替换成可执行的新配置。
- 配置 API 读写及启动归一化共用此边界。当前 `modelProfiles`（包括空数组、null 等显式清空）不被平铺缓存重新填充；当前平铺 Native 模型字段仍可生成当前 profile，不能借用旧模型或旧密钥。
- 仅清理配置顶层命名空间，不递归改写嵌套用户文档。数据库清理继续使用跨 profile 事务：任一写入失败则全部回滚。没有打开、修改或清理用户真实数据库。
- **行为变更：只有旧 Strix 字段的配置不再是可用模型配置，需要重新填写当前模型名称、地址和密钥。** 这是取消兼容的明确结果，不应恢复旧映射来“修好”空配置。
- 配置测试保留原有本地/云端密钥隔离、显式地址要求和上下文覆盖清理断言；用当前字段构造这些有效配置。旧 profile 能激活模型的断言已替换为拒绝激活；API 清理期望值独立书写，不再调用被测归一化函数生成预期。

已落实的全局日志页：

- 新增 55 行 `useRunnerLogPanel.ts`，复用任务详情的 `useLiveRunnerLog`。订阅在组件生命周期内独立建立，不再等待 App 全量刷新后才注册。
- 事件仅作“已持久化内容发生变化”的提示，读取后端脱敏尾部快照（300 行）；不直接显示事件正文，不按文字去重连续日志。50ms 合并突发事件；15s 仅用作补查和订阅重试。
- 固定轮次严格隔离；“最新轮次”接受本任务有效新轮次提示后重新读取最新记录，不会卡在旧轮次。任务/轮次切换及清空选择立即使旧请求失效；失败保留当前快照，原始传输错误不进入 UI。
- 包含订阅晚到补读、手动读取期间通知保留、后台/非日志页面暂停、返回补查、失败订阅恢复和卸载释放。App 仍保留其他业务事件的原有生命周期，此处不宣称安装/资产事件链已修复。

验证记录：先新增 5 个配置用例，旧实现 **3 通过、2 失败**，复现旧配置激活及借用旧模型名；实现后首轮配置专项 **10/10**。扩大集合初次 **74/78**，4 个失败来自仍依赖旧字段的测试夹具，已改用当前字段并保留业务/隔离断言；最终相关 Rust 集合 **78/78**，包含配置、退役登记、当前 JSON 导入导出和日志。新全局日志回归 **9/9**，连同任务详情专项 **30/30**；完整 UI 初次 **732/737**，5 个失败是项目删除夹具未隔离新日志模块，修正后 **737/737**；最终类型检查/生产构建、离线单作业全目标/全特性严格 Clippy、Cargo fmt 与 `git diff --check` 均通过。上述集合存在重叠，不累加测试数；不是完整 Rust 或真实桌面验收。测试与检查进程均已退出。

证据文件：`/tmp/oviraptor-settings-retirement-red.log`、`/tmp/oviraptor-settings-retirement-focused.log`、`/tmp/oviraptor-settings-retirement-verified.log`（中间失败）、`/tmp/oviraptor-settings-retirement-final.log`、`/tmp/oviraptor-global-log-focused.log`、`/tmp/oviraptor-settings-global-log-ui.log`（中间失败）、`/tmp/oviraptor-settings-global-log-ui-final.log`、`/tmp/oviraptor-settings-global-log-build-final.log`。首次 fmt 从仓库根调用而非 Rust 子目录导致命令路径失败；纠正目录后新模块的排版差异已由 formatter 修正。

最终静态检查日志：`/tmp/oviraptor-settings-global-log-clippy.log`、`/tmp/oviraptor-settings-global-log-fmt-final.log`。没有添加 warning 抑制或放宽退役检查范围。

本轮没有部署/外部目标测试、真实旧数据删除、授权/范围/审阅保护调整或多智能体执行能力扩展。退役登记只更新已复核的文件，登记通过不证明零残留。

仍须继续：旧格式解析器、serde 别名、旧数据视图和旧记录封存/迁移的定向退役；真实数据删除清单与恢复方案；安装日志与资产日志的持久化通知/脱敏/有界缓存/重连；详情 mailbox/tool history 自动更新；Agent 对话后端 250ms 数据库事件泵改为生产者通知及端到端验证；脚本直接写文件的通知覆盖；状态卡/列表补齐通知；实际桌面、安装包和长时间运行验收。

### 2026-09-30 续做：后端隐式导入退役与任务详情日志事件化

本节覆盖下方较早批次中“后端同步仍存在”“任务详情未接事件”的状态；**不是 Strix 所有字段/旧数据清零，也不是全部日志实时化完成。**

- 已删除 `sync_sentinel_results`、`run_artifact_import` 两个隐式目录导入 IPC 的注册与实现，移除 `result_ingestion_sync.rs`、`historical_artifact_roots` 和旧目录 home 展开逻辑。应用不再通过这两个命令枚举旧默认目录或配置中的旧根。共享的跨平台用户目录函数迁到环境模块，行为不变。
- 专属自动同步测试随功能退役删除；项目删除的表快照助手独立为中性测试模块，并扩大为全部非 SQLite 内部表。仍保留的导入隔离/只读投影测试只传显式临时目录，不再依赖生产隐式同步入口。通用导入库仍被当前 Native 源码报告与 JSON 使用，未整体删除；其中 Strix 格式识别仍待单独退役。
- 任务详情日志新增 `useLiveRunnerLog.ts`：只在执行详情可见时处理当前任务/轮次通知，50ms 合并、串行尾读、手动读取期间保留通知、后台暂停/前台补读、晚到订阅补读、失败订阅重试、15s 兜底及卸载清理。事件正文不直接进入 UI；复用当前脱敏尾读接口，最多 200 行。自动刷新保留已有快照，读取错误与连接降级提示分开，恢复订阅清除降级提示。
- 上述是持久化日志的通知后快照刷新，不是模型逐 Token 流；直接写文件但未经过 `append_runner_log` 的生产者尚未全部覆盖，不能把 15s 补查说成实时。

最终验证：相关 Rust 回归 **164/164**（覆盖通用导入、当前 JSON 导入导出、只读隔离、任务/项目删除、日志、退役登记与静态残留）；完整 UI **728/728**；详情专项 **21/21**（包含新增实时日志 **11** 项）；类型检查/生产构建、离线单构建作业全目标/全特性严格 Clippy、Cargo fmt 和 `git diff --check` 均 exit 0。静态登记只证明已登记边界符合当前源码，**不证明兼容字面量为零**。本批小模块：实时日志 hook 77 行、详情状态 205 行、事件用例 124 行、夹具 46 行。

验证过程透明记录：第一次入口编译检查暴露 6 个旧测试调用者，均改为显式临时目录；严格检查随后暴露 2 个生产不再消费的临时统计字段和 5 处测试借用切片的冗余 clone，已修复，未添加 `allow` 掩盖。生产状态计算、持久化修订/撤销记录及提交回执保留，临时输出计数仅供测试诊断。扩大到 616 项命令层检查时，**314 项通过后主动 SIGINT 停止**耗时的无关源码审阅恢复测试，exit 101 来自该中断；这不是完整 Rust 通过结果。随后重新编译并跑完上述 164 项相关集合，各集合有重叠，不累加测试数。

最终日志：`/tmp/oviraptor-retired-import-focused-final.log`、`/tmp/oviraptor-task-log-events-final.log`、`/tmp/oviraptor-retired-import-live-log-ui-final.log`、`/tmp/oviraptor-retired-import-live-log-build-final.log`、`/tmp/oviraptor-retired-import-live-log-clippy-verified.log`、`/tmp/oviraptor-retired-import-fmt-final.log`。未完成的扩大检查保留在 `/tmp/oviraptor-retired-import-commands.log`；`clippy.log` / `clippy-final.log` 为上述中间失败，`clippy-verified.log` 才是最终通过快照。本批测试进程已退出。

没有部署、外部目标探测、执行真实任务、删除真实数据库或磁盘原件，也没有移除授权/范围/租约/审阅保护。未做真实桌面、安装态或长期运行验收，不能把工作区构建成功当成已安装应用更新。

本批剩余边界：Strix 解析分支与旧 serde/配置别名、旧数据只读视图、数据库迁移和真实清理清单；全局日志启动订阅竞态与断线补齐；Native 状态卡/任务列表/脚本日志生产者实时链路；真实桌面、安装版本与长时间运行验收。不得以残留白名单或旧历史测试通过替代“已完全剥离”。

### 最新需求变更：取消历史兼容，逐路补齐实时展示

用户已取消 Strix 历史兼容要求；Master Plan §0、§18.3/4 和兼容文档已明确废止旧合同。**当前只完成下面列出的增量，不是所有旧字段/数据清理或全部日志实时化。** 下方兼容测试结果为旧批次证据，不再作为必须保留旧功能的条件。

本批变更：

- 删除任务页 `useInitialHistorySync` 及其启动/激活钩子、会话标记、专属加载提示和前端同步 API。移除 8 项被明确退役的自动导入生命周期测试，替换为启动/两次激活不调旧 importer、不安排目录计时器、不写会话状态的运行时测试；保留 Native 定时刷新及隔离回归。该入口后端仍存在，不能宣称全链路删除。
- 轨迹页改为订阅已有 `nest://collaboration-event`，50ms 合并提示、串行读取、在途提示补查；15s 定时器仅作丢事件补查。覆盖晚到订阅、隐藏恢复、完成后迟到行、手动加载及其补查期间再次到达的事件、无效/其他任务事件、卸载和初次读取失败。仍是通知后读取现有详情快照接口，不是新增逐 Token 流，也没有修改任务执行器。
- 聊天与轨迹复用 `timeline/eventContract.ts` 的通知形状校验，通知不能替代数据库内容或执行权限。
- 修复实时日志的展示差异：`append_runner_log` 原先把未脱敏原文发往 UI；实时事件与磁盘尾读现共用展示过滤（ANSI 去除、凭据脱敏、1200 字符上限）。原证据文件不改写；此修复不等于任意未知秘密都可自动识别。

当前检查证据：任务页专项 **80/80**；最终完整 UI **717/717**；Rust 日志专项 **8/8**（含本批新增两项）、既有活动残留静态基线 **1/1**；类型检查/生产构建、离线单作业全目标/全特性严格 Clippy、Cargo fmt 与 `git diff --check` 均 exit 0。**静态基线只检测已登记的活动执行路径，不代表历史字段/兼容已清零。** 未重跑 Rust 全量、真实桌面或安装态。没有访问外部目标、启动扫描、部署或删除真实旧数据。

日志：`/tmp/oviraptor-native-only-startup.log`、`/tmp/oviraptor-trace-events-{red,green}.log`、`/tmp/oviraptor-native-realtime-ui-final.log`、`/tmp/oviraptor-native-realtime-build-final.log`、`/tmp/oviraptor-realtime-log-privacy-rust.log`、`/tmp/oviraptor-native-realtime-{clippy,fmt,residual}.log`。红灯包含旧 4s 计时器与新验收不符；不加 `-final` 的 UI **716** 项和 build 是追加最后一条在途补查测试之前的通过快照，不是失败记录。

本批小模块行数：实时轨迹 91、轨迹选择 85、共享事件合同 15、日志展示函数 14；新增轨迹事件测试 131 行/13 项，日志测试文件 239 行；刷新测试精简至 272 行。`SentinelBoard.vue` 仍为 3553 行，`App.vue` 等既存巨型文件也仍是结构债，不因此宣称拆分完成。原始日志展示长度函数从旧目录兼容模块移出，避免当前日志继续依赖该旧职责。

#### 剥离剩余项（源代码已定位，尚未完成）

| 范围 | 已定位入口 / 处理边界 |
|---|---|
| 后端导入 | 两个隐式目录导入命令及注册已退役；只读状态接口和通用导入库仍存在，后者与当前 Native JSON/SARIF 共用，需继续拆除 Strix 格式识别。 |
| 默认旧目录 | `historical_artifact_roots`、旧配置目录读取及 home 展开已删除；尚不能据此断言所有模块的旧路径兼容都已清零。 |
| 历史 UI/类型 | `HistoricalJsonImport`、导入台账/预览、`historical_external` 轨迹及旧阶段标签仍存在；不可仅改名成 Native。 |
| 数据库/配置 | `db.rs`、`db_migrations/legacy_strix_to_native.rs` 仍有旧字段提升与后端退役逻辑；先明确旧库拒绝/清理边界再删，不能误恢复旧状态。 |
| 真实数据 | 未核验用户数据库的逐表归属/共享引用与磁盘清理清单，未执行删除。 |

#### 实时展示检查矩阵（不是全产品完成声明）

| 入口 | 当前已观察到的链路 | 剩余验收 |
|---|---|---|
| Agent 聊天 | 协作事件通知 → 校验后的 DB 增量；50ms 合并，3/15s 补查 | 真实桌面断线、长期运行与全消息来源覆盖；后端通知泵仍为 250ms DB 查询，不是纯生产者直接推送。 |
| Agent 轨迹 | 本批新增事件驱动；15s 兜底、切换任务隔离、后台恢复补查 | 真实桌面延迟/压力；现有详情接口仍读快照，长轨迹增量分页待评估。 |
| 全局任务日志 | `nest-runner-log` 事件追加，最多 300 行；本批统一生产者脱敏 | 订阅在启动刷新后才安装；尚缺游标/重连补齐及历史尾读与实时追加的竞态验收。 |
| 任务详情执行日志 | 本次续做接 `nest-runner-log`；50ms 合并提示后读取脱敏末尾 200 行，15s 兜底；任务/轮次隔离与订阅恢复 | 合成组件回归已覆盖竞态与清理；真实桌面延迟/重连、直接写文件的日志生产者和高负载待验收。 |
| 任务详情通信/工具历史 | `useTaskExecutionDetails` 初次或手动读取各轮 mailbox/tool 分页 | 这里并非聊天主面板；仍需事件补查，不能用 AgentDialog 实时能力替代此项验收。 |
| Native 状态卡 | 3s 定时读取 | 尚未事件驱动。 |
| 任务列表 | 12s 数据库刷新 | 尚未按任务事件局部刷新。 |
| 安装日志 | `environment.rs` 从 stdout/stderr 发事件；App 在初次刷新前订阅 | 当前事件直接带 message，App 的 `environmentInstallLogs.push` 没有此处可见的截断；需统一脱敏、有限缓存、失败订阅/晚到卸载和恢复验收。 |
| 资产任务日志 | App 在初次刷新后订阅 `job-progress`，展示限 300 行 | `jobs.rs::log_line` 只写 SQLite，`set_progress` 才发事件，不能假设每条持久化日志都已推送；需独立持久化日志游标/事件、脱敏和补齐竞态验收。 |

以下为较早批次记录；“本轮/最新”均指当时快照，不覆盖顶部状态。

最新人工操作错误隐私增量：发送/确认未知失败、取消、当前回执核对、历史回执核对和补充预览六个入口，不再直接展示后端异常原文。已知路由/接收方/租约拒绝仍提供固定建议；未知写入结果提示刷新核对，不声称提交一定未发生。保留输入、已核验状态、迟到回包隔离及不自动重放语义，不改变后端门禁。**这不是新增幂等协议或阻止人工重复点击的证明。** 两个业务模块 **277/42 行**，新增业务测试 **61 行 / 16 项**，复用原入口和夹具。

本轮门禁终态：完整 UI **711/711**、类型检查/生产构建、离线单作业全目标/全特性严格 Clippy、Cargo fmt 与 `git diff --check` 全部 exit 0。日志 `/tmp/oviraptor-action-privacy-{red,green,ui-final,build,clippy}.log`；`red` 为预期 12 失败/4 通过，`green` 为保留的中间失败，并非最终通过。中间失败来自两条旧测试仍要求异常原文；其中补充预览测试未注册失败清理而遗留轮询，已只终止该测试子进程并补 `t.after(unmount)`，随后完整复跑通过。当前无该测试残留进程。未重跑 Rust 全量、安装包/真实桌面或外部目标；整体 Master Plan 未完成。详见聊天异步审计顶部。

### 2026-09-30：协作体验续做与发布证据复核

本轮继续实现，并非暂停开发。聊天消息的事件类型与 ID 现统一按二元组编码；增量缓存、翻页锚点、已显示消息判断及 Vue 渲染键共用业务合同，避免合法字段中的冒号/连字符导致消息相互覆盖或锚点跳转。两项用例先红后绿，另补真实模板键断言；新增测试 **61 行 / 3 项**，复用既有入口与夹具，不增加依赖、执行权限或数据库格式。详见聊天异步审计顶部。

消息身份修复快照的完整 UI **695/695**、类型检查/生产构建均 exit 0；发布比对工具合成测试 **23/23**。首轮 UI **694/695** 的失败是旧用例写死旧锚点编码，现使用同一身份函数，保留原“不能误标未显示消息已读”断言。日志：`/tmp/oviraptor-timeline-identity-{red,green,ui,ui-final,build}.log`、`/tmp/oviraptor-closeout-release-tools.log`。未重跑 Rust 全量、未打包/安装或访问外部目标。更新的前端结果见顶部。

本地时间 **2026-09-30 01:11:48（UTC+8）** 的只读安装比对 exit 2：当前源码输入不匹配旧构建记录；旧构建包仍匹配其记录；已安装 `/Applications/Oviraptor.app` 不匹配该构建包。16/15 个文件、5 项差异包括可执行文件、浏览器配置资源、两个 worker 和签名资源。记录在 `/tmp/oviraptor-closeout-installation.json`。这是本地观察，不是签名信任、源码溯源或运行时验收；此后又有本轮前端修改，更不能把旧安装包当作当前源码。**§18.15 未通过。**

#### §18 发布合同逐项状态（不折算虚假的完成百分比）

下表区分已有实现与当前工作区完整证明；下方旧阶段表中的“主路径完成”不等于该发布条件通过。

| 条件 | 当前证据与剩余边界 |
|---|---|
| 1 四种模式 Native | 路由已覆盖 Web/Code/Greybox/CI；不是四种模式环境端到端验收证明。 |
| 2 无可执行 Strix 路径 | 源码退役专项与残留检查已有通过记录；当前发布包和安装态未验收。 |
| 3 旧兼容退役 | 用户已撤回历史兼容要求。旧 importer 通过记录不再是保留依据；配置、字段、导入、历史 UI 和目录回退仍需退役。 |
| 4 旧库处理/当前数据保全 | 需要可重复拒绝或定向清理旧数据且不影响当前数据；不得用旧兼容迁移测试证明新合同完成。 |
| 5 控制面与预算 | lease/fencing/owner/调度/预算模块和专项已存在；当前全量及故障矩阵未重新收齐。 |
| 6 独立角色 run | 有 child run 与条件式身份/Reviewer 链；不代表任意角色请求都能独立执行。 |
| 7 动态补充协作 | 部分 EvidenceGap 流程已有实现；通用多角色人工提案仍明确 deferred，不能宣传任意任务自主闭环。 |
| 8 并发与限流 | 有 lane 和调度回归；本轮未执行当前并发专项，不把测试文件存在当作通过。 |
| 9 独立 Reviewer | 有候选门禁及回归；本轮未重新验收全部发布入口。 |
| 10 结果证据绑定 | 已有绑定合同和原子发布回归；当前跨入口完整证明未收齐。 |
| 11 失败与恢复 | 已有恢复/隔离测试；UI 迟到响应隔离不证明真实进程崩溃后请求、费用及副作用都不重复。 |
| 12 自定义指令权限 | 保留两阶段确认与权限校验；提示词和智能体相互监督不能替代权限门禁。 |
| 13 真实协作 UI/隐私 | 最新 UI 711 项通过，历史外层私密消息专项已通过；尚非真实桌面全链路和所有自由文本隐私验收。 |
| 14 全量与包门禁 | 最新 UI/构建、前序 Rust 专项与静态检查通过；修改后的 Rust 全量及发布包验收未完成。 |
| 15 安装态对应当前源码 | 只读比对已明确不匹配；阻断发布声明。 |

人工提案的具体限制位于 `src-tauri/src/agent_runtime/multi_agent/directive/proposals/scheduling.rs`：当前只接受一个受支持角色的只读建议；直接要求 Reviewer 会返回 `proposal_reviewer_requires_frozen_candidate`，多角色或其他角色返回 `proposal_role_decomposition_required`。这不否定已有调度链，但不能描述为“用户随意发一句话，所有角色就自主完成任务”。本轮没有扩展目标触达、解除门禁或将建议转换为攻击执行。

后续普通产品可靠性验收必须分别给出：人工消息收到/待确认/被拒绝/建议生成/执行回执的真实状态；任务、轮次与线程隔离；失败后显式重试且不重复提交；归档后只读、继续任务另建关联；知识条目带来源、适用范围、审阅状态、版本及淘汰规则。此清单是验收要求，不是声称这些能力已全部实现。安全门禁与产品状态提示应分开，不能用其他 Agent 的赞同代替授权。

以下为前序增量记录；测试数字、文件行数与构建结果均属于各自快照。

本轮历史消息外层隐私增量：真实合成 SQLite 导入与读取回归确认 `channel: analysis` 在正文提取时丢失，私有内容可进入展示回包。先红 **4 通过 / 2 失败**；修复为在完整消息边界判断隐私标记，并使固定隐藏提示可重复过滤。预览函数迁入已有隐私模块，`scan_lifecycle.rs` **966 → 932 行**、隐私模块 **99 → 150 行**、原业务测试 **67 → 181 行**；没有新增文件、依赖、公共工具或测试入口。最终隐私专项 **6/6**、相关轨迹 **15/15**、退役登记 **9/9**、残留检查 **6/6**，全目标/全特性严格 Clippy、Cargo 格式检查、两个修改的 include 文件独立 rustfmt 与空白检查均通过；测试筛选有重叠。日志 `/tmp/oviraptor-trace-envelope-{red,green,green-final,related,retirement,residual,clippy}.log`（`green` 是保留的中间失败，最终通过为 `green-final`）。公开回答、事件计数、原始 SQLite 与 canonical 修订保留；未改执行器、模型输入或门禁。未重跑 Rust 全量/UI 套件、部署或访问外部 URL；§18.13 及整体 Master Plan 仍未完成。详见 UI 模块边界审计顶部。

本轮收尾验证：历史导入器 **30/30**、全目标/全特性严格 Clippy、Cargo 格式检查、两个新增 include 文件的独立 rustfmt 检查及 `git diff --check` 均通过。导入器和 Clippy 日志为 `/tmp/oviraptor-history-views-{importer,clippy}.log`。未重跑完整 Rust 或 UI 套件；本轮前端代码未变更。

本轮历史展示合同与结构增量：先复现三个读取入口接受未知 canonical schema 或数字权限标记的问题，再补齐 schema 与 JSON 布尔类型校验。`artifact_import_status.rs` **518 → 278 行**，历史展示迁入 **275 行**业务模块；**148 行 / 3 项**测试复用原导入夹具，覆盖非法声明、只读连接以及有效任务中排除无效候选。历史相关回归 **47/47**、包导入 **12/12**、退役登记 **9/9**、残留检查 **6/6** 均 exit 0；各筛选集合可能重叠，不累计为独立测试总数。仅逐项审查并更新迁移模块与前轮隐私测试两个登记摘要，未扩大豁免；原兼容文档已改为历史产物只读边界，不再指导恢复旧 CLI。未改变执行器、运行门禁、历史原件或 Native 权威状态，未重跑 Rust 全量、部署或访问外部 URL。完整 Master Plan 及当前安装版本仍未验收。

本轮 Board 轨迹展示增量：**100 行**只读业务组件接管轨迹模板、目标过滤与标签，Board **3,739 → 3,571 行**。历史快照不再冒充当前执行步骤或 LIVE，任务累计统计与筛选记录范围分开说明；无事件不推断停止策略。新增 **73 行 / 6 项**业务回归复用原入口及夹具；专项 **110/110**、完整 UI **692/692**、类型检查/生产构建和空白检查通过。未新增依赖、执行能力或轮询；未做真实桌面和安装态验收，巨型 Board 仍需继续整理。详见 UI 模块边界审计顶部。

本轮轨迹展示隐私增量：新增 **99 行**业务内只读投影与 **67 行**业务测试；历史端到端回归复用原导入夹具，原文件共 **180 行**。先复现私有推理正文泄露，再验证明确标记的事件、嵌套字段/类型块、JSON 编码字符串与截断内容过滤；公开回答、事件计数及原始历史记录保留。新增专项 **4/4**、相关轨迹回归 **13/13**，均 exit 0；全目标/全特性严格 Clippy、Cargo 格式检查、新增 include 文件独立 rustfmt 检查及 diff 空白检查通过。测试和 Clippy 均离线、单构建作业，测试单线程；日志 `/tmp/oviraptor-trace-privacy-{red,green,related,clippy}.log`。未修改执行逻辑、门禁或历史源数据；不是自由文本语义识别、全部接口或 Master Plan 隐私总验收。详细范围与保守隐藏行为见 UI 模块边界审计顶部。前序完整 Rust **1443/1443 + 30/30** 不覆盖本轮新增代码，本轮未重跑完整套件。

最新轨迹读取增量（2026-09-30，本地时间）：修复 Board 未核对回包任务 ID、两处轨迹详情缺少显示合同以及读取异常透传问题。共用合同 **42 行**、Board 读取状态 **48 行**，均在 `traces/` 业务目录；Board **3,755 → 3,739 行**。新增 8 项回归复用原入口和合法轨迹夹具；专项 **104/104**、完整 UI **686/686**、类型检查/生产构建与空白检查通过。任务/项目切换、同任务旧请求及卸载隔离保留，历史轨迹与 nullable 元数据兼容。没有改动 Rust、执行能力、门禁或部署；详见 UI 模块边界审计顶部。形状校验不等于来源真实或脱敏证明，整体 Master Plan 仍未完成。

最新发布核验增量（2026-09-30）：发布业务目录新增 **120 行**只读入口，复用既有输入快照、文件摘要及应用比对器；现有构建测试文件 **251 行**，沿用合成夹具及原测试入口，无新增依赖或生产执行能力。先红 **17 通过 / 6 失败**，实现后发布工具 **23/23**、Node 语法与空白检查通过。本轮未修改前后端生产代码，未重新执行 UI/Rust 全量。

实际三方核验返回 **exit 2**：当前输入不再匹配 9 月 29 日旧回执；旧构建包仍匹配保存摘要；安装目录与旧包 **5 处文件差异**（构建 16 文件、安装 15 文件）。原始记录 `/tmp/oviraptor-release-observation-current.json`，详情见后端剥离审计顶部。工具即使返回全匹配，也明确不代表可信来源、签名或运行验收；**§18.15 与整体 Master Plan 仍未完成**，本轮未构建、安装、启动应用或访问测试 URL。

最新结构增量：任务总览的日期/类型分组与卡片迁到 **214 行**业务组件，Board **3,943 → 3,755 行**；父页面保留筛选及全部原操作处理，子组件只发事件，不增加 IPC 或执行能力。标题明确“已加载任务”，不再暗示全库完整。五项回归复用原入口，Token 与任务卡片共同使用已有夹具中的渲染器，未删除独有测试。Board **151/151**、完整 UI **678/678**、类型检查/生产构建通过；详情见 UI 模块边界审计顶部。本轮未做卡片浏览器布局或安装态验收，整体目标未完成。

最新 Rust 全量已收取 **exit 0**：session **39098**，主库 **1443/1443**（1566.31 秒）、历史导入器 **30/30**（2.39 秒）、main target 零测试，无失败或忽略项。覆盖 CSV 行限制增量；运行期间未再修改 Rust。日志 `/tmp/oviraptor-csv-sidebar-rust-full.log`，原 Cargo/测试 PID 6664/6744 已退出；没有重启第二个 Cargo。以下“运行中”时间点均为历史观察，不再代表当前状态。

最新本地浏览器复验：现有聊天夹具实测四个增量窗口追读 400 条新增消息、300 条近期缓存、100 条当前页；旧页退出近期缓存后保留阅读快照，跨任务旧响应不覆盖新视图，草稿按任务恢复，新 attempt 清理和卸载隔离符合预期。仅模拟传输，不代表真实 IPC、数据库持久化或扫描执行验收；记录复用 `tools/chat_dom/README.md`，本轮没有新增代码文件、测试入口或依赖，未重复运行自动化门禁。2026-09-29 23:50 完整 Rust 仍在运行，整体目标未完成。

前序返回合同增量：调查摘要在发布前验证全部 12 项指标为非负安全整数，拒绝畸形/缺失回包；有效缓存不被覆盖，合法重试恢复，零值和附加元数据兼容。实现 **59 行**，现有专项测试 **183 行**，重复摘要夹具归并到已有夹具。新增 11 项回归，当时 Board **146/146**、完整 UI **673/673**、类型检查/构建通过；当时 Board 本体 **3,943 行**。当前体量与完整门禁结果以上文最新记录为准。

最新项目隔离增量：调查摘要由 **48 行**业务状态模块统一管理，切换项目清空旧摘要、同项目刷新保留缓存但隐藏，旧请求与卸载后响应不发布；首屏/实时/证据刷新三个入口共同使用。当前错误固定提示，摘要独立于其他列表发布，不代表跨面板原子快照。Board **3,943 行**，新测试 **147 行 / 13 项**，专项 **135/135**、完整 UI **662/662**、类型检查/构建通过；详见 UI 模块边界审计顶部。

最新只读界面增量：概览侧栏迁入 **57 行**业务组件，Board **3,954 行**；流程说明不再固定标记执行完成，项目机会统计读取中/失败时不展示旧值，用量与已加载调查摘要分别说明范围。五项回归复用原文件/入口，Board 专项 **122/122**、完整 UI **649/649**、类型检查/构建通过。未改变真实执行状态、策略或门禁；巨型文件、调查摘要独立快照隔离与安装态仍待验收，见 UI 模块边界审计顶部。

前序完整 Rust 门禁命令：`nice -n 15 cargo test --offline -j 1 --all-targets --all-features -- --test-threads=1`，日志 `/tmp/oviraptor-csv-sidebar-rust-full.log`，该快照 **1443/1443 + 导入器 30/30、exit 0**，早于后续历史展示与隐私修改，不是当前全量证明。单构建任务与单测试线程用于限制并发，不是 CPU 使用率硬上限，测试全绿不代表 Master Plan 全部验收通过。

最新历史导入增量：CSV 按逻辑输入行提前限制分配，坏行也计入资源预算；超限停止派发并拒绝整包，保留旧投影与来源文件，健康兄弟包继续。相关模块 **117/276/217 行**，六项回归集中在 **230 行**业务测试文件，复用原入口。导入模块 **95/95**、公共入口 **11/11**、导入工具 **30/30**、格式、严格 Clippy、退役/残留 **9/9 + 6/6** 通过；不是改动后的全量 Rust、安装态或 CPU 根因验收。具体边界见 `NEST_JSON_SNAPSHOT_IMPORT_AUDIT.md` 顶部补充。

最新列表边界增量：Board 首屏、实时刷新和历史分页已统一使用既有任务页合同，拒绝畸形/跨项目页且保留旧数据可重试。具体证据与局限见下表和 `NEST_RESULT_SYNC_ISOLATION_AUDIT.md` 顶部；本次没有扩大扫描执行能力或放宽门禁。

随后补齐聊天状态/事件监听，以及任务导航、历史页、阅读偏好八个入口的错误展示隐私边界：固定提示不回显底层原文，保留缓存失效、旧响应隔离及偏好失败后的显式重读；人工指令错误、消息正文和嵌套回执等不在本增量证明范围，详见聊天审计顶部。

最新结构增量：概览 KPI/图表迁入 110 行只读业务组件，Board **4,094 → 3,974 行**；修正摘要在统计刷新/失败时仍绘制旧数据的展示问题，状态图注明已加载任务范围。结构债与完整产品验收仍未完成，见 UI 模块边界审计顶部。

**2026-09-29 生产基线全量已收取，最新前端增量已验证，整体目标仍未完成。** 前序完成本地 macOS 构建观察记录及新包/安装态只读比对；之后修复任务详情独立读取、历史预览合同、当前草案绑定、实时刷新/历史导入解耦、首次导入生命周期，为当前与历史聊天增加共用消息展示合同；随后修复 Board 初始化监听阻塞、卸载后监听泄漏和轮询重建，补齐只读任务查询的乱序/项目隔离、畸形返回校验和固定错误提示；最新修复历史任务分页卸载/项目/首屏刷新隔离及旧分页覆盖较新状态，保持旧 JSON 格式兼容。此前 `.app` 不包含这些前端修复。最新增量没有修改 Rust 执行链、派发能力或用户任务门禁；没有新增依赖或重复测试入口，原回归保留。

| 最近检查 | 终态与范围 |
|---|---|
| 项目内 Cargo 配置静态退役检查 | 补查根目录及 `src-tauri/.cargo` 可选树，无扩展名/嵌套文件和悬空链接均有回归；当前工作树无这两处目录，不代表发现活动配置。新增两项先红后绿，联合退役专项 **15/15**、导入器 **30/30**、发布合成测试 **17/17**、严格 Clippy/fmt 和空白检查通过。既有测试文件 353 行，没有新增运行时功能、依赖或测试入口；未重跑主库全量或打包，详见后端剥离审计顶部 |
| 前序 Rust 全量快照（不是当前全量） | session **39098** 已收取 **exit 0**：主库 **1443/1443**、历史导入器 **30/30**，主库 **1566.31 秒**；包含 CSV 输入限制回归，但早于后续历史展示与消息隐私修改。日志 `/tmp/oviraptor-csv-sidebar-rust-full.log`。当前修改后 Rust 全量未重跑；不能替代当前验收或证明旧 CPU 事故完全解决 |
| 前序 Rust 全目标/全特性 | 主库 **1435/1435**、历史导入器 **30/30**，session 70643 exit 0；离线、单作业、单测试线程；此数字早于新增两项 Cargo 配置树回归 |
| Rust 格式与严格 Clippy | fmt check、全目标/全特性 `-D warnings` 均 exit 0 |
| UI 与构建 | 当前轨迹展示拆分后完整 UI **692/692**（包含全部聊天测试），轨迹/刷新/删除专项 **110/110**。类型检查/生产构建 exit 0：主 JS **309.67 kB**、Board **294.89 kB**、聊天 **69.99 kB**。没有重打 macOS 包或安装；较早聊天 DOM 复验不代表本次轨迹回包、轨迹展示或任务卡片的真实桌面验收 |
| Board 概览摘要结构与展示 | 110 行只读业务组件接管 KPI/图表，不请求接口/执行任务；Board **3,974 行**，仍需继续拆分。统计刷新/失败时不画旧分布或展示旧项目计数，保留缓存及独立的加载任务/Token 数据；任务状态图注明加载范围。新增 **67 行 / 5 项**回归复用既有入口，未删除独有回归；不证明全库统计一致、性能优化或整页错误状态，见 UI 模块边界审计顶部 |
| 聊天导航错误隐私与模块体量 | 八个错误入口固定提示；原任务/阅读/历史模块 **230/227/104 行**，聊天组件 399 行未增长。新增业务回归 **53 行 / 16 项**，复用原入口/渲染夹具，无新依赖和公共层；失败保存不重放、隔离/校验不变。Board 4,094 行结构债仍未消除，详见聊天审计顶部 |
| 聊天状态错误隐私 | 状态读取与事件监听改为固定提示，不回显路径/凭据等底层异常原文；普通错误保留快照，三类回执错误仍清空缓存并隔离迟到响应，恢复强制全量读取。状态模块 **233 行**，新增业务回归 **61 行 / 10 项**；任务选择/历史页/阅读偏好等错误入口未包含在本次证明中，详见聊天审计顶部 |
| Board 列表数据合同 | 首屏/实时合并/历史页共同复用业务内任务页校验，整页拒绝畸形、重复、超量与跨项目返回，保留原列表/游标可重试；全局混合项目、无项目及历史空字段兼容。分页模块 **92 行**，新增业务测试 **76 行 / 40 项**，复用原入口/夹具；Board 本体未增加行数。不是完整任务 schema 或跨区块事务，详见结果同步隔离审计顶部 |
| 任务中心全库搜索 | 与聊天任务列表共用 18 行导航合同，先整页校验项目、ID 唯一性、状态/时间戳类型与响应上限，再发布结果/游标；失败分页保留原游标可重试，错误不再同时显示无匹配任务。保持空旧字段、未知字符串状态及全局混合项目兼容。新增 16 项回归复用既有入口，80 行业务测试；聊天任务模块 230 行。不是全字段 schema、执行授权或跨页快照证明，详见 UI 模块边界审计顶部 |
| Token 概览结构与口径 | 只读统计模块 63 行、展示组件 85 行，归 Sentinel 业务目录；Board **4,094 行**，仍未达体量目标。删除已加载/部署筛选 Token 与项目整体确认数的相除公式，单位成本明确不可计算；显示加载/部署范围，并将“零漏洞产出”修正为“未关联漏洞记录”，不推断无漏洞或执行完整。新增四项先红后绿回归，复用已有文件、入口与夹具。全库同快照聚合仍未实现。详见 UI 模块边界审计顶部 |
| 聊天消息合同 | 展示字段共用合同 31 行；新增状态窗口合同 31 行，拒绝水位/起点越界、增量重放和单次超过 100 条，保留 300 条本地累计缓存、空/稀疏窗口及可选字段兼容。新增 20 项测试归入聊天业务子目录并复用现有入口；状态模块 227 行。不等于嵌套回执、授权或 Secret 脱敏核验。详见聊天审计顶部 |
| 实时刷新与历史导入 | 12 秒 Native DB 刷新不再遍历历史目录；首次发现与手动导入保留，不是持续监听。首次导入模块 59 行，拒绝卸载后回写/重试、隐藏时额外刷新并脱敏错误；不取消已发出的后端导入。启动生命周期模块 51 行，监听注册不阻塞 DB 加载、晚注册可释放、卸载后不重建轮询；固定注册错误提示，不新增自动重连。此前启动增量新增 11 项回归。详见 `NEST_RESULT_SYNC_ISOLATION_AUDIT.md` |
| 只读任务查询隔离 | 查询模块 50 行，按请求代次/搜索词/项目隔离结果，拒绝空白或非字符串 ID 的畸形回包；旧成功/失败不发布，当前错误固定提示，查询本身不创建或启动任务。此前新增搜索回归 14 项，未增加 debounce/后端取消。见上述审计 |
| 历史任务分页 | 分页模块 84 行，项目变化/卸载/首屏刷新使旧分页失效；旧 finally 不释放新请求的忙碌状态，当前失败可按原游标重试。较旧任务状态不覆盖当前数据，历史页同时间戳保留当前记录；不是 DB revision 隔离证明。该批新增 13 项，当时四份 Board 测试 186/362/119/168 行、夹具 119 行；当前 Board 体量见 Token 概览结构行。见上述审计顶部 |
| 人工确认界面 | 确认/取消均绑定当前快照唯一待处理草案及 revision/hash；确认另核验合同和接收方，旧卡片或矛盾状态不发 IPC。历史未绑定草案可取消不可确认。指令模块 273 行、聊天组件 399 行，两份指令测试 215/362 行；详见聊天审计顶部 |
| 任务中心结构与生命周期 | 组件原 540 行，拆分及归档修复后 354 行，统计说明与搜索校验后 **370 行**；详情模块 **196 行**、历史台账模块 **80 行**，共享历史合同 **12 行**；执行历史主用例当前 **319 行**，复用既有夹具，18 项原用例保留 |
| 历史台账展示与兼容 | 两处展示共同核验任务归属与规范化只读标记，拒绝矛盾返回，不修改旧 JSON 格式；多轮次与空页/300 条边界通过。最新 Rust `canonical_history` 定向 **2/2**、历史导入器完整 **30/30**；见后端剥离审计顶部 |
| 本地浏览器 | native runtime 回环夹具通过；生产聊天组件配模拟传输验证旧页保持、历史读取、返回最新；补验 A/B 迟到响应隔离、草稿在任务间隔离、线程/已读模拟存储重挂载恢复、卸载后旧回包丢弃。未发送草稿不跨卸载保存；该 DOM 续核先于最新确认边界修改，不是真实数据库重启、安装态或外部目标证明 |
| 安装态 | **未通过当前源码一致性验收**：已安装应用与构建目录二进制摘要不同；两者均不能仅凭 1.1.59 版本号认定包含本轮修改 |
| 前序只读发布核对 | 当时定向退役检查 **4/4 + 3/3**；新包 16 文件、已安装包 15 文件，发现 5 处差异（exit 2）；安装目录未改变。新包严格 codesign 完整性检查通过，但仅 ad-hoc 签名、未公证，不证明可信来源或正式发布通过；不代表此后增量的当前包 |
| 本地构建观察记录 | 发布工具 **17/17**；前序冷构建会话 **92640 已退出 0**，回执 `build_observed`，当时 743 个输入文件前后摘要一致，新包约 37 MB；**不覆盖之后的详情独立读取、历史预览合同、当前草案绑定、实时刷新解耦、首次导入生命周期及聊天消息合同修复**。证据和约 1.6 GB 的 target 保留在临时目录，未安装/启动；不等于可信源码绑定或运行验收，详见后端剥离审计顶部 |

完整原始结果、CPU 采样边界与结构债见 `NEST_TEST_CPU_INCIDENT_2026-09-27.md` 的 2026-09-29 小节；聊天修复证据见 `../tools/chat_dom/README.md`；安装文件身份见 `NEST_BACKEND_RETIREMENT_AUDIT_2026-09-26.md` 的安装态核对。仍未完成 §18 的完整角色/动作/恢复矩阵、真实桌面联合验收、最终包和跨平台验证。没有部署、访问外部目标或删除用户数据；静态退役检查和历史导入测试不能替代安装包级剥离证明。

### 后续实施约束：目录、体量与测试治理（2026-09-29）

- 前后端均按业务归属组织。页面/命令入口负责装配，展示、状态、持久化、合同校验分别放在所属业务模块；不继续向大型入口堆叠功能。优先沿用现有目录，不为本轮整理整体搬迁或同时保留新旧两套实现。
- 公共模块必须有明确复用者：业务内复用留在业务目录，跨业务且语义一致才上移公共层；不建立收纳任意代码的 `utils`/公共类。不能只因代码相似就合并不同权限或生命周期边界。
- 新增文件优先控制在 200–300 行内；接近 400 行时审查是否职责混杂，超过时记录原因与拆分边界。这是审查尺度而非机械切文件或压缩行数；已有超大文件不视为达标，涉及它们的后续改动应优先抽离相关职责并保留行为回归。当前 Board 3,739 行仍是结构欠账。
- 单元/集成测试保留独有失败场景和安全边界。仅清理已确认重复、无调用、临时调试或被等价覆盖的测试代码；先确认入口和覆盖，再删除。夹具归业务测试目录，真正重复才共享，不增加重复测试入口，不复制生产算法用于自证。
- 每轮交付列明新增/拆分/删除文件、主要文件行数、验证范围及残留结构债。生成物、日志、大型样本和临时沙箱不得作为源码长期累积；未经确认不删除用户资料、历史 JSON 或现有构建证据。沿用现有实施与审计文档，不重复生成同主题计划。

### 历史增量记录（以下数字不代表当前版本）

2026-09-28 初评已收回执未交付续核：首项 Mapper／第二项 Analyst 的原模型响应若已可靠持久化，且原 lease 仍有效，现可先证明完成前缀、冻结请求、回执、唯一待结预算及权限，再直接进行本地事务交付；未知响应绝不重发。真实生产顶层入口的成功、未知与八种损坏快照回归已通过。最终快照的全目标／全特性 Rust 主库 **1376/1376**、历史导入器 **30/30**，严格 Clippy、fmt、差异空白检查、完整 UI **251/251** 和生产构建均 exit 0。详见 `NEST_SOURCE_PARTIAL_TOOL_REENTRY_AUDIT_2026-09-28.md`。整个 Master Plan、授权 URL、真实桌面与安装包仍未完成。

2026-09-28 源码部分阶段重入续核：新增首轮工具完成后的原 lease 安全恢复，以及第一／第二项初评完成时的回执和账本前置审计。真实红测复现了预算损坏后旧路径错误写入，修复后拒绝分支逐表无写入；正常恢复不重复模型调用。当前版本全目标／全特性 Rust 主库 **1373／1373**、历史导入器 **30／30**，严格 Clippy、fmt、空白检查、完整 UI **251／251** 与生产构建通过。准确边界见 `NEST_SOURCE_PARTIAL_TOOL_REENTRY_AUDIT_2026-09-28.md`。仍不支持任意中断点、过期租约或未知效果的自动接管；真实桌面、授权 URL、安装包与 Master Plan §18 全量发布验收未完成。下文旧测试数字均为历史快照。

本轮提案交付完整性续核：最新人工指令全特性专项 **97／0**、严格全目标 Clippy、fmt、空白检查均通过；最新完整 Rust 单作业／单线程已正常退出，主库 **1353／0**、导入器 **30／0**，日志 `/tmp/oviraptor-proposal-integrity-full-current.log`。完整 UI **248／0**、生产构建及本地 Native 浏览器回环重新通过；聊天 **99** 项为先前专项结果。提案审计文档顶部为准确记录；下方 `4919` 编译等文字是此前快照。主 JS **867.35 kB** 告警、真实桌面／平台及安装包、其余专家动作矩阵和整个 Master Plan 仍未完成。

最新收取：队列增量全量 session `91631` 已结束，**1348／30，exit 0**；下段“仍在运行”为该次执行尚未结束时的历史快照。当前继续修复相邻的只读提案交付完整性：完成提交、本地补回执、聊天及模型上下文接入共同证明，损坏结果不静默采用、不重试模型。聊天先红后绿 **99**、完整 UI **248**、构建通过，主 JS **867.35 kB** 告警保留。新 Rust 提案/本地补回执专项正在 session `4919` 编译运行，严格 Clippy 和包含新生产代码的全量待执行；详见 `NEST_DIRECTIVE_PROPOSAL_AUDIT_2026-09-26.md` 顶部续核。不得用队列版全量证明本增量或完整目标已完成。

当前队列回执完整性修复：以 2 项真实红测复现了缺失/损坏完成回执和 `RAISE(IGNORE)` 静默漏写导致的假成功；提交后置检查、恢复重放、聊天投影现共用原确认/冻结 payload/原领取 fence/动作回执/完成事件核验。新增 3 项 Rust 测试含多种故障子场景，保留新租约恢复和终态历史只读；前端明确显示无法核验，不展示陈旧成功回执。首轮动作专项 **16**、聊天 **99**、完整 UI **248**、前端构建、fmt 和空白检查已通过，主 JS **866.76 kB** 告警保留。扩展后动作 **17** 已在本次全量中逐项通过，但完整 Rust session **`91631`** 仍在运行（`/tmp/oviraptor-queue-integrity-full.log`），尚未收取全量退出状态；新严格 Clippy 尚未执行，不能借前序结果宣称本增量全门禁通过。详见 `NEST_DIRECTIVE_QUEUE_ACTION_AUDIT_2026-09-26.md` 顶部续核。完整目标、其余专家/动作/恢复矩阵与真实桌面/平台验收仍未完成。

本轮 Strix 退出续核：补齐**活动符号守卫自身**的脚本格式、打包资源和单文件构建入口覆盖，新增 3 个先红后绿的回归。原有 REM-012 全文件字面量/哈希守卫已覆盖这些位置，不能把本修复描述成整个旧门禁可被绕过；两道守卫均保留，活动 baseline 仍全部为空。全特性守卫与 Native 默认/旧谱系边界 **18**、历史后端专项 **163**、迁移/封口 **25**、导入器完整 **30**、历史导入 UI **6**、本地浏览器集成、严格 Clippy、fmt 和空白检查均通过。以上集合存在重叠，不相加为全量；未重新执行全部 1345 个主库测试。本轮仅修改测试守卫与其审核登记，不改变生产扫描/历史导入行为。详情见本文末尾“Strix 双守卫与历史兼容续核”。完整目标仍未完成。

最新状态：工具轮次关注点的完整 Rust `4007` 已收取 **1342／30，exit 0**。前端已补事件突发合并、同视图事件读取单飞、终态错误重读，以及每页 100 条的长时间线展示；完整历史保留、旧页锚定、不自动标读。最终聊天 **99**、完整 UI **248**、构建、fmt 和空白检查通过，主 JS **866.27 kB** 告警保留。已用生产组件完成本地 Chrome 的部分 DOM/事件/任务切换验收，传输为显式模拟，不冒充 Tauri IPC 或全应用验收。详见聊天审计“事件突发与长时间线 DOM 续核”和 `tools/chat_dom/README.md`。完整目标、真实桌面/平台与其余专家/动作闭环仍未完成；源码三档与独立主机模块仍为设计。下方“新全量尚未完成”等为前序快照，不替代本段。

工具轮次关注点最新续报：源码普通建议和指定角色关注点可进入现有 assignment 的下一未冻结工具轮次；保留原轮数/预算、空快照冻结、旧草案合同与真实回执，不新增权限或专家。前序指定角色全量 `88757` 已收取 **1338／30，exit 0**，不覆盖本增量。当前增量源码引导专项 **22**、工具轮次回归 **9**、完整 UI **238**、严格 all-targets/all-features Clippy、fmt、空白检查和构建通过；主 JS **863.46 kB** 告警保留。首次测试编译遗漏及未送达消息错误测试预期已修正并留档。**当前增量尚未完成新全量运行**，真实桌面/平台验收及整体目标未完成；源码三档与独立主机模块仍为设计。详见聊天审计“源码工具轮次内的人工关注点”。下方数字与“运行中”均为各自历史快照，不替代本段。

Reviewer 阶段引导续报：前序完整 Rust session `44173` 已收取 **主库 1331／导入器 30，exit 0**。随后接入候选与覆盖 Reviewer 的独立建议快照、实际请求/响应回执和历史投影，保持已派发及 legacy 输入不变，无候选阶段不消耗建议，不增加调用/权限。新专项 **14**、完整 UI **235**、严格 Clippy/fmt/空白检查均已收取通过；首次测试编译的 SQLite 计数类型错误已修正并留档。新全量 **session `6253`** 正在运行，日志 `/tmp/oviraptor-source-review-guidance-all-targets-all-features.log`，尚不能宣称新全量或整体目标完成。详见聊天审计“候选与覆盖 Reviewer 的阶段引导增量”；该审计也修正了将 Stage 9 来源 gap 再审误写为未接入的旧描述。下方数字与“运行中”均为此前快照，不替代本段当前状态。

源码回执完整性续核：补上 `sourceGuidance` 缺失/置空时仍沿用通用“已送达”字段的问题；从冻结阶段及已保存模型响应识别缺口，读取不自动修复，不重发调用。新增旧库升级/重开与租约不变验收，生产入口损坏回执回归覆盖初评和工具阶段。最新 Rust 专项 **11**、聊天 **84**、完整 UI **233**、严格 all-targets/all-features Clippy、fmt 及空白检查通过。首轮测试曾错误尝试删除不可变归档回执，现改为验证其被拒绝，没有关闭保护。前序 `60591` 已退出，源码 **274／导入器 2** 通过，但不覆盖最新修复。新完整 Rust session **`44173`** 运行中，日志 `/tmp/oviraptor-source-guidance-integrity-all-targets-all-features.log`；尚不能宣称新全量或总体交付完成。详见聊天审计“送达回执缺失与旧库升级续核”，下段数字是上一版本结果。

源码阶段人工建议续报：路由快照 session `34604` 已收取 **主库 1320／导入器 30，exit 0**，不覆盖本增量。现已接入初评与工具阶段首轮的不可变建议快照、真实请求输入、同事务模型/事件/送达回执及归档；聊天重新核验执行证据，损坏回执不显示成功。送达 completed 仅代表偏好送达，不是建议采纳或漏洞确认。专项 Rust **10**、directive 回归 **92／1**、聊天 **83**、完整 UI **232**、构建及本机浏览器回环通过；主 JS **861.70 kB** 告警保留。此快照源码回归 `60591` 后续已收取 **274／2，exit 0**；新修改与门禁以上段为准。Reviewer 引导、源码指定角色/动作、轮内晚到消息策略、真实桌面/平台及 Master Plan 仍未完成。详见 `NEST_CHAT_ASYNC_AUDIT_2026-09-26.md` 新增源码阶段一节。以下为历史快照，不能将“尚未收取／未接入”旧记录作为当前状态。

本轮多目标路由续报：上一任务选择快照 session `96706` 已收取 **主库 1309／导入器 30，exit 0**。其后修复草案按最近更新时间猜接收方、确认漂移及未绑定历史指令首个目标收养路径；接收方解析/插入原子化，使用独立 Coordinator 线程键而非脱敏展示 URL 路由。新增路由回归 **11** 项包含于最终 directive 专项 **92／1**；最终完整 UI **228**（含聊天 **79**）、严格 Clippy、fmt 与构建已通过，主 JS **859.78 kB** 大包告警保留。当前修改后完整 Rust session **`34604`**，日志 `/tmp/oviraptor-directive-routing-all-targets-all-features.log`，尚未收取终态，不能复用旧全量结果。已确认源码正常分析路径尚未消费人工指令；独立主机、源码按需模式仍为设计，不是交付。详见 `NEST_CHAT_ASYNC_AUDIT_2026-09-26.md` 多目标人工指令路由增量。以下“96706 未收取”为前序记录。

2026-09-28 上次任务恢复续报：线程/已读快照全量 session `37381` 已收取 **主库 1302／导入器 30，exit 0**，不包含之后的任务选择改动。现新增项目/全局 DB 任务偏好、跨首屏 300 条的直接恢复、显式导航优先、串行合并与 CAS 冲突重载，恢复不自动启动/审批/标读。补紧 NULL scope SQL 约束，修复首次偏好读取失败后重读误提交旧选择的竞态。最新专项 Rust **7**、聊天 **76**、完整 UI **225**、严格 Clippy/fmt、构建、本机浏览器及空白检查通过；主 JS **858.26 kB** 拆包告警仍在。修改后完整 Rust 为 session **96706**，日志 `/tmp/oviraptor-dialog-selection-all-targets-all-features.log`，尚未收取，不宣布当前全部门禁或 Master Plan 完成。真实桌面重启/IPC/安装包、完整协作矩阵及其他既有缺口保留。详见 `NEST_CHAT_ASYNC_AUDIT_2026-09-26.md` 的项目上次任务恢复增量。以下为此前快照记录。

2026-09-28 聊天阅读状态续报：默认 v4 全量 session `62699` 已收取 **主库 1295／导入器 30，exit 0**，但其编译后新增了聊天持久化，不代表新代码全量。现已接通每个 scan/attempt 的线程选择、显式已读游标、双窗口版本冲突和真实 UI；只保存界面偏好，不保存正文/草稿/权限。专项 Rust **7**、聊天 **66**、完整 UI **215**、严格 Clippy、fmt、构建、localhost 浏览器和空白检查均通过；新 Rust 全量为 session **37381**，日志 `/tmp/oviraptor-dialog-persistence-all-targets-all-features.log`，仍需收取终态。项目上次选中任务（含旧分页）、真实桌面重启/IPC/安装包及整体目标仍未完成；主 JS 854.70 kB 的拆包警告保留。详见 `NEST_CHAT_ASYNC_AUDIT_2026-09-26.md` 的线程选择与已读游标增量。以下均为此前快照记录。

2026-09-28 默认 v4 与聊天隔离续报：默认注册 **10**、生产执行 **9**、授权恢复 **2**、claim 后权限漂移 **1** 均已收取 exit 0；修改后 fmt、严格 all-targets/all-features Clippy 通过。新 Rust 全量正在 session `62699`，日志 `/tmp/oviraptor-source-v4-default-all-targets-all-features.log`，未记通过。同时真实红测复现并修复聊天补充任务预览的跨任务锁/跨轮次回包，以及回执完整性错误清空快照后旧人工动作仍可发布 UI 结果的问题；完整聊天组件 **57** 项通过，包含 16 种人工动作恢复组合。见 `NEST_CHAT_ASYNC_AUDIT_2026-09-26.md` 的 2026-09-28 增量。当前完整 UI/构建/浏览器门禁尚未收取；线程选择与未读状态持久化、实机 IPC/安装包和整体目标仍未完成。

2026-09-28 当前验收续报：消费链快照的精确 all-targets/all-features 全量已收取 session `84418` exit 0：主库 **1293**、历史导入器 **30**，独立 fmt 检查 exit 0；该快照的严格 Clippy、本机浏览器、UI **201**、构建也已收取通过。随后新增默认 v4 和授权恢复验收：默认注册红测实际复现 `3 != 4`，本地入口已改为新任务 v4，保留旧根冻结合同；修改后专项及全量尚需收取，不能用上述旧编译结果代替。见 `NEST_SOURCE_COVERAGE_REVIEW_EXECUTION_AUDIT.md` §4。下方“默认仍为 v3”为前序快照，主机能力、源码三档与整个 Master Plan 均未完成。

2026-09-28 源码总体覆盖审查增量：v4 独立覆盖 subject、真实 child/回执/ACK、不可变裁决及统一消费投影已接入；零候选不制造候选 Reviewer，CI、Findings、概览、导出与 UI 区分审查完成/覆盖充分/门禁通过。专项 **21**、最新消费边界 **6**、UI **201** 与构建已通过；消费边界包含待交付、异常发布、损坏回执、重建/级联、真实零缺口通过路径和非 CI 合并断言。首次充分覆盖用例的一处测试字段路径错误已修正，失败日志保留。详见 `NEST_SOURCE_COVERAGE_REVIEW_EXECUTION_AUDIT.md`。**默认仍为 v3；修改后严格静态检查及全量门禁尚待收取，不宣布本节、源码三档模式或 Master Plan 整体完成。** 下方仅文档更新及旧 coverage 状态均为前序快照。

2026-09-28 主机产品设计复核（仅文档）：沿用独立「主机安全评估」及 §15 执行卡，不另建重复计划。当前代码已将 v4 覆盖裁决的统一投影接入 CI、findings、导出和 UI，修正主机设计文档 §3.1/§14.3 中较早的“仍只消费覆盖准备”描述；普通注册仍默认 v3，代码接入不等于最新工作区完整验收。主机离线 MVP、源码工具/辅助/深度三档仍是待交付产品规格。本轮未修改运行时代码、未启动编译或测试、未连接任何目标。

2026-09-28 产品设计补充（仅文档）：主机模块继续独立且后置，新增两平台首版模板、少量设备批次/单机 UI、复测数据关系、按需协作条件及 Qoder 分包验收卡，见 `NEST_HOST_ASSESSMENT_AND_AGENT_STRATEGY_2026-09-27.md` §15；总计划 §6.4 同步引用。只读核对确认 `source_coverage_decisions.rs` 已存在，旧文档“缺模块”记录已标记为历史；v4 消费链与默认启用仍需完整验收。**本次未实现主机能力或源码三档模式，未启动新测试，不将下方历史门禁作为当前工作区证明。**

2026-09-27 当前源码阶段合同增量：新增统一 `SourcePhaseContract`，注册、角色准入、候选 Reviewer、裁决发布和结案不再分别用请求数／schema 数字推断阶段权限。v1–v3 原始合同与预算不变，未来覆盖阶段字段在旧版本中拒绝，未支持的注册版本在发布前拒绝。新增 4 项合同单测及 3 项数据库/真实本机传输回归；源码整组 **244 passed、0 failed、0 ignored**、严格 all-targets/all-features Clippy、fmt 和空白检查通过，无残留 Cargo/Rust 测试进程，详见 `NEST_SOURCE_PHASE_CONTRACT_AUDIT_2026-09-27.md`。**这只是总体覆盖 Reviewer 的实现前置增量，没有签发新阶段或完成总体审查；本轮未重跑 Rust 全量/UI/构建/浏览器，以下 CPU 全量门禁是此修改前的基线。**

2026-09-27 当前 CPU／门禁增量：实际栈采样定位 Web 工具链文件摘要热点，改用已依赖的 Native SHA-256 实现，完整读取／防替换／限额校验及原摘要格式不变。兼容专项 7 通过；同名恢复用例本机单次 15.41→8.31 秒，不代表硬 CPU 上限。精确 `--all-targets --all-features` Rust 主库 **1268／导入器 30**、严格 Clippy/fmt、UI **196**、构建、Native 本机浏览器回环和空白检查通过，无残留 Cargo／Rust 测试进程。详见 `NEST_TOOLCHAIN_DIGEST_CPU_AUDIT_2026-09-27.md`。**这恢复本轮通用门禁通过，不代表整体项目交付；总体覆盖 Reviewer、完整恢复／未知效果核对、真实 WebView/IPC 与安装包等仍未完成，主 JS 分包告警保留。** 下方默认 1267 是修复前基线，不再是当前全量状态。

2026-09-27 当前覆盖准备增量：从四阶段回执、冻结范围/分析结果及候选 Reviewer 交付重建覆盖材料，工具阶段发现的缺口贯通根收口、普通源码报告、CI、正式审查页与三种导出。材料固定为 `prepared_not_reviewed`，保留 `source_coverage_review`，没有新增覆盖 Reviewer、模型调用、权限或预算版本。源码主库 **237**、UI **196**、构建、严格 Clippy/fmt、退役 **4** 通过；默认 features Rust 基线 **1267** 通过，801.08 秒，未包含 feature-gated 导入器，不能当作 §15 完整门禁。见 `NEST_SOURCE_COVERAGE_PREPARATION_AUDIT.md`。**总体覆盖独立审查、完整恢复、性能及整体 Master Plan 仍未完成；主 JS 848.11 kB 告警保留。** 下方各段是前序增量记录。

2026-09-27 当前整体发布增量：审查页新增 JSON/SARIF 单容器导出，两份报告来自同次核验，固定成员/摘要/共同内容校验后可经公共 JSON 文件入口与目录回导；候选去重、双来源摘要与原文保留，不恢复执行权。历史导入 **72**、源码 **15**、公共路径复测 **1**、原子发布 **5**、UI **193**、构建、严格 Clippy/fmt、退役 **4**、残留基线 **1** 通过，见 `NEST_SOURCE_ATOMIC_BUNDLE_AUDIT.md`。**未跑 Rust 全量/真实 WebView/跨平台包；总体 coverage、完整恢复、性能等仍未完成。主 JS 846.32 kB 告警保留。** 下方“多文件整体发布未完成”由此单容器方案补齐，不代表远端 CI 制品部署或整个 CI 已完成。

2026-09-27 当前规划补充：独立「主机安全评估」与 Web/源码/灰盒/CI 平级，实施优先级在现有功能收口之后；未来覆盖 Windows/Linux，先只读离线采集包，远程执行另阶段。按需 Agent 和源码工具/辅助/深度三档是待实现合同，不是当前自适应能力。详见 `NEST_HOST_ASSESSMENT_AND_AGENT_STRATEGY_2026-09-27.md`；§11–13 补充少量设备操作流程、共用协作底座、CI 触发与评估类型分离，以及 Qoder 工作包和停止线。**本次仅设计补充，未启用任何主机执行器，也不扩大现有 Web 权限。**

2026-09-27 当前联合导入增量：真实原生源码 JSON/SARIF 共享完整报告身份及作用域，同裁决跨格式归并、来源与冲突保留；精确字节和身份匹配的旧 SARIF membership 可事务内迁移，歧义拒绝。历史导入 **69**、真实源码发现/导出 **14**、公共 bundle **11**、严格 Clippy、退役 **4**、残留基线 **1** 通过。证据与迁移边界见 `NEST_SOURCE_JOINT_IMPORT_AUDIT.md`。**未跑 Rust 全量或 UI/build；多文件发布、总体覆盖、完整恢复、性能与 Master Plan 整体仍未完成。** 下方“跨格式联合导入未完成”为此前状态。

2026-09-27 当前生产 CI 导出增量：JSON/SARIF 在同一读取事务内从真实源码裁决、冻结材料和该尝试不可变策略重建 `review.ciGate`；保留 head/base/tree/rule-pack/analyzer、发布策略、状态/退出码/缺口。完成任务、过期租约和旧尝试可只读导出，执行入口的 current/scanning 条件不放宽。两种格式各自回导保留 CI 上下文且不恢复 Native 权威。源码 **13**、JSON 实际往返 **1**、策略 **8**、概览 **6**、UI **192**、构建、严格 Clippy/fmt、退役 **4+3** 通过，详见 `NEST_SOURCE_CI_EXPORT_AUDIT.md`。**不是完整 CI-006：跨格式联合导入归并、多文件整体发布、总体覆盖独立审查、完整恢复、性能及 Master Plan 整体验收仍未完成；本轮未跑 Rust 全量。** 下方“生产 CI 冻结输出未接入”为此前增量历史。

2026-09-27 当前源码 SARIF 增量：正式审查页增加 SARIF 导出，与 JSON 共用整套账本核验和不覆盖发布器；真实源码报告回导保留来源与严重度，不恢复 Native 权威。修复标准 kind/codeFlows 导入缺陷，保留旧扩展兼容；零结果摘要独立保存，避免按发现重复整份运行属性。正式源码 **11**、历史导入 **63**、发布器 **5**、JSON 真实往返 **1**、完整 UI **192**、构建、严格 Clippy/fmt、退役 **4+1** 通过，真实失败记录见 `NEST_SOURCE_SARIF_ROUNDTRIP_AUDIT.md`。**未跑 Rust 全量；当前是源码审查报告，不是完整生产 CI bundle。生产 CI 冻结输出、JSON/SARIF 跨格式归并、总体覆盖、完整恢复、性能和 Master Plan 整体验收仍未完成。** 以下“SARIF 待接入”等为此前增量历史。

2026-09-27 当前概览确认增量：正式源码裁决通过与详情/导出共用的完整账本审计进入当前轮次 KPI；源码确认与旧 Findings 数分开，混合统计不会错误相减。概览统一读取事务/后台工作线程，排除删除记录；Web root/Reviewer 绑定真实当前轮次。UI 区分已核验、尚无审查、无法核验、读取中和读取失败，并拒绝跨项目/过期响应，周期同步不叠加未完成审计。源码概览 **6**、Web 溯源 **1**、共用源码详情/导出 **8**、完整 UI **191**、构建、严格 Clippy/fmt、退役 **4+1** 通过；真实红测与夹具修正见 `NEST_OVERVIEW_SOURCE_REVIEW_AUDIT.md`。**未重跑 Rust 全量；大规模审计性能、其他 Findings/SARIF、总体覆盖、完整恢复、真实 WebView/打包及 Master Plan 整体验收仍未完成。** 以下“概览待接入”等表述为此前增量历史。

2026-09-27 当前 Findings 读取边界增量：四项后端红测复现并修复查看旧清单时重读源码/写回历史、只读 DB 写入、删除记录显示和 proof 索引资格不一致；详情与索引统一类型化显示条件，损坏策略不再默认 breadth。三项实际 SFC 红测修复旧行数被伪装为零，语言行数同步区分“未记录”和真实 0。授权启动仍生成清单，历史读取不会刷新。读取 **7**、启动 **11**、正式源码发现 **8**、历史导入 **11**、同步 **7**、完整 UI **186**、构建、严格 Clippy/fmt、退役 **4+1** 通过。详见 `NEST_FINDINGS_READ_BOUNDARY_AUDIT.md`。**未重跑 Rust 全量；显示条件不等于 Reviewer 确认。概览正式源码统计、其他 Findings/SARIF、总体覆盖、完整恢复及 Master Plan 整体验收仍未完成。** 以下为此前增量历史。

2026-09-27 当前历史快照 IO 增量：五项目录快照红测、三项旧导出红测复现后已修复。目录 manifest 使用一次有界读取，摘要/payload 同字节，拒绝增长超限与规范化冲突；任务/项目/正式源码报告统一 UUID、Unix 私有权限、完整写入后不覆盖发布，任务/项目集合使用同一 SQLite 读取事务。真实任务/项目文件往返保留原文且不改变 Native 权威表。导出专项 **10**、公共导入 **11**、源码发现 **8**、共用导入器 **70**、完整 UI **183**、构建、严格 Clippy/fmt 与退役守卫 **4+1** 通过；Clippy 首轮一处新增测试 clone 告警已修正，失败日志保留。见 `NEST_SNAPSHOT_IO_AUDIT.md`。**没有重跑 Rust 全量，未完成跨平台 ACL/强杀清理、其他 Findings/SARIF、总体覆盖、完整恢复与 Master Plan 整体验收。** 下文旧导出/目录竞态待办为此前增量状态，以本段及审计页的准确边界为准。

2026-09-27 当前公共 JSON 导入增量：真实红测复现并修复任务/项目 IPC 直接覆盖 Native 状态和清除删除标记的问题。旧任务包 v1、项目包 v2 与正式审查报告 v1 统一进入只读历史台账，原文按字节保留，已识别凭据密封存储，不恢复任务/确认权。任务中心新增原生文件选择导入入口；远端 Worker 共用同一历史服务，不再生成明文临时包。专项 **11**、共用导入器主库 **61**、完整 UI **183**、构建、严格 Clippy/fmt、退役字面量 **4** 与活残留基线 **1** 均通过。详见 `NEST_JSON_SNAPSHOT_IMPORT_AUDIT.md`。**未重跑 Rust 全量；真实 WebView/Worker 网络/跨平台打包未验证，旧导出安全、目录快照竞态、SARIF 往返、总体覆盖与整个 Master Plan 仍未完成。** 以下为此前增量历史。

2026-09-27 当前源码结果与导出增量：正式裁决读取已接入原源码结果页，按真实轮次展示；历史分析器、人工标记和旧门禁汇总保留在独立折叠区。新增重新核验完整集合的本轮审查 JSON 导出，文件独占创建、不授予执行权、不伪造总体覆盖。真实红测复现并修复父页面迟到详情串任务、旧请求污染新加载状态；补齐同任务重开、后台刷新与轨迹代次隔离。源码后端定向 **8**、完整前端 **177**、构建、严格 Clippy/fmt、退役字面量 **4** 与活残留 **1** 通过。**本增量未重跑整个 Rust 全量，不能引用下文旧全量作为新版本全量证明。** 证据与限制见 `NEST_SOURCE_RESULT_EXPORT_AUDIT.md`。**原项目包/其他 Findings、SARIF 往返、总体覆盖、完整恢复和整个 Master Plan 仍未完成。** 以下为此前增量历史。

2026-09-27 当前源码证据页增量：正式裁决已接入真实只读 Tauri API 和任务中心“证据”页，按任务/轮次完整核验后分页展示；未核验、旧版或证据损坏不显示确认发现，历史 JSON 仍独立只读。补齐切换轮次后的日志/通信/工具缓存失效，修复两处剩余测试服务退出生命周期。最终完整 Rust 主库 **1195／历史导入器 30**、实际 API/SFC **165 项**、构建及严格 Clippy/fmt 通过；CPU 样本中测试进程最大 100%、含编译的进程树最大约 247.1%，不是硬限额，收尾无残留测试进程。准确证据以 `NEST_SOURCE_FINDINGS_READ_AUDIT.md` 为准。**其他 Findings 消费与源码导出、总体覆盖独立审查、完整恢复/人工核对及整个 Master Plan 仍未完成。** 下文为此前增量历史，不能以旧段“未交付读 API”覆盖本次已接入范围，也不能把本次任务证据页当作全 Findings 统一完成。

2026-09-27 当前 Reviewer 边界恢复增量：真实顶层已能在四阶段完成、原租约有效时恢复 Reviewer 未创建/未派发/已收回执/暂停且已收回执/已交付未收口五种状态；未知模型结果不重发，拒绝准入不旋转 fence、不退款、不写终态。补测实际复现并修复撤权/损坏账本仍触发错误终态化的问题。**主库全量 1187 通过 / 2 项登记失败**；修正后退役守卫 4、历史兼容主库 30 / 导入器相关 3 项通过，完整导入器另跑 30 项通过；恢复及 claim 写后权限测试在全量中通过，严格 Clippy/fmt 通过。为控制测试负载没有再次全量，不声称单轮全绿。准确状态以 `NEST_SOURCE_REVIEW_REENTRY_AUDIT.md` 为准。**部分初评/工具恢复、租约过期/未知结果核对、真实进程强杀重启、统一 Findings UI/读 API/源码导出、总体覆盖与整个 Master Plan 仍未完成。** 下文均为此前增量历史。

2026-09-27 当前源码消费增量：正式裁决经不可由 JSON 构造的类型化消费者再次审计后，产生确认 Finding 后端投影；CI 与根终态/事件同事务提交并写后复核。保留 analyzer 原始计划、gate/gaps 与灰盒线索；候选审查不能删除总体覆盖缺口。源码 **192/2**、CI 分类/策略 **26** 项通过；新增后续回归与严格检查结果见 `NEST_SOURCE_DECISION_CONSUMER_AUDIT.md`。同步修复 Strix 残留守卫漏检带引号启动表达式、测试正向对照启动旧名假程序的问题，空基线未增加豁免。**统一 Findings UI/读 API/源码导出、总体覆盖审查、顶层恢复和全 Master Plan 验收仍未完成。** 以下“最新”均为此前增量的历史状态。

2026-09-27 最新共用证明增量：Reviewer 准入/交付与命令层收口统一到只读四阶段审计，冻结 slice、真实回执、ACK、精确用量、任务绑定及撤权不再因入口不同而漏检。历史只读材料与活动执行分开，扫描结束/旧 attempt/原仓库移动仍可核验保留证据，但不能重启或串轮次消费。两个新增测试含 30 种阶段损坏、冻结文件丢失与真实公共调度拒绝；修复前两项红测均已复现。最终 `source_` 回归 **189/2**、严格 Clippy/fmt/空白检查通过；98 次测试进程 CPU 采样最大值 99.9%、线程数 2–4，不是硬上限。**Finding/CI 消费、覆盖审查、顶层恢复及整体 Master Plan 仍未完成**；未重跑全工程/UI/浏览器。见 `NEST_SOURCE_PHASE_PROOF_AUDIT.md`，下文全为此前增量历史。

前序正式源码裁决增量（v3）：新任务在真实 Reviewer 交付事务内发布不可变、逐候选、完整回执绑定的 `agent_source_review_decisions`，并在 root manifest 暴露稳定裁决 ID。旧 v1/v2 合同及预算不升级、不回填；v3 仍为 9 次模型请求上限。新增 5 项验收，Reviewer **12/12** 通过，覆盖原子回滚、损坏拒绝、无写入重放、重开迁移与删除级联；源码首轮为 186 通过/1 个测试夹具失败，修正后源码回归 **主库 187/历史导入器相关 2** 通过，严格 Clippy/fmt/空白检查通过，退出后无残留测试进程。**Finding/CI 消费、整体覆盖、初评证明统一、顶层恢复及完整目标仍未完成。** 本轮未重跑整个工程全量/UI/浏览器；本段之后均为历史增量，1172/30 全量不能充当 v3 证明，准确日志见 `NEST_SOURCE_REVIEW_EXECUTION_AUDIT.md`。

最新真实源码 Reviewer 增量：新建 v2 根已执行第五个独立 assignment/child、实际模型审查、严格逐候选合同、模型回执/预算结算和 `source_review_result` ACK；追加式真实聊天同时校验 assignment、child 与消息终态，保存响应后只重试本地交付。v1 根仍保留 8 次调用合同，新 v2 为 9 次；成功夹具实际使用 7 次调用/140 Token。心跳写后注入额外权限的红测已复现并修复，同时覆盖 lane 丢失/取消/忽略写入。源码回归 182/2、Reviewer 7 项、心跳定向、UI 155、构建、localhost 浏览器、严格 Clippy/fmt 均通过；最新全量 session 51972 已 exit 0：**主库 1172、历史导入器 30 项通过**，退出后无残留测试进程。**正式源码决策、Finding/CI 消费、整体覆盖、顶层 crash recovery 与完整目标仍未完成。** 详细证据和严格后续顺序见 `NEST_SOURCE_REVIEW_EXECUTION_AUDIT.md`。以下各“最新”段落均为先前增量历史，不能覆盖本段或充当本增量全量结果。

最新源码审查材料增量：生产根收口已绑定真实 analyzer 来源、图候选身份/作者/修订和工具回执，终态写后复核防止材料被篡改；仍显式未独立审查。后续低负载集成发现候选自然键算法不一致，已提取共享 v1 算法修复并新增独立旧身份兼容测试，未改历史 JSON/图 ID。session 4250 材料 4 项、实际工程 HTTP 夹具 5 项、session 96167 源码主库 172／导入器 2 项、session 19381 严格 Clippy 与 fmt/空白检查通过。**完整 Rust session 41293 已 exit 0：主库 1162、导入器 30 项通过；不覆盖运行期间新增的审查输出合同。独立 Source Reviewer 与整体目标仍未完成。** 详见 `NEST_SOURCE_REVIEW_MATERIAL_AUDIT.md`。

前序源码 Reviewer 执行面修复：已用真实红测复现并修复源码 root 接受未计划通用 Reviewer/Web assignment 的漏洞，根执行面与冻结阶段版本在调度、启动、模型派发写前/写后均校验；旧行不能绕过。真实 SourceBroker 候选已验证能经 revision trigger 出现在有效证据快照。首轮全量另发现 single Web 根兼容回归，已修复并通过对应真实模型提案测试；新增四项、该版本严格 Clippy/fmt、UI 154、构建和本地浏览器均通过。**旧全量 session 46549 因测试线程泄漏与 CPU 异常已人为中断（exit 101），不是完整验收。** 资源处置见 `NEST_TEST_CPU_INCIDENT_2026-09-27.md`。前序门禁不能替代当前版本验收，执行面要求见 `NEST_SOURCE_REVIEW_SURFACE_AUDIT.md`。

最新执行历史增量：任务详情改用版本化 Web/Source 混合分页，核对真实源码回执，区分 planned/completed/refused/unverified 与 finish 控制标记；保留旧数字游标接口和历史 JSON，不补造开始时间。严格 Clippy 首轮复杂类型告警已修复；最新完整回归 session 64211 已 exit 0：主库 **1149**、历史导入器 **30**；fmt/严格 Clippy、实际界面/API **154** 项、构建、本地浏览器回环及空白检查均通过，主 JS **831.51 kB** 告警保留。准确门禁、真实失败与剩余工作统一见 `NEST_SOURCE_EXECUTION_HISTORY_AUDIT.md`。**独立 Source Reviewer、完整聊天分页、恢复与整个 Master Plan 仍未完成。**

前序源码轨迹增量：修复 Native Trace 空工具名、原子工具回执漏计调用、结果邮箱错误显示 Coordinator 的问题；绑定不一致的发送者显示 unknown，源码专家在聊天和运行卡片上使用明确标签。最终 session 75619 已 exit 0：fmt/Clippy、主库 **1148**、导入器 **30**；session 50568 已 exit 0：UI **149**、构建、本地浏览器回环及空白检查通过（主 JS 830.35 kB 告警保留）。详见 `NEST_SOURCE_TRACE_PROJECTION_AUDIT.md`。此基线不覆盖之后的 execution history 改动。

源码根收口证明：生产已改为逐角色/阶段、模型 journal、完整 transcript/工具回执、精确 mailbox ACK 和费用守恒的同事务核验；终态写入后再次核验，正常聊天控制消息不计入四份结果。真实数据工具计数不包含 finish/denial，子任务费用不重复记入 root。最终 session 58086 已 exit 0：fmt/Clippy、主库 **1147**、历史导入器 **30** 通过；session 10887 已 exit 0：UI **147**、构建及本地浏览器回环通过。见 `NEST_SOURCE_COMPLETION_PROOF_AUDIT.md`。**Source Reviewer 与整体 Master Plan 仍未完成。** 该结果不覆盖后续轨迹修复；下段多轮执行的最终基线 session 61232 为主库 1146、导入器 30。

最新源码多轮执行：生产已接入独立 `source_tools` assignment 的真实模型/工具多轮循环、逐轮不可变 journal、精确 transcript、持久 finish、mailbox 与原子预算结算。真实入口回归曾发现初评取消合同被误用，新增负面回归又发现单项工具撤权漏检，均已修复；工具拒绝的部分写入通过 SAVEPOINT 回滚，专用工具阶段 heartbeat 与负面路径 3 项定向测试已通过。生产成功路径已验证 6 次真实 localhost 模型请求、4 个已完成 assignment/ack 消息、120 tokens / 6 requests、零剩余预留。UI 147/147、构建、本地浏览器回环、最终严格 Clippy、fmt/空白检查通过；本增量全量 Rust session 61232 尚待收取终态，不能借用下段 1134/30。见 `NEST_SOURCE_ROUND_EXECUTION_AUDIT.md`。**独立 Source Reviewer、真实裁决驱动 CI、完整顶层恢复、整个 attempt 共用期限、美元费用账本及整体目标仍未完成。** 下段“仅初评”描述属于工具自动调度接线前的历史基线。

最新源码工具授权：新增独立 `source_tools` assignment 合同，真实 Native source child 不再依赖 Web plan；冻结视图、模型 runtime、逐工具 capability、fence/revision、根运行活动状态和模型/工具阶段共用期限均在执行事务中复核。Diff 不能读未选文件，撤权/漂移写入整体回滚，源码读取不消费 Web 请求预算。新增 8 项通过；首轮 Clippy 测试初始化告警已修复，session 70892 已退出 0：严格 Clippy、全量 Rust 主库 **1134/1134**／导入器 **30/30** 通过；实际界面 **147/147**、localhost 浏览器回环、前端构建、fmt/空白检查通过。主 JS **830.15 kB** 拆包警告保留。详见 `NEST_SOURCE_TOOL_AUTHORITY_AUDIT.md`。**生产仍仅自动派发无工具初评；source tools 自动多轮调度、持久 finish、独立 Reviewer/CI、整个扫描跨阶段统一时限与整体目标未完成。** 以下初评 1126/30 是历史基线，不覆盖本增量。

最新源码初评接线：生产入口已调用真实 Coordinator → RepoMapper/SourceAnalyst → 模型回执 → 预算结算/mailbox → root 收口。补上派发事务内已发布模型绑定、显式输出上限、总时限恢复、活跃租约续期；不重发 unknown 调用、不虚构退款。新增 8 项真实 localhost 回归通过，session 25504 已退出 0：严格 Clippy、全量 Rust 主库 **1126/1126**／历史导入器 **30/30** 通过；实际界面 147、浏览器回环和构建已通过。**显式美元上限仍因费用账本缺失而阻止源码模型派发；独立 Source Reviewer、CI 审查闭环和整体目标未完成。** 见 `NEST_SOURCE_INITIAL_ASSESSMENT_DISPATCH_AUDIT.md`。以下各段为先前增量的历史状态，不能覆盖本轮结论。

源码 Coordinator 注册/恢复增量：新增事务化 insert-only 入口，绑定本轮源码材料与私有密钥验证后的模型合同；恢复不覆盖终态、用量、预留或未知调用，材料变化不生成第二个 root，源码专家夹具改用真实注册入口。新增 8 项，源码相关 133 通过；完整 session 23857 已退出 0，主库 **1118／历史导入器 30**、严格 Clippy/fmt/空白检查通过，实际界面 147、localhost 浏览器与前端构建通过，主 JS 830.15 kB 拆包警告保留。**入口尚未由生产启动器调用，不能据此宣称自动编排；费用、请求输出上限、真实派发/收口和独立 Reviewer 仍须继续。** 见 `NEST_SOURCE_COORDINATOR_REGISTRATION_AUDIT.md`。下面 1110/30 是此前全量基线，不涵盖本增量。

最新多引擎来源修复：相同 bytes 按本次导入文件名/hash 保留各真实引擎；不同报告合并时由 importer 返回全部实际贡献 artifact。list/get、灰盒 claims 与源码专家输入按 logical key+revision 分组，不重复计数、不合并不同修订、不补写旧回执来源；持久 v1 JSON 和历史 envelope 不变。新增 7 项，定向 session 49429 已退出 0：结果来源 15／导入器 30／退役守卫 4、严格 Clippy 通过。完整 session 75538 已退出 0：主库 1110／历史导入器 30、严格 Clippy/fmt/空白检查通过；最新实际界面 147 与 localhost 浏览器回环通过。真实红测和验证入口纠错记录见 `NEST_SOURCE_MULTI_ENGINE_PROVENANCE_AUDIT.md`。**生产源码专家编排、独立 Reviewer 和整体目标仍未完成。**

最新源码发布合同增量：真实工作台事务冻结源码模型/预算/人工策略/技能，独立 attempt 私钥绑定公开回执和实际配置；生产源码入口在快照/分析器前验证，历史 JSON 不补权。新增 10 项，源码相关 118／路由 3／退役守卫 4／界面 147 通过；完整串行 session 86524 已退出 0，主库 1103／历史导入器 30、严格 Clippy/fmt/空白检查通过，构建和 localhost 浏览器回环通过，主 JS 830.15 kB 拆包警告保留。此结果不涵盖之后的多引擎来源修改。实现、失败历史和后续接线验收见 `NEST_SOURCE_RUNTIME_PUBLICATION_AUDIT.md`。**源码生产 Coordinator/专家自动编排、美元计费、独立 Reviewer 和整个目标仍未完成。**

最新专家传输增量：共享生产模型调用已抽为不含 Web 计划、浏览器或身份权限的最小上下文，源码真实 localhost 测试不再构造虚假 Web context。在途取消检查 child/root、fence、能力租约及执行通道；12 类局部/全局变更和真实 HTTP 等待取消均有回归，未知结果不自动重发。最终定向 9 项、退役守卫 4 项通过；完整 session 41545 已退出 0，主库 1093／历史导入器 30、严格 Clippy/fmt/空白检查通过；实际界面 147 项、构建和 localhost 浏览器回环通过。上一完整 session 6329 的 1090／1 失败与审核摘要复核证据保留在 `NEST_SPECIALIST_TRANSPORT_CANCELLATION_AUDIT.md`。该全量数字不涵盖随后进行的源码发布合同修改。**源码生产自动编排、独立 Reviewer 及完整目标仍未交付。**

最新源码专家合同增量：RepoMapper/SourceAnalyst 已接入真实 scheduler、模型调用回执、预算结算和 acknowledged mailbox，并绑定本轮源码计划/范围/视图/接受修订及独立 Source Coordinator 身份。仅为无工具初评，不授予 Web/主机/Reviewer 权限；localhost 真实 transport 验证去重和输入拒绝。生产源码 CI 已停止将 scan ID 冒充 root 或将未复核当作通过，明确保留 `source_review_not_completed`。新增 7 项与严格 Clippy 已通过；完整 session 6329 的 1090／1 失败历史及后续重跑索引见 `NEST_SOURCE_SPECIALIST_CONTRACT_AUDIT.md`。**源码启动器尚未自动编排这些角色，源码工具合同、独立 Reviewer、有效 CI 通过资格和整体 Master Plan 仍未完成。**

最新分析结果回执增量：实际导入事务返回本次接受修订，源码投影冻结 attempt/view/artifact/revision，生产 analyzer 工具与灰盒 sourceClaims 不再从 scan 级 current membership 读结果。历史补导不能替换本轮结果，缺失回执不能回退历史；失败分析器和越界位置缺口保留。最新完整 session 9125 退出 0：主库 1084／历史导入器 30、严格 Clippy/fmt/空白检查通过；新增结果回执 9 项、retirement 守卫 4 项、实际界面 147 项和 localhost 浏览器回环均通过。失败夹具的两次修正、完整日志与剩余边界见 `NEST_SOURCE_RESULT_RECEIPT_AUDIT.md`。source assignment/独立 Reviewer/CI 当前 revision 资格和整体目标仍未完成；下段的 1075／30 是此前文件工具增量的历史门禁。

最新 SourceBroker 范围增量：生产模型入口已绑定实际分析视图、精确选中文件、真实 run/root/attempt 与逐工具事务权限；候选必须有有效位置及内容摘要，触发器导致撤权或回执变化会回滚。新增 9 项回归。完整 session 93712 退出 0：主库 1075／历史导入器 30、严格 Clippy/fmt/空白检查通过，取代下段“仍待重跑”的状态。详见 `NEST_SOURCE_BROKER_SCOPE_AUDIT.md`。分析结果跨 attempt 绑定、真实 source assignment/Reviewer/CI 资格及整体目标仍未完成。

最新实际分析视图增量：完整来源快照保留，另建选中文件只读视图，分析器真实 `/src` 与逐 engine `/out` 隔离；显式 Diff 不回退整仓，Auto 明示回退，旧合同版本 0 不自动升级。不可变视图回执、来源回执、恢复 workdir 与 CI scope/fileCount/digest 校验已接通。新增 20 项定向回归、实际 SFC 147、前端构建和 localhost 回环通过；完整顺序验证 session 57719 退出 101，主库 1065／1，迁移文件单条退役审核摘要已复核更新但仍待重跑，见 `NEST_SOURCE_ANALYSIS_VIEW_AUDIT.md`。**SourceBroker 的实际范围权限、源码独立 Reviewer、CI root／当前 revision 资格与其他 Master Plan 要求仍未完成；下段“实际视图未实现”保留为上一增量当时的状态。**

最新源码请求范围合同：发布事务按 attempt 冻结源码路径／canonical root／scan type／full-diff-auto／有效 base；拒绝线程更换目录或将 CI 冒充 code，Full 忽略隐藏旧 base，历史合同缺失不能从 task JSON 自动补授权。发布后及逐分析器／取消探针／投影复核；首版源码相关 59／2、工作台 40、路由 3、残留守卫 4 已通过。文件读取加固后的最终验证 session 50125 已退出 0：新增 12 项回归、完整 Rust 1046／30、严格 Clippy/fmt 通过；前端 146／构建／localhost 流水线 session 68638 已退出 0。详见 `NEST_SOURCE_SCOPE_CONTRACT_AUDIT.md`。**实际分析 manifest／视图、CI scope 一致性和源码独立 Reviewer 仍未完成，本轮只完成请求合同层。**

最新源码增量清单修复：`base→HEAD` 改为包含工作区变化及非忽略新文件的清单，修复删除／重命名、NUL 文件名和子目录相对路径；检测隐藏索引与读取失败，不改用户索引，也不返回虚假的成功空集。禁用 Git external diff／textconv／fsmonitor，捕获后复核 HEAD／清单，保留整仓来源完整性。新增 7 项回归，快照相关 22、Native Pipeline 61、残留守卫 4 通过；顺序流水线 46228 退出 0，完整 Rust 1034／30、严格 Clippy/fmt 通过；前端流水线 87851 退出 0，实际 SFC 146／构建／localhost 回环通过，主 JS 828.82 kB 拆包警告保留。证据及失败夹具修复见 `NEST_SOURCE_DIFF_MANIFEST_AUDIT.md`。**实际 full/diff/auto 持久范围、分析输入材料化与 CI scope 一致性仍未接通，本次不能称为增量扫描完整交付。**

最新源码 CI 发布策略增量：修复工作台 `maxCritical/maxHigh/blockRelease` 未进入实际 gate、完成时读取可变全局设置的问题。发布事务按 attempt 保存不可变策略，分析前／逐分析器前后／最终投影复核；独立严重度阈值、关闭阻断后保留警告、缺失历史策略拒绝直接执行已覆盖。新增 8 项测试；最终顺序流水线 session 12044 退出 0，Stage 4 路由 3、残留守卫 4、完整 Rust 主库 1027／历史导入器 30、严格 Clippy/fmt 均通过；实际 SFC 146／构建／localhost 回环和空白检查通过，主 JS 828.82 kB 拆包警告保留。首轮完整回归 1026／1 的失败历史及三项 fixture 审核哈希更新见 `NEST_SOURCE_CI_POLICY_AUDIT.md`。源码实际 scope 与 CI freeze 一致性、source Reviewer 的 root／attempt／当前 revision 资格、完整冻结派发仍未完成，主计划保持进行中。

最新工作台重试增量：已接通锁内恢复原任务意图与发布前摘要复核，修复人工 instruction 丢失、按 skill 名恢复及空选择变成所有启用技能、损坏 JSON 回落默认值的问题；补齐真实 policy 后一并修复认证单身份掩盖空多身份列表。新增 12 项重试回归，工作台组合 40 项；最终完整 Rust 1019／30、严格 Clippy/fmt、实际 UI 146 项、构建及 localhost 浏览器全部通过，顺序流水线退出 0。首轮 REM-012 失败及未扩充白名单的修复保留，主 JS 828.82 kB 拆包警告仍在；详见 `NEST_WORKBENCH_RETRY_INTENT_AUDIT.md`。工作台完整冻结派发／在途撤权、源码 scope／人工限制与 CI freeze 范围的一致执行仍未完成，整体目标保持未完成。

最新工作台浏览器认证增量：修复历史单身份恢复遗漏策略身份列表及旧浏览器凭据直接重用；历史 JSON 仅提供身份 ID，当前数据库同一快照重查任务／项目归属、有效状态、期限及文档身份，并重建认证文件。发布事务在全部写入后再核对文件、当前材料、policy 与认证上下文，变化则整体回滚且旧 attempt 保留。新增 8 项回归，工作台定向 **28 项**；最终完整 Rust **1007／30**、严格 Clippy/fmt、实际 UI **146 项**、构建与 localhost 浏览器回环均通过，主 JS 828.82 kB 拆包警告保留。证据、首次编译失败及限制见 `NEST_WORKBENCH_AUTH_REFRESH_AUDIT.md`。提交后的派发绑定／在途撤权、原始手工认证生命周期仍未覆盖；另确认重试入口丢失人工 instruction 和读取失败默认值问题，需继续修复，整体主计划保持未完成。

最新工作台准入增量：线程创建成功但分支准入失败时，只对持锁确认仍未领取的当前源码／灰盒／CI 分支原子记录“未开始执行”；同步该分支目标和 attempt，不取消独立 sibling。重复调用、已领取、回执缺失／损坏、提交未知、暂停／更换／删除和普通 Web 不被据此改写。目标发布补齐当前 attempt 归属，仍有 sibling pending 时只更新进度而不重新触发 scanning 激活门禁。工作台定向 **20 项**、完整 Rust **999／30**、严格 Clippy/fmt、实际 UI **146 项**、构建与 localhost 浏览器回环均取得通过终态。详情仅对严格匹配的未领取回执显示“未执行”；主 JS 828.82 kB 拆包警告保留。终态和失败历史见 `NEST_WORKBENCH_ADMISSION_AUDIT.md`。完整配置冻结、未知效果恢复及整体主计划仍未完成。

上一工作台启动增量：源码／灰盒／CI 入口已将任务、身份绑定、后端计划、attempt、上下文、目标及分支派发槽位改成同一事务发布，并在提交前检查精确持久化结果、项目状态、目标熔断和重试归属。任务/认证文件改为独占 attempt 所有，已知发布失败清理本次文件，未知提交结果保留证据；显式预算不再被 full-power 抹除，统一 Web policy 保留 mode/skills/人工补充要求和 CI 字段。同步线程创建失败记录局部分支“未执行”，不伪造 claim。该增量最终全量 Rust **990／30**、严格 Clippy/fmt、前端 **143** 与构建通过，见 `NEST_WORKBENCH_STARTUP_AUDIT.md`；这是后续准入修复前的基线，不代替本次门禁。普通 Web 冻结派发合同尚未覆盖灰盒/源码、全通道恢复及整体 Master Plan 仍未完成；未部署、未访问外部目标、未启用 Host Agent。

最新人工结案与独立交接增量：已派发、已停止的 Web 任务可以人工结案，保留原执行结果、未知效果、预算/lane 与历史证据，禁止旧任务恢复/重试；随后可由用户明确确认生成唯一关联 Web 草稿，重新选择目标子集/身份/预算，不自动启动或重放。原结案、交接关系与团队聊天来自不可变回执和真实事件；游标之后的损坏事件也拒绝，前端可恢复不确定提交与重启后的规范草稿。最终完整组合 Rust 主库 979／历史导入器 30、严格 Clippy/fmt 退出码 0，实际 SFC 143 项、构建、localhost 浏览器回环及空白检查通过，主 JS 828.06 kB 拆包警告保留。首次本轮全量为主库 978/1，派发输入绑定发生未解释差异；已增加仅字段路径诊断，单测和最终全量通过，但没有把复跑成功当作偶发问题已修复。具体合同、失败日志及最终证据统一见 `NEST_ADMINISTRATIVE_CLOSURE_AUDIT.md`、`NEST_CLOSURE_HANDOFF_AUDIT.md`。此增量取代下列历史记录中的“结案/交接未实现”，不是远端效果结算、全通道控制或整体完成。未部署、未访问外部目标、未启用 Host Agent。

最新人工核对回执一致性增量：补齐首次提交、历史/幂等读取、更正及团队全量/增量时间线的真实事件校验，拒绝缺失、重复、跨任务或矛盾回执；前端比对完整返回内容，错误提交保留准确重试，聊天完整性失败后清空缓存并拒绝迟到旧响应/失去基础的增量。3 项新增后端、4 项新增实际 SFC 均先复现失败再修复；最终完整组合 Rust 主库 963／历史导入器 30、严格 Clippy/fmt 退出码 0，实际 SFC 129 项、构建及空白检查通过，主 JS 813.57 kB 拆包警告保留。首轮 961/1 派发绑定失败、最终复跑与范围限制见 `NEST_REQUEST_REVIEW_RECEIPT_INTEGRITY_AUDIT.md`。未改门禁/allowlist，未部署、外部探测或新增主机能力；未知效果行政结案、独立任务交接及其余 Master Plan 仍未完成。

最新工作空间删除增量：修复“事务外判空后并发新增记录仍被删除”的窗口，IMMEDIATE/FULL 事务内重查项目关联并验证删除结果；补计未归类的直接项目关联，保护调查/知识/归属配置和任务归属，空项目删除不清理全库资产或磁盘文件。实际 UI 显示补充数据数量，查询和提交均防止重复/换目标，拒绝时不假报成功。旧逻辑 2 项红测失败；最终完整组合 Rust 主库 960／历史导入器 30、严格 Clippy/fmt 退出码 0，实际 SFC 125 项、构建及空白检查通过，主 JS 812.62 kB 拆包警告保留。测试、实现边界和日志见 `NEST_PROJECT_DELETION_AUDIT.md`。没有部署、外部探测或新增主机执行能力；未知效果结案/独立任务交接、全通道清理及其余 Master Plan 工作仍需继续，整体目标未完成。

最新历史同步隔离增量：阶段/recon 磁盘 JSON 全部经现有 canonical importer 进入只读历史，不再创建或更新原生任务、不覆盖原生发现/检查点，也不再轮询重绑/删除目标。保留默认阶段目录、原文和历史来源，移除废弃原生投影辅助代码；部分损坏文件不阻止已提交健康证据触发刷新。最终完整组合 Rust 主库 953／历史导入器 30、严格 Clippy/fmt 通过，退出码 0；实际 SFC 120 项、构建及空白检查通过，主 JS 812.09 kB 拆包警告保留。首轮 946/7 的计数兼容失败、修复、7 项定向回归及最终证据详见 `NEST_RESULT_SYNC_ISOLATION_AUDIT.md`，未外部探测或部署。此实现取代下一段旧的“两入口共用 lifecycle 锁”同步设计，删除入口自身的可靠性合同不变；整体目标未完成。

最新任务删除增量：删除入口不再信任持久 PID 或 task_path 执行停止/文件删除；仅在任务状态、真实所有权、已登记清理与未结执行义务允许时，原子删除数据库任务及关联记录，保留产物文件与独立后续任务。实际页面明确范围并防止重复/在途换目标。两个后台结果投影入口与删除共用 lifecycle 所有权；删除标记只用于跳过，不再擅自清除矛盾任务。9 项新增后端、3 项新增实际 SFC 测试及最新完整回归证据见 `NEST_SCAN_DELETION_AUDIT.md`。这不是磁盘清理、完整未知效果结案或整体完成。

该增量最终验证：Rust 主库 949／历史导入器 30、严格 Clippy/fmt 完整组合通过；最后修正卡片“强制删除”残留文案后，实际 SFC 120 项、retirement literal 4 项与前端构建重新通过，进程退出码均为 0，空白检查通过。主 JS 812.09 kB 拆包警告保留。localhost 浏览器夹具的 8 次请求/身份隔离回归通过；未部署、未访问外部授权目标。中间版结果与一次预期失败回归的区别见上述审计，整体目标仍未完成。

最新暂停可靠性增量：暂停不再按登记 PID 直接停止并清空记录，而是先记录 pausing，等待分支、producer、独立 recon、target 的实际所有权退出及已登记清理确认，再以事务验证方式标记 paused。旧 attempt/历史目标所有权也参与新尝试准入；晚到普通更新不能覆盖暂停状态。详情及验证进度见 `NEST_PAUSE_QUIESCENCE_AUDIT.md`。该增量是人工结案的前置修复，尚不是未知效果结算、重放许可或整个 Master Plan 完成。任务删除旁路的后续修复见上段，完整结案、新任务交接及全通道清理仍需继续。

最新人工核对增量：目标详情已接入三类请求来源的真实查询与人工声明 API/UI，支持追加更正、幂等提交、来源变更失效、预算保持、真实聊天事件和跨 attempt 历史。新 checkpoint 不再遮挡旧权威 HTTP journal；旧计数无账本时仍不猜测。见 `NEST_REQUEST_OPERATOR_REVIEW_AUDIT.md`。这不是退款、请求重放、原停止任务恢复或安全结案功能；本次所有验证限定 localhost。下文“人工对账 API/UI 尚未交付”是前一增量快照，完整结案/恢复流程仍未交付。

该增量最终验证：Rust 主库 929／历史导入器 30、实际 Vue SFC 115 项通过；严格 Clippy/fmt、前端构建、本地浏览器回环与空白检查通过，相关进程退出码均已收集为 0。首轮全量的 927 通过／2 失败及夹具修正、人工残留复核记录见上述审计。主 JS 811.42 kB 拆包警告保留；未部署或访问外部 URL，不代表整体完成。

本记录对应 `docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`，用于说明当前代码已经做到什么、还没有做到什么，以及下一步真实 URL 验收的边界。

最新 HTTP 停止语义增量：已区分未知结果、授权失效、预算、存储与完整性故障；首次致命停止不会因更换 URL/invocation 或计数而清除。已领取请求失败保留原授权历史，嵌套请求失败保留部分证据链接；共享 reducer、实际 run/checkpoint/目标投影与 Reviewer/重复复核保留人工核对原因。缺失一个对照会话回归为局部证据不足，不再误判整支撤权。请求消耗界面已有未决核对引导，但人工对账 API/UI 尚未交付。见 `NEST_NATIVE_HTTP_STOP_RECONCILIATION_AUDIT.md`；本轮最终全量主库 **919／导入 30** 全通过，实际 Vue **108**、构建、严格 Clippy/fmt、本地浏览器回环和空白检查通过。完整回归日志 `/tmp/oviraptor-http-stop-authoritative-full.log` 已收齐 exit 0；主 JS **803.85 kB** 拆包警告保留，下方旧数字为历史快照。未部署、未访问外部目标，整体主计划仍未完成。

2026-09-27（本地时区）通用 HTTP journal 增量：发送前逐请求持久化占用，收到响应头后保存独立回执；预算使用不可变历史基线加新记录，不与 checkpoint 双计。未决 Executor 记录作为已记录尝试的子集显示，真实 claim/receipt 故障、并发、续跑／fresh、权限与删除清理的 9 项定向回归通过；最终全量主库 910／导入 30、实际 Vue 106 项、构建、严格 Clippy、fmt、本地浏览器回环通过。见 `NEST_NATIVE_HTTP_JOURNAL_AUDIT_2026-09-27.md`。完整人工对账、明确终止码／引导 UI 与强杀恢复验收尚未完成，不能宣称整个 Master Plan 完成。

2026-09-26 多角色目标请求记账增量：新增只读 `requestAccounting`，按明确续跑关系合并 Native 累计尝试、External Surface 与 Authorization 已记录响应及未决 claim。fresh 不继承旧预算；未知统计返回 null，不把读取变成重试。Authorization 新请求不再挤占模型预算；工具完成事件保存实际请求增量，旧事件不猜测一次请求。任务界面、根任务／独立角色快照和共享请求上限同步修正。详见 `NEST_TARGET_REQUEST_ACCOUNTING_AUDIT_2026-09-26.md`。本增量最终验证为 Rust **901／30**、实际 Vue **105** 全通过，严格 Clippy、fmt、构建、本地浏览器回环通过；主 JS **802.51 kB**。最终全量日志为 `/tmp/oviraptor-20260926-accounting-audited-full.log`。这不覆盖随后开始的通用 HTTP journal 修改；未知效果恢复、其余专家和整个 Master Plan 仍未完成。

2026-09-26 External Surface 匿名入口合同增量：真实独立 child 在冻结授权入口执行一次匿名 GET，不跟随重定向、不带 Cookie/Authorization、不调用浏览器/发现/主机工具。唯一持久化 claim 防止未知结果自动重放，received 回执、私有文件与 observed 证据核验后才交付；独立模型、真实 mailbox、WebExecutor 交接及 Reviewer 事实资格均有回归。429/WAF 保存回执后返回正确保护停止码。详见 `NEST_EXTERNAL_SURFACE_ENTRY_AUDIT_2026-09-26.md`。只交付首个合同，不代表完整公开面专家、Stage 10 或整个 Master Plan 完成；跨角色目标请求统一显示、通用恢复和其余专家仍未完成。主机边界设计另见 `NEST_HOST_BOUNDARY_DECISION_2026-09-26.md`，当前不新增可执行 Host Agent。

External Surface 增量的历史验证：Rust 主库 **894**／历史导入器 **30**、实际 Vue SFC **101** 全部通过；严格全 targets／features Clippy、fmt、前端构建与 Native localhost 浏览器回环（8 请求、身份隔离 true）通过，相关进程退出码均为 0。最终日志前缀 `/tmp/oviraptor-20260926-public-surface-`，Rust/Clippy/fmt 使用 `verified-full.log`／`verified-clippy.log`／`verified-fmt.log`；文档收口后检查 diff。主 JS **800.39 kB** 拆包警告保留。首轮 891 通过／3 失败及后续测试字段修正已在审计披露，未放宽生产校验。未部署、未访问外部授权 URL；原目标与整个 Master Plan 仍未完成。下方验证数字均为历史快照。

2026-09-26 普通 Web 未派发尝试结案增量：配置改变使旧冻结计划不可恢复时，新增“结束未派发尝试”两步确认入口，只关闭已证明从未领取派发权、没有执行进度的当前普通 Web attempt。结案回执、分支报告和任务终态同事务提交，保留原计划、文件、绑定与累计消耗；迟到同步不能重新激活，重复确认不重写回执。用户必须另行选择重试，由正常启动路径创建新 attempt 并重新检查当前配置与权限，不自动恢复或重放。已派发／效果未知、补证／跨对象授权控制及源码／CI／混合任务仍不支持。详见 `NEST_WEB_UNDISPATCHED_CLOSURE_AUDIT_2026-09-26.md`。

普通 Web 未派发结案增量的历史验证：Rust 主库 **882**／历史导入器 **30**、实际 Vue SFC **101** 全部通过；严格全 targets／features Clippy、fmt、构建与 Native localhost 浏览器回环（8 请求、身份隔离 true）通过，测试和严格检查进程退出码均为 0。日志前缀 `/tmp/oviraptor-20260926-web-closure-`，使用 `verified-full.log`／`verified-clippy.log`／`verified-fmt.log`。主 JS **800.39 kB** 拆包警告保留。首轮全量的 880 通过／2 失败及夹具修正已在审计中披露，未放宽生产校验。未部署、未访问外部授权 URL、未新增 Host Agent；不是整体完成证明。

2026-09-26 子任务完成原子性增量：修复 Scheduler 在静默少写／后续触发器破坏时误报完成的问题，核验 child 绑定、终态、撤权、lane、预算和新终态事件；WebExecutor／Authorization 的结果消息、定向确认及资源收口在单事务提交，保留真实已结算消耗。新增正常路径回归发现并修复中间版本嵌套事务错误，另覆盖两个角色的消息／状态／事件／commit 故障和非默认 revision，不增加请求或自动重放。详见 `NEST_CHILD_COMPLETION_ATOMICITY_AUDIT_2026-09-26.md`。

子任务完成原子性增量的历史验证：Rust 主库 **875**／历史导入器 **30**、实际 Vue SFC **88** 全部通过；fmt、严格全 targets／features Clippy、构建、Native localhost 浏览器回环（8 请求、身份隔离 true）与 diff 空白检查通过。主 JS **795.73 kB** 拆包警告保留。本增量日志前缀 `/tmp/oviraptor-20260926-child-completion-`，其中 `verified-full.log` 是实际全量结果（退出码 0），其余门禁对应审计第 6 节。未部署、未访问用户外部 URL、未增加 Host Agent。未知结果对账、其余专家、源码／CI／combined 启动、沙箱、UI／资产／知识生命周期及真实环境验收仍未完成；原目标保持进行中。

2026-09-26 普通 Web 人工恢复增量：新增同 attempt 两步确认入口，仅支持绑定完整、输入与授权未变化、从未领取派发权且没有执行进度的普通 Web 任务；事务提交后交接唯一 guard，不重置预算、不自动重放。补证／跨对象授权控制、源码／CI／混合及已领取或结果未知任务不支持。当时缺少的配置变化后安全结案流程，由本页顶部的限定范围增量补齐，其他不支持范围未变化。普通首次派发与恢复另增加会话任务归属和目标主机范围复核，旧诊断日志拒绝链接／特殊文件。详见 `NEST_WEB_MANUAL_RECOVERY_AUDIT_2026-09-26.md`；下方更早的「无恢复 API／UI」只代表当时快照。本增量新增 10 项 Rust／12 项实际 SFC 回归；一次中间全量因未登记的新旧后端拒绝 fixture 失败，人工审阅并登记其完整文件摘要后，最终全量通过，退役门禁未放宽。主机边界设计补充在 Master Plan §6.4，仍未实现主机审批后端或 Host Agent。

普通 Web 人工恢复增量的历史验证：Rust 主库 866／历史导入器 30、实际 Vue SFC 88 项全部通过；严格全 targets／features Clippy、fmt、前端构建、Native 本地浏览器回环（8 请求、身份隔离 true）与 diff 空白检查通过。主 JS 795.73 kB 拆包警告仍在。该增量日志为 `/tmp/oviraptor-20260926-web-recovery-*`，最终 Rust／Clippy／fmt／diff 以 `verified-full.log`／`verified-clippy.log`／`verified-fmt.log`／`verified-diff.log` 为准，前端与本地浏览器证据为 `ui.log`／`build.log`／`native.log`。包含普通 Web 私有启动绑定、工具候选入口身份、即时熔断、严格 SIGKILL 与人工恢复回归。未部署或测试外部 URL；本段及下方更早数字保留为历史快照。Master Plan 与原目标仍未全部完成。

2026-09-26 Web 启动绑定与工具入口复核后续：普通 Web 新 attempt 的启动文件、执行相关数据库输入、模型／身份／代理／策略／技能／settings／worker／应用构建及前端时限被纳入私有 HMAC 凭据，首次 claim 前后复核且不补造历史凭据。进一步绑定共用发现规则的 Node／浏览器候选入口文件（包括不存在的候选），捕获工具安装、替换、链接改指与 Node override 变化；浏览器位置 JSON 与 bundled worker 一起验证，不执行候选来做清单。领取前后即时重查本 attempt 的目标熔断，拒绝不生成 claim／失败 guard，也不误写整项任务终态。13 项原绑定、6 项工具清单及 1 项熔断新增回归纳入最终 856 项主库验收；另强化旧强杀测试必须真实 SIGKILL。上轮绑定的两次全量实际是 848 成功／1 个 macOS 非 canonical fixture 失败，已只修正测试路径并重跑通过，不放宽生产校验。详见 `NEST_WEB_DISPATCH_BINDING_AUDIT_2026-09-26.md` 与 `NEST_WEB_TOOLCHAIN_BINDING_AUDIT_2026-09-26.md`。**仍不是完整工具链封存或恢复器：共享库／浏览器 framework 与校验后替换窗口、显式同 attempt API／UI、完整授权恢复预检、未知副作用对账、源码／CI／combined 原子启动、完整桌面强杀及真实沙箱／安装包／授权环境验收仍未完成，原目标保持进行中。**

2026-09-26 分支持久派发凭据后续：Web／源码分支登记时同事务创建空派发凭据；公共领取入口先获得活锁，再以 FULL 同步级别及 IMMEDIATE 事务领取一次执行权，成功提交后才返回有清理责任的 guard。数据库忽略写入、AFTER 篡改、提交失败、旧轮次、墓碑、项目归档／Web 项目丢失和准备租约均拒绝执行；锁释放不能清除 claim，历史分支不回填为未领取。状态 API／分支卡显示未领取、已领取、历史未知，不把 claim 当成活跃／完成证明。8 项新增 Rust（含一个子进程入口）及 6 项实际 SFC 回归覆盖真实子进程强杀、故障回滚、Web startup 集成和无自动重派，详见 `NEST_BRANCH_DISPATCH_RECEIPT_AUDIT_2026-09-26.md`。**这仅补齐派发凭据与同 branch/attempt 防重；冻结配置和显式同轮次恢复、已发出调用的未知结果对账、桌面完整强杀验收及源码／CI 完整启动事务仍未完成，目标保持进行中。**

2026-09-26 普通 Web 启动原子性后续：确认／恢复／重试统一进入 scan 级活调用锁和 IMMEDIATE 事务，真实新 attempt 与后端计划绑定，目标、策略、账本、当前结果面和 pending branch 一次提交；成功提交后才派发 worker。恢复／重试不再通过分步提交和失败反向写回补偿。独占 attempt 目录中的 task JSON 不覆盖历史，提交前已知错误回滚并只清理本次已知文件，未知提交保留文件且不派发。草稿取消身份和任务同事务删除，不移除历史 task path。14 项新增测试覆盖写入 ABORT／IGNORE／AFTER、并发确认、续跑／fresh 失败原状恢复、历史文件、提交失败、熔断归档和取消竞争边界，详见 `NEST_WEB_STARTUP_ATOMICITY_AUDIT_2026-09-26.md`。该增量替代下方“普通确认启动同步事务仍未完成”的旧状态；**通用持久派发、强杀／跨重启恢复、源码／CI 完整启动事务仍未完成，不表示全部 Stage 7 或 Master Plan 完成**。

2026-09-26 资产入口草稿原子性后续：`create_sentinel_scan` 改为从项目/资产/熔断检查到策略/全部目标写入的一次 IMMEDIATE 事务。重复 ID/URL 显式稳定去重，移除目标 `INSERT OR IGNORE`；提交前核验实际任务、策略和目标内容，AFTER 删除/篡改也不能误报成功。成功只保存草稿，不创建 attempt/run、不消费身份。5 项新增测试覆盖完整结果、6 组 ABORT/IGNORE、6 组 AFTER 故障、预检错误及另连接归档。完整门禁和真实未完成项见 `NEST_ASSET_DRAFT_ATOMICITY_AUDIT_2026-09-26.md`。普通确认启动的事务/竞争/文件清理仍未完成，不能由本增量替代。

2026-09-26 补证提交持久化恢复后续：冻结输入和请求键先写本地 SQLite journal；组件重开只读恢复、明确保存才重试，已有任务只打开。双窗口 pending 唯一约束、显式结束与迟到创建互斥；永久 created scan ID 与任务/来源同事务提交，删除任务后旧请求也不能重建，来源删除不受新增外键阻止。查询故障拒绝换键创建，迟到生命周期结果不导航。新增 9 项后端、10 项实际 SFC 回归，最终主库 809／导入 30、UI 70 和其余门禁通过，详见 `NEST_GAP_SUBMISSION_RECOVERY_AUDIT_2026-09-26.md`。该增量替代下方“补证请求键仅在组件内／跨卸载重开对账未实现”的旧状态，不替代桌面进程强杀、全局 outbox、通用启动/模型恢复或全部 Stage 9。

2026-09-26 来源缺口独立再审后续：补充任务启动 specialist 前重新验证原来源，并把缺证项交给本次执行上下文；独立 Reviewer 使用新 root 的可归属 Broker HTTP 事实，分别审核原假设与本次新 finding。新增不可变 gap assessment 收据，与审核决策、mailbox 确认、child 完成和候选发布原子交付；保存响应可用于同 fencing 的无模型重试补交。历史展示重新验证回执、消息和证据，终态原任务显示逐个关联结果，不重写旧候选或复活旧权限；新 attempt 不继承旧 resolved 标记。连续补证的历史假设投影移除重复任务指导，防止递归上下文膨胀。详见 `NEST_GAP_REVIEW_CLOSURE_AUDIT_2026-09-26.md`，最终门禁以该审计为准。本增量替代下方“关联全部 gapResolved:false／来源关系尚未被运行时消费／独立补证再审收据未实现”的历史状态，但不宣称全部 Stage 9 或 Master Plan 完成。非 HTTP 证据来源校验、跨卸载/重启幂等恢复、终态/跨 fencing 对账、其余专家、沙箱与真实环境验收仍需继续。

2026-09-26 来源关联补证后续：新增不可变 root→证据目录绑定；聊天补证预览重验真实 Reviewer/gap/manifest，不能接受前端路径或任意项目/目标。保存补充草稿时原项目、精确 URL、新身份及来源/幂等键同事务落库，同键重放只返回实际任务，来源失败完整回滚。新任务必须登记明确控制组，确认前校验仍是原项目唯一目标及实际 attempt；工作台仅保存草稿，未知提交使用冻结请求重试，来源 A→B→A/卸载后迟到结果不导航。来源/后续任务关系由数据库投影，明确 `gapResolved:false`。最终主库 791／历史导入 30、UI 57、严格 Clippy/fmt、构建、本地浏览器回环及 diff 空白检查通过，完整范围与失败修复记录见 `NEST_GAP_FOLLOWUP_HANDOFF_AUDIT_2026-09-26.md`。此增量替代下方旧记录中“来源关联尚未实现”的状态，但不替代新采证回写、新 revision、独立再审及原缺口解决闭环；跨组件/重启幂等恢复、Stage 10、沙箱与真实环境验收仍未完成。

2026-09-26 Web 补证准备入口后续：工作台新增“保存草稿 / 配置控制组”，进入任务中心登记真实控制组后由用户明确启动，同时保留直接启动入口。Web URL 创建的项目/身份校验、熔断查询及任务/策略/身份/目标写入改为同一事务，检测静默少写，失败不留下半成品或占用身份；熔断查询错误不再默认放行。创建成功后即清除已消耗的草稿身份，启动响应不确定时保留原任务入口、不自动重建；切换工作空间/模式或关闭组件不会由迟到响应触发启动。5 项新增后端测试覆盖 8 组写入故障、真实控制组登记和双连接身份竞争，9 项新增工作台测试执行实际 SFC。最终主库 783、导入 30、UI 47、严格 Clippy/fmt、构建、本地回环通过；主 JS 779.24 kB 拆包警告保留。任务中心整文件哈希变化经语义复核后，仅更新退役清单中该历史显示文件的哈希和理由，未放宽门禁。详见 `NEST_WEB_DRAFT_HANDOFF_AUDIT_2026-09-26.md`。聊天补证来源到新任务的持久化关联、新采证回写和新 revision 再审仍未完成，整体 Master Plan 保持未完成。

2026-09-26 Investigator 回执恢复后续：相同有效 fencing/活跃 root 下，failed/paused child 可从已验证 received 回执补交 gap proposal/assessment，正常交付失败清理成功后最多自动本地补交一次。无需重新运行模型、恢复能力或签发目标请求；并发恢复只交付一次，已完成重放使用同一读快照，提交前核验四实体真实聊天事件。已结算失败清理增加实际状态/撤权/lane 检查，防止 SQLite 静默跳过仍假报成功。新增 6 个回归测试；最终全量主库 778／导入 30、UI 38、严格 Clippy/fmt、构建、本地浏览器回环及空白检查通过，前端 777.36 kB 拆包警告保留。具体矩阵见 `NEST_INVESTIGATOR_RECEIPT_RECOVERY_AUDIT_2026-09-26.md`。清理本身失败的 running child、终态/跨 fencing 对账、实际新合同采证与新 revision 再审及整体 Master Plan 仍未完成。

2026-09-26 Reviewer 回执恢复后续：活跃根任务、相同有效 fencing 下，failed/paused Reviewer 可从已验证 received 回执直接补齐费用及业务结果，不重调模型、不恢复执行权限；生产交付失败最多本地补交一次，同候选重入也可使用原回执。完成重放新增冻结候选字段与实际发布内容核验；正常交付和恢复均要求本事务的 assignment/child/review/mailbox 聊天事件存在，否则一起回滚。新增 4 项回归覆盖 60 组写入故障、16 组绑定/损坏拒绝及真实生产链。最终全量 Rust 主库 772、导入 30、UI 38、严格 Clippy/fmt、前端构建和本地浏览器回环通过；主 JS 777.36 kB 拆包警告保留。详见 `NEST_REVIEW_RECEIPT_RECOVERY_AUDIT_2026-09-26.md`。Investigator 恢复、终止后统一对账、真实补证与新 revision 再审，以及整体 Master Plan 仍未完成。

2026-09-26 审查交付原子性后续：Reviewer 的决策/request、mailbox 确认、child 完成及候选发布现合并为一个事务；Investigator 的提案/评估两条消息、确认和 child 完成也原子提交。候选绑定/插入/upsert/状态更新检查行数及实际发布内容，避免静默少写仍报成功；已发生的模型费用和 received 回执不随业务失败回滚。新增真实 loopback 矩阵覆盖 Reviewer 24 组、Investigator 16 组写入故障及成功重入，最终 Rust 主库 768、导入 30、UI 38、严格 Clippy/fmt、构建和本地浏览器回环通过，见 `NEST_REVIEW_DELIVERY_ATOMICITY_AUDIT_2026-09-26.md`。未修改退役允许清单，未部署或访问外部目标；终止后通用恢复、历史半完成修复、实际补证新 revision 再审及整体 Master Plan 仍未完成。

2026-09-26 外层调用所有权后续：Web/Source 分支在启动探测或构造失败守卫前取得跨进程 OS 锁并复核 pending；单目标入口在计划写入、root 复用和角色编排前取得独立锁，未取得执行权的返回与业务终态分离，不能收口原任务。结果对象持锁至生产投影完成；已结束 target 不在同一 attempt 重跑。新增 9 项线程／第二进程／真实 loopback 回归，最终全量 Rust 主库 766、导入 30，UI 38、严格 Clippy/fmt、构建与本地浏览器回环通过，见 `NEST_INVOCATION_OWNERSHIP_AUDIT_2026-09-26.md`。两条退役测试 fixture 经仅返回值适配的旧哈希还原复核后更新审查记录，没有放宽门禁。重复调用 UI 观察、历史 pending/tally 收敛、进程崩溃恢复及补证再审仍未完成，不能据此声称完整 Master Plan 已交付。

2026-09-26 只读分析恢复后续：Mapper/Identity 的创建及启动已改为单事务；活跃 root、有效且相同 fencing 下复用原 running/paused/completed 分析 assignment，不重授能力或重复预留。交付失败可从已验证 received 回执直接结算 paused child，生产链路最多本地补交一次，不重试模型；同一未开始目标执行的初始化编排也能重新进入。新增 9 项实际 loopback／故障／并发测试通过，全量 Rust 主库 757/导入 30、UI 38、严格 Clippy/fmt、构建与本地浏览器回环通过，见 `NEST_READONLY_RECOVERY_AUDIT_2026-09-26.md`。终止后通用 API/UI、整体执行并发所有权、Reviewer/Investigator 业务恢复及补证再审仍未完成，不能把本增量当成任意任务续跑。

2026-09-26 独立角色回执后续：新增 `agent_specialist_calls`，四类只读角色在 HTTP 前落库一次派发许可；received 响应/用量事件/checkpoint 原子保存，executing/uncertain 不自动重发，同请求回执可校验后本地读取且不恢复能力。Mapper/Identity 的结算、结束、结果 mailbox 及确认现已原子提交。11 项新增测试覆盖并发、故障回滚、换代、损坏回执、派发拒绝与实际编排交付失败；全量 Rust 主库 748/导入 30、UI 38、严格 Clippy/fmt、构建和浏览器回环通过。见 `NEST_SPECIALIST_RECEIPT_AUDIT_2026-09-26.md`。整条编排对 paused/completed child 的续跑、通用终止后对账、Reviewer/Investigator 后续业务恢复和补证再审仍未完成。

2026-09-26 独立角色真实传输后续：Mapper、IdentitySession、Reviewer、Investigator 改用单次模型传输，真实 loopback 证明一次预留不再触发隐式重试；调用/结算失败保留未结算预算和 lane、暂停角色并撤销能力。Reviewer 请求失败和 child 清理原子提交，提交前核验可识别 SQLite 静默跳过暂停/撤权。9 项定向测试、全量 Rust 主库 737 项及导入 30 项、38 项聊天测试、严格 Clippy/fmt、构建和本地浏览器回环通过。详见 `NEST_SPECIALIST_TRANSPORT_AUDIT_2026-09-26.md`。通用 specialist 的持久化 dispatch/response 对账、成功 mailbox 原子恢复、缺口到新合同采证再审仍未完成；四角色真实 HTTP 测试不是外部目标端到端验收。

2026-09-26 本地回执后续：已为终止任务的 `receipt_pending + received` 只读评估交付补齐 API 与聊天入口；在不重新调用模型/目标、不恢复执行权限的前提下，原子补齐消息确认、原调用成本及不可变回执。过期但未换代的历史归属可用于此纯本地操作；旧 fencing、未知响应、新 attempt 或损坏绑定仍拒绝。9 项新增后端测试、38 项聊天测试、全量 Rust 主库 728 项及导入 30 项、严格 Clippy、fmt、构建与本地浏览器回环通过，详见 `NEST_LOCAL_RECEIPT_RECONCILIATION_AUDIT_2026-09-26.md`。未知结果/费用、通用未结算 child 及旧归属的人工对账仍未交付。

2026-09-26 编排终态后续：已删除按整体任务结果批量结束人工指令的旧函数；各结束分支统一返回收口失败与原始原因。Executor/Authorization 的未结算失败暂停并保留预留，不以异常清理制造零成本完成。新增真实编排故障注入、Reviewer 假完成回归及预算保留/回滚/fencing 检查，详见 `NEST_FINALIZATION_RECEIPT_AUDIT_2026-09-26.md`。该阶段尚未交付的已存响应补齐，现由上一段的有界接口处理；不能推广为所有恢复场景已完成。

2026-09-26 后续：目标结束时的人工指令收口已接入 root 终态事务；未启动提案取消并释放未用预留，未知调用及已存响应但缺少完成回执分别保留待对账状态，聊天时间线不再冒充成功或无限等待。目标结束检查同时进入工具 Broker 与续租入口，兼容单智能体历史自根和旧库迁移。12 项新增回归与当时限制见 `NEST_DIRECTIVE_CLOSURE_AUDIT_2026-09-26.md`；此后错误传播与已存响应本地补齐的进展以上述两段为准，**未知结果人工对账及整体 Master Plan 仍未完成。**

本期边界决策：不做主机层 Agent，Web attempt 默认为 `web_only`；疑似 RCE/SSH/主机线索只记证据及未覆盖项。未来 Linux 主机验证必须独立授权、后端审批、精确目标/动作合同和新 attempt；Windows 主机不在当前范围。详见 Master Plan §6.4 与轻量沙箱合同 §2.1/§5。**这只是设计约束，尚未实现完整的主机边界分类、审批后端或主机执行器；不得将 UI 开关视为已可用功能。**

当前实现已经完成以下主干目标：

- 新任务执行面为 **Native-only**。Web、Code、Greybox、CI/CD 不会探测、安装、启动或回退到 Strix。
- 旧 Strix JSON、SQLite、SARIF、CSV、Markdown、日志和 provenance 继续通过隔离的兼容层只读导入。
- 新数据库只创建中性的 `agent_*` 知识表，不再创建旧 `strix_*` 知识表；升级数据库中已经存在的旧表保留不删，并只读复制到中性表。
- 已有 Coordinator、SPA/API Mapper、当前通用 `web_executor` 的独立 run；仅在存在真正待审漏洞候选时运行 Evidence Reviewer。API 清单、请求证据、覆盖账本和发现但不触达的主机记录保持原结果视图，不再为它们制造 CandidateFinding/Reviewer。存在有效、任务绑定且目标作用域匹配的登录身份时，会额外运行只读 IdentitySession child。登记有效控制组时还会运行独立的 Authorization TargetTouching child。Deep Investigator 已限制为只读角色；旧版以 `deep_investigator` 触达目标的在途 attempt 不会自动重派发，必须新建 attempt。Stage 10 细分专家仍未完成。
- 已接入 Coordinator lease/epoch/fencing、child run、assignment、capability lease、typed mailbox delivery/ack、预算原子预留/结算、Reviewer 独立门禁。工具 Broker 逐次检查活跃角色、assignment、lane、具体工具权限及 fencing；三条 lane 由数据库占位和跨连接竞争测试证明每目标容量为 1。
- child 启动时 assignment 与 run 状态现在同事务提交；启动失败会在当前 fencing 下终结未启动的 child 并释放 lane 与预留预算。WebExecutor 启动后如 assignment mailbox 发送/确认失败也会终结子任务，不留下可调用目标工具的孤儿能力。
- 前端 `AgentDialog` 使用数据库持久化时间线展示真实运行事件；用户自然语言先生成不可执行草案，经过安全判定和显式确认后才可能进入 Coordinator 队列。
- 协作状态已使用全局单调 `sequence`、`nest://collaboration-event` 和 `afterSequence` 增量恢复；Tauri event 只负责低延迟提示，SQLite 仍是唯一事实来源。
- Native 状态读取现在在同一个数据库快照中派生只读 `stopDiagnostic`：类别、稳定代码、阶段、下一步、分支/目标/源码缺口/清理义务和截断提示。单任务执行页与协作对话显示诊断。它不从自由文本推断 WAF/授权，也不批准自动续跑；这是 Stage B 的可见诊断部分，不等于持久化停机状态机或安全续跑完成。
- 长时间执行器在仍持有当前 fencing 的前提下续租；本地模型长生成期间也有独立心跳。工具调用沿用运行账本预分配的 invocation ID，审计与证据关联不再二次分配。
- 工具启动行与启动事件、结束行与完成事件分别原子提交，重复/缺失的完成写入拒绝。恢复时对中断且结果不明的目标或写入工具 fail closed，不能在同一 attempt 自动重放；只读证据查询可幂等恢复，恢复读取错误不再被吞掉。
- 未实现的 Stage 10 专家目前禁止签发 assignment/capability；角色枚举或配置草稿的存在不能冒充已可执行专家。
- 单账号的身份句柄必须精确匹配；模型写 `anonymous` 或未知句柄不能再被静默映射成唯一登录账号。多智能体 Broker 每次调用重新核验当前任务的身份策略、状态、有效期、凭据差异和目标作用域；取认证材料时也复核 `owner_scan_id`，会话解绑后拒绝继续发送凭证。
- 一次通用工具调用可能展开为多条 HTTP 请求；每条实际发出前再次检查子 run 权限、身份、暂停/保护状态和总目标请求上限。A/B 对照剩余预算不足两条时不会先触达单侧；对照 URL 保留 query 且拒绝跨 origin 地址。独立 Authorization child 使用单独的三侧 Broker，不借用通用执行器的额外探测。
- Reviewer 明确给出 `insufficient_evidence` 时，现在按 candidate evidence revision 有界启动一次独立、只读的 Deep Investigator run；建议与 Coordinator 的暂缓评估经持久 mailbox 投递并确认。提案不能直接授权请求，同 revision 不重复调用模型；当前还没有补证合同自动签发及证据更新后的再次协商，不能视为完整 EvidenceGap 循环。
- Reviewer 合法返回 `rejected` 时，其独立 child/assignment 以“审核执行完成”结束，而候选与 request 仍保持驳回、不得发布漏洞。Nest 显示“候选已驳回”，并用最新 revision、候选状态、Reviewer 完成态和决策消息 ack 核验审查门禁；`insufficient_evidence` 不会被误显示为已闭环。拒绝审核的局部状态不等于整条多智能体链已就绪，后者仍要满足真实 Executor 工具调用等全部证明条件。
- Finding 发布现在要求 Reviewer child/assignment **先**成功完成，再在独立 fenced 事务中核验其终态、冻结候选快照、同 request/revision 的决策及 mailbox ack；之前“先发布、后结束 Reviewer”的时序已纠正。完成状态写入失败时不发布，尝试把 child 安全终结为失败；完成已落库但发布事务失败时保持可审计的已完成 Reviewer 和未发布候选，需显式恢复，不能静默把它标成审核失败或重放目标请求。本地故障注入证明已 ack 的决策无法绕过子任务完成门禁。
- 本轮加固 mailbox：同一幂等键必须保持发送者、接收者、角色、revision 和脱敏后的 payload 完全一致；损坏的 payload JSON 阻断投递，不能静默变成 `null` 并被确认。投递作为事务执行，发送、投递、确认都受当前 Coordinator fencing 限制；旧持有者在租约切换后不能确认消息。子任务现在只确认本轮生成的预期消息，其他 Agent 的未读消息不会被顺带 ack；这仍不等于已完成通用 Coordinator 消息调度。Reviewer 候选记录 JSON 损坏也会在生成冻结快照前报错，而非带 `null` 进入评审。
- EvidenceGap 同 revision 的重入现在只有在原 child 确认完成、proposal 和 assessment 均已 ack 且候选匹配时才视为幂等成功；失败或中断会明确返回需恢复的错误，不会隐去失败，也不会为相同 revision 再次调用模型。这是故障安全加固，不是补证合同签发和重评闭环。
- Reviewer `insufficient_evidence` 后的 finding 候选现在保留在后续冻结快照中；只有同一 root run 的持久 evidence graph 新增可归属的 observed/source-derived 事实引用（带 artifact ref，排除推断、候选 finding 和 Reviewer/Deep Investigator 自述），才能打开新的 revision，由独立 Reviewer 判断是否足以确认。单改内存 `context.evidence` 或模型叙述只能重放旧决策，不会再花一次 Reviewer 调用；旧冻结快照无事实索引时也不会据此自动放行。相同冻结 revision 重入只核对已 ack 的 Reviewer 决策、子 run 终态和逐条候选的落库状态，不重复调用模型；中断/不一致要求恢复。已发布 finding 不会仅因退出待审列表而触发虚假的新 revision。Reviewer 模型输出必须满足有界、类型正确且自洽的严格 schema，错误输出不会被默认为有效决策。**事实引用只是一道重审门禁，不等于其内容已经被验证为真；系统仍未自动签发补证合同或收集新事实**。
- Authorization 三侧 GET 在响应后和最终证明前重新检查暂停/取消状态；在第一、第二、第三侧响应时暂停的本地故障测试均证明不会发布 finding，已领取的请求仍按账本计费。进程崩溃与真实目标侧的复核仍未完成。
- `confirmed` 结论一旦指定 `contractKey`，两侧已执行请求必须都属于该精确合同；相同 coverage family 不能把另一业务对象的对照证据借过来。此门禁只是防止跨合同误归因，不等于已完成对象所有权及控制组证明。
- A/B 身份对照现比较完整的 query 键值（含重复键），同名参数指向不同对象不再被当作同一请求；地址不可解析直接拒绝。`authorization` confirmed 必须绑定同一次 `compare_identities` 实际发出的同方法、同 origin、同路径、同参数且身份不同的两侧请求，差异记录的方法及精确合同也须匹配。省略或重标 coverage family 不能绕过此门禁。此加固仍不能替代业务对象归属/控制组代码证明，也不代表独立 Authorization run 已完成。
- `authorization` 覆盖率汇总现只接受已记录为 `covered` 的受控 `identity_pair`，并核对同一次 compare 调用、请求方法/origin/路径/合同与两侧不同身份。两个身份分别 replay（即使 URL 相同）或访问不同接口只算 partial；无实质差异的 A/B 仍是 `tested_no_finding`，不能因“见到两个身份”自动变成 covered。
- 2026-09-25 复核发现：原有两个 `/api/profile` 双账号角色/权限差异测试错误地把个性化响应当成 authorization confirmed。现在这两个场景只允许 `insufficient_evidence`；通用 A/B `record_hypothesis_result` 在缺乏对象归属/合法控制组时仍 fail closed。独立控制组的三侧请求可另行产生候选，但必须通过独立 Reviewer 才能发布；这不能证明通用 A/B 门禁已解除，也不能视为真实目标验收。
- 操作员侧 `save_authorization_control` 只允许在 Web 草稿执行前登记同 origin、相同路径、唯一 query 对象选择器的 owner X / tester Y、两个有效身份与 JSON 对象指针，冻结到下一次 attempt；草稿 UI 和只读预检不返回会话材料。现在独立 Authorization child 消费合同，仅逐条发出 A→X、B→X、B→Y 三次 GET；Broker 原子登记侧请求并核验能力、lane、fencing、当前任务状态/attempt、身份及预算。原始响应保留在私有 artifact，代码核对三侧状态、对象 ID、同一对象内容与错误页/缓存/重定向/截断后才暂存候选。本地 HTTP 已走通 WebExecutor → Authorization → 独立 Reviewer 的正例发布与跨账号 403 反例不发布；mailbox 故障注入验证 Authorization 子任务释放 lane/预算；第一侧响应前任务暂停时只保留一条已 claim 请求，不发跨账号/控制侧请求，也不产生候选。进程崩溃恢复与真实账号矩阵仍未端到端验收。
- 后续全量测试揭示另外两项旧验收仍期望“管理员 `/api/profile` 响应不同”产生越权 finding；现已修正为不发布 finding，并验证重新执行清空工作态后仍保留 A/B 原始证据而不会把它变成漏洞。finding 候选持久化失败时，`record_hypothesis_result` 不再先将图状态写成 `validated` 或计入内存 verdict；新增候选存储不可用的回归测试。

这不是“Master Plan 每一项均已发布验收”的声明。当前 Coordinator、Mapper、Executor 主流程与有漏洞候选时的独立 Reviewer、条件式 IdentitySession 和已登记控制组的 Authorization 代码链可用，但 Stage 9 剩余 EvidenceGap、Stage 10、跨平台安装包及用户真实授权 URL 联调仍未完成。

实施目录：`/Users/swyiic/Desktop/Rust/Oviraptor`。

基线 HEAD：`8b3300fc02a79c5677a7bdbbe47322467e9c1ad4`。工作区包含用户原有和本轮累计的未提交修改；未执行 `git reset`、`git restore`、`git clean`，未提交或推送。

## 2. Master Plan 阶段对照

| 范围 | 当前状态 | 说明 |
| --- | --- | --- |
| Stage 0–3 基线、导入、中性迁移 | 已完成当前实现 | Canonical importer、幂等/CAS、损坏拒绝、旧格式兼容和中性配置迁移已接入。 |
| Stage 4 Native Code/Greybox/CI | 已完成当前主路径 | 固定 snapshot、AnalyzerRunner、统一 SARIF 导入、CI gate、coverage gap、取消/超时/容器清理收据已接入。真实批准镜像仍需环境验收。 |
| Stage 5 活跃 Strix 删除 | 源码级已实现；安装包级未验收 | 活跃启动/安装/升级/fallback/API 已移除；历史 importer、迁移 alias、fixture、旧 provenance 和历史 UI 标签保留。前序源码专项不证明当前安装包完成剥离，安装态差异及构建观察状态以上方最新证据为准。 |
| Stage 6 控制面语义 | 已完成当前四角色主路径 | Coordinator lease/epoch/fencing、typed mailbox/ack、预算账本、capability lease、review request 已落库；Broker 入口新增逐次硬门禁，已撤销、超时和旧 fencing 的调用被拒绝。专业角色扩展仍需对应能力策略。 |
| Stage 7 Scheduler/child runs | 已完成当前 child-run 主流程，专业角色未实现 | Coordinator 调度真实 child run；数据库 lane lease 同事务占用/终态释放，assignment/run 同事务启动且启动失败回收占位，executor handoff 失败释放目标 lane/预算；升级前活动 assignment 也占位；Web 目标执行归属 `web_executor`，旧目标触达 assignment 的同 attempt 恢复拒绝重派发。未实现专家拒绝调度。 |
| Stage 8 Mapper/Reviewer | 已完成当前候选门禁 | SPA/API Mapper 为独立只读 run；漏洞 CandidateFinding 必须经过独立 Reviewer 才能发布。无候选任务不创建 Reviewer；已知非漏洞记录直接留在原证据/覆盖结果视图，未知类型仍走更严格的候选门禁。 |
| Stage 9 Identity/EvidenceGap | 部分实现 | 代码级身份模式、独立只读 IdentitySession、通用 WebExecutor A/B 硬门禁已落地。操作员登记的对象控制组可派发独立 Authorization TargetTouching child，专用 Broker 对三次 GET 逐次核验并记录；三侧响应代码证明通过后仅暂存候选，仍由 Reviewer 独立判定。本地 HTTP 完整编排正反例、三侧各自暂停及 executor/Authorization mailbox 异常释放已验证；其他逐侧网络故障、进程崩溃恢复和真实 URL/账号归属矩阵未验收。Reviewer insufficient 可触发一次只读 Deep Investigator；同 revision 的已完成重入不重复调用，新增持久事实引用才可打开下一轮重审，但完整自动补证合同签发、事实采集和重评闭环未完成。 |
| Stage 10 额外专家扩展 | 未完成 | `web_executor` 仍是 TargetTouching 通用执行专家；Deep Investigator 仅在 Reviewer 证据不足时只读启动，不能触达目标。External Surface、Parser、Client-Side、Supply Chain、Upload、Business Logic、Concurrency 仍需真正拆分为独立 run 与能力包；当前不得签发这些角色的 assignment。 |
| Stage 11 协作 UI/指令 | 已接入主要界面与有限指令链；通用协作未完成 | 聊天式时间线、角色消息、mailbox ack、两阶段确认及事件补齐已接入；人工提案目前仅单个受支持角色的只读建议，多角色分解和直接 Reviewer 请求明确 deferred。不得称任意人工消息已完成调度/执行/独立复核闭环；自定义 AgentDefinition 与复杂频道亦未完成。 |
| Stage 12 发布验收 | 未完成 | 前序 Rust/Clippy/前端及本地夹具通过的范围以上方最新表为准；真实 macOS 冷构建 exit 0，已有输入/产物观察记录和 ad-hoc 完整性检查，但安装态源码对应关系、批准容器镜像、真实桌面和 Windows/Linux/macOS 安装包矩阵均未完成。不得把局部本地门禁或模拟传输验收当作完整发布通过。 |

## 3. Strix 剥离边界

### 3.1 已从活跃执行面删除

- `StrixAgentBackend` 和 `AgentBackendKind::Strix` 活跃分支。
- Strix CLI/process/image 的探测、安装、升级、启动和 fallback。
- `check_strix_update`、`update_strix`、`test_strix_llm`、`start_strix`、`rescan_strix` 等活动命令。
- 新任务的 Strix backend policy；旧 `strix`/`docker` 配置值只作为迁移 alias 解析为 Native。
- 新任务 prompt 中的 Strix 身份描述。
- 新任务 provenance 中的 `strix-workbench` 入口。
- Native evidence 中旧 `/workspace/strix-evidence-input/...` 路径。
- 普通导出中的“Strix 状态/次数”标签。
- 新数据库中的 `strix_skills`、`strix_knowledge_entries`、`strix_learning_candidates` 建表。

当前静态门禁未发现以下活跃调用：

```text
Command::new(Strix)
resolve_strix / launch_strix / run_adaptive_strix
prepare_strix / cleanup_strix
AgentBackendKind::Strix
check_strix_update / update_strix / test_strix_llm
start_strix / rescan_strix
specialist 直接调用旧 run_native_agent 总执行器
```

### 3.2 必须保留的历史兼容

以下命名不是活动后端，不能机械删除：

- `artifact_import/adapters/legacy_strix.rs` 及历史 JSON/SARIF/CSV/Markdown 解析。
- `result_ingestion_artifacts.rs`、`result_ingestion_runs.rs`、`result_ingestion_sync.rs` 的历史结果读取。
- `strix_runs`、`strix-jobs`、`strix_run:*`、`sent_to_strix` 等旧目录、字段和 checkpoint key 的只读识别。
- 旧配置键，如 `strixLlmProfiles`、`strixProxyEnabled`、`strixFrontendPacket*`，仅用于一次性迁移或兼容读取。
- 升级数据库已经存在的旧表和旧 provenance。初始化不会删除它们，也不会继续向其写入。
- 历史 fixture、测试名称中的输入样本、Release Notes 和“历史只读”来源标签。
- LLM hook 对旧 lifecycle recovery marker 的精确只读识别；Native 不生成该 marker。

兼容原则是：**旧数据可读、可导入、可追溯，但不可恢复执行，不可成为新任务后端。**

## 4. 联合多智能体当前设计

### 4.1 角色与职责

| 角色 | 权限与职责 |
| --- | --- |
| Coordinator | 唯一计划、预算、assignment、终态和发布协调者；自身没有目标触碰工具。 |
| SPA/API Mapper | 只读分析冻结的前端/运行时证据，输出 observed/source-derived/inferred API 和参数映射。 |
| IdentitySession | 仅在当前任务绑定有效身份时启动独立只读 run，分析脱敏身份元数据和认证证据缺口；没有目标工具，不能宣称完成越权验证。 |
| Web Executor | 目前在冻结 scope、capability lease 和预算内执行有界 Web 目标验证；是过渡性的通用执行角色。 |
| Deep Investigator | Reviewer 明确返回证据不足时按候选 revision 有界启动独立只读 run，提交缺口提案和暂缓评估；当前不触达目标，也不能自行签发补证合同。 |
| Evidence Reviewer | 独立读取冻结候选与证据，不触碰目标；决定 confirmed/rejected/insufficient。 |

### 4.2 已实现的真实性约束

- 每个专家有真实 `agent_runs` child row 和 `agent_assignments`，不是前端临时生成的角色气泡。
- Coordinator lease 使用 epoch 与 fencing token；旧 Coordinator 不能继续写入。
- mailbox message 有 dedupe key、delivery 和 ack；恢复时可重放但不能重复计费或重复执行。
- capability lease 与 child run/assignment/epoch 绑定，过期或被撤销后拒绝调用。
- 工具调用时身份策略若被替换、会话过期/失效/解绑或 child 持有的身份集合与当前策略不一致，Broker fail closed；身份句柄不会因任务只有一个账号而自动别名到该账号。
- 持久化 run/assignment 若出现未知角色词汇，读取会显式报错，不能默认解释为 Coordinator。
- child budget 先原子预留，再按实际使用结算；重放幂等，冲突和超卖拒绝。
- CandidateFinding 先冻结 revision，再创建 fenced Reviewer request；只有 Reviewer confirmed 可以发布 Finding。
- 相同内容的 finding 候选重放不再将 `rejected` / `published` 重置为 `pending`；审核中的候选不可被另一份内容覆盖。发布事务核验真实 Reviewer 决策、已投递且 ack 的 mailbox 消息和冻结的候选清单；审核后新插入或改动候选时拒绝整批发布，不会顺带发布未审候选。
- Scheduler 现强制 Reviewer 独占 Review lane、只可获得 evidence/review capability；Mapper 独占 ReadOnlyAnalysis lane、不能获得目标工具。拒绝发生在 assignment 和预算落库前。
- Reviewer 失败或中断时会收口 request、assignment 和 directive，不会留下假 confirmed。
- readiness 会检查 Coordinator lease、真实 child runs、mailbox ack；有候选时严格检查独立 Reviewer gate，无候选时要求没有 Reviewer/request 且 Coordinator 已终态。

## 5. 聊天式协作界面

`src/features/sentinel/components/AgentDialog.vue` 已提供持久化多智能体时间线：

- 展示 Coordinator、Mapper、Investigator、Reviewer 和 operator 的真实落库事件。
- 展示 mailbox delivery/ack、角色状态和 Reviewer gate。
- 普通聊天只写入 `agent_directive_drafts`，不会直接产生 `pending` 指令或调用工具。
- 确认卡展示 intent、requested roles、side-effect class、预算估算、所需批准和 reason codes；只有 revision/hash/scan/attempt/root/target/fencing 全部匹配的草案才能确认。
- scope 扩大和预算增加只能进入“授权变更提案”文本，不能改变冻结 scope 或根预算；`@Agent` 只能请求评估和提案，不能直接派发、扩权或绕过 Coordinator。
- 要求绕过 Reviewer、清理合同或并发上限的消息会被拒绝；controlled write 缺少 cleanup/compensation contract 时会被拒绝。
- Coordinator claim 时会再次校验草案绑定。升级前遗留、手工插入或被篡改的 `pending` directive 会保留审计记录并转为 `rejected/unconfirmed_directive_blocked`，不会执行。
- 普通输入中的 Cookie、Token、密码和单行 `Authorization` header 会被拒绝并脱敏；单行 header 已有防原值残留的回归测试。
- 指令中每个显式 HTTP(S) URL 都分别对照当前冻结目标的 scheme、host、有效端口；混合域名、子域名、改端口、嵌套/畸形 URL 和无活动目标均进入非扩权的授权变更提案。URL userinfo 凭证先脱敏并拒绝，不会把明文密码写入草案。
- 页面刷新后从后端记录恢复，不依赖仅存在于浏览器内存的假消息；所有尚未确认的草案卡会从 SQLite 重新出现，确认或取消后才会消失。
- 草案 API 不再向前端序列化 Coordinator lease epoch 或 fencing token；这些并发控制材料只保留在后端绑定和确认校验中。
- 多个实时通知同时到达时，前端会拒绝乱序返回的旧状态响应，避免较旧时间线覆盖较新 sequence。
- `agent_collaboration_events.sequence` 覆盖 draft、directive、mailbox、run、assignment 和 review gate 的插入/状态变化；提交后由事件泵发送 `nest://collaboration-event`。
- UI 使用 `latestSequence`/`afterSequence` 合并增量。即使窗口隐藏、监听器重连或 Tauri event 丢失，也可从 SQLite 补齐，不丢消息、审批或 Agent 状态。

当前版本已经能支持“几个智能体发消息、内部通信、用户发送修正指令并由 Coordinator 引导”的条件式多角色模式；无漏洞候选时不生成 Reviewer 消息。后续扩展项是更完整的频道/未读模型、自定义 AgentDefinition 管理和 Stage 10 的额外专家，而不是恢复自然语言直达工具的路径。

## 6. 本地验证记录

2026-09-25 本次补丁后的验证状态：

| 检查 | 结果 |
| --- | --- |
| prompt 防回归测试 | 通过；local/cloud Native prompt 均包含 `Native Agent`，且不含 `strix`。 |
| `cargo test --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1` | 2026-09-25 全量串行复跑通过：lib 556 passed、importer 22 passed、main 0 failed。新增零候选任务真实入口回归；旧 mailbox、历史导入、项目隔离和有候选 Reviewer 回归仍通过。 |
| `cargo clippy --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` | 通过，0 warning。 |
| `npm run build` | 通过；仅保留 Vite 大 chunk 提示。 |
| `node tools/test_native_runtime.cjs` | `passed=true`、`observedRequests=5`、匿名和身份隔离 capture 完成。 |
| `cargo fmt --check` / `git diff --check` | 通过。 |
| Strix 活路径静态检查 | 未发现启动/安装/升级/回退路径；README 的旧默认 Strix/安装描述已修正。历史 importer、配置迁移别名和 fixture 仍保留。 |

上述自动化使用本地 fixture、mock runtime 和回环 HTTP 服务。非漏洞的 API 清单、请求证据、覆盖账本及被动发现主机仍落入原结果视图，但不消耗 Reviewer；零候选完整编排验证 3 个真实 run、0 个 Reviewer/request、`multiAgentReady=true`，UI 同步显示“无漏洞候选·免审”。对用户给出的两个入口仅做过匿名低影响 GET；**尚未对实际目标启动 Native 扫描，也未使用真实账号做联调。**

默认并行 `cargo test --all-targets --all-features` 曾出现 Code analyzer 测试的非确定性文件锁争用（`analyzer_invocation_in_progress_or_lock_unavailable`）。2026-09-25 定向并行压力测试在第 2 次复现了同一输出槽的第二次调用无法获取锁；锁守卫现于退出作用域时显式解锁，并新增活锁拒绝/释放后继承的回归测试。改动后定向并行用例连续 20 次及完整并行 lib 519、importer 22 均通过。由于之前是间歇性故障，跨机器/多次 CI 复跑仍值得观察。

## 7. 真实授权 URL 验收建议

第一次联调建议按以下顺序，避免一上来混入身份、写入和并发变量：

1. 单个明确授权 URL。
2. AnonymousOnly。
3. quick 或 standard 模式。
4. 仅启用 read-only / non-destructive capability。
5. 检查 AgentDialog 中 Coordinator、Mapper、Executor 的真实 child run、mailbox ack 和终态；若有真正待审候选，再检查独立 Reviewer 和审核门禁。没有候选不应出现虚假 Reviewer。
6. 确认没有 Strix process、网络请求、安装尝试或 `strix-jobs` 新目录。
7. 再加入单身份，然后加入 A/B 身份差异验证。
8. 最后才启用受控写入，并为上传、创建、修改等动作提供 CleanupContract。

若批准的 Analyzer 镜像、rule pack、身份材料或浏览器能力缺失，结果必须显示 coverage gap / insufficient evidence，不能显示“无漏洞”或“完整通过”。

## 8. 尚未完成的发布级工作

2026-09-25 产品重组补充：`NEST_PRODUCT_REDESIGN_AND_LEARNING_CONTRACT_2026-09-25.md` 明确了任务、对话、证据、资产画像与知识治理的统一信息架构。任务中心目前已增加待处理/已结束/全部分流、搜索、停机摘要和协作跳转；任务中心与对话页均可按更新时间＋任务编号游标继续读取更早任务（每页 300、项目隔离），对话页按已落库消息关联键筛选并跳转结果页。全局知识精炼目录只选启用 Skill（最多 24 个）与项目为空的高质量公开来源/多任务聚合知识（最多 16 条），总文本上限 42,000 字符；这不是全库容量治理。这仍只是 Stage A 的部分实现。资产类型 AI 推断、跨任务证据统计、Skill 容量/退役、自动 EvidenceGap 补证与完整频道模型仍未实现，不能标记为完成。

用户提供的两个入口在 2026-09-25 匿名低影响 GET 均返回 HTTP 200，8099 页面可见登录表单。10080 的浏览器交互在当前环境受到不安全端口策略阻止，未绕过；这不代表应用授权/业务可用。未获授权书、账号矩阵和动作白名单前不启动真实扫描。

- 用用户真实授权 URL、账号和动作白名单做端到端联调。
- 用实际批准且已本地加载的固定 digest Semgrep/CodeQL 镜像做 Native Code 验收。
- 完成 Stage 10 全部可选专家的独立角色化，而不只是当前四角色主流程。
- 完成 Stage 9 剩余部分：逐侧取消与断连故障已覆盖本地回归，仍缺进程崩溃恢复、证据落盘中断竞态和真实授权目标的 owner X、tester X、tester Y 对照验收；完整 EvidenceGap 补证合同签发、证据更新与再次协商。本地 HTTP 完整编排正反例不能替代真实目标、真实模型与账号矩阵的验收；未登记控制组的普通 A/B 差异仍须拒绝 authorization confirmed。
- 为旧版 `deep_investigator` 目标触达任务实现有证明的 checkpoint 级恢复；目前同 attempt 一律 fail-closed，须人工核对后新建 Native attempt，避免双重执行。新 `web_executor` 同 attempt 重派发同样被拒绝；单 run 中断目标调用也拒绝自动重放。
- 补齐复杂频道/未读模型和自定义 AgentDefinition；当前 `@Agent` 安全提案化与指令确认交互已经完成。
- Windows、Linux、macOS 安装包与升级迁移矩阵。
- 性能、长时间恢复、真实模型供应商和大上下文压力测试。

在这些项目完成前，可以称当前代码为“Native-only Coordinator/Mapper/Executor 主链及有候选时的独立 Reviewer 已通过本地门禁”，不应称为“Master Plan 所有扩展与发布矩阵全部完成”。

## 9. 收尾验证记录

2026-09-25 单任务详情增量：任务中心预览加入概况、执行、证据、学习四视图。attempt 仍是原任务的历史行，证据链接原结果页，学习候选与知识通过可选 `scan_id` 后端过滤；全局知识目录不传过滤条件时保持原语义。新增跨任务隔离/全局兼容单测；异步读取对任务切换采用序列保护。协作页增加只读上下文侧栏，展示本轮状态、实际出现过的角色、诊断和下一步，不声称所有角色正在运行。该 UI 不能当作资产分类、EvidenceGap 自动补证或 L0–L3 治理完成的证据。前端 build、Rust lib 559 + 历史 importer 22、Clippy、Native loopback、fmt、diff check 通过；未扫描用户提供的外部目标。

2026-09-25 执行回看增量：任务中心可按已保存的 attempt 读取该轮日志尾部（最多 200 行），展示独立目录/历史目录来源以及 `ready/not_created/empty/read_failed` 状态；切换任务或轮次会丢弃过期异步日志响应。当前 `stopDiagnostic` 只属于 Native 状态当前 attempt，不冒充旧轮次诊断。读日志只走既有脱敏、64 KiB 尾部限制的只读 API，不触发模型/目标请求或恢复任务。验证：`npm run build`、`cargo test --all-targets`（lib 559 passed）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、runner log 定向 6 测试、`node tools/test_native_runtime.cjs`（本地回环 passed）、`git diff --check` 通过，构建仍有 >500 kB chunk 警告；本轮未触达真实 URL。仍缺按 attempt 的完整 Agent 消息/工具事件和证据 revision 回看。

2026-09-25 Agent 消息回看增量：任务详情“执行”页可读取所选 attempt 的真实 `agent_messages`，每页 50 条，稳定时间＋消息 ID 向前分页（后端硬上限 100）；只从同一任务/轮次及对应 Coordinator 根链查询，未知轮次拒绝，消息文本只返回 500 字符以内的再次脱敏摘要，不返回原始 payload。协作页既有时间线也对 mailbox 摘要再次脱敏。任务/轮次切换清除旧响应。该增量尚不包括工具事件、人工指令和证据 revision 的分段历史，也不签发 EvidenceGap 补证。新增定向后端测试覆盖轮次隔离、分页、凭据脱敏及无效轮次/游标。验证：`npm run build` 通过（仅有 >500 kB chunk 提示）、`cargo test --offline --all-targets --all-features -- --test-threads=1` 重跑 lib 560 与历史 importer 22 全通过、`cargo clippy --offline --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check`、本地 Native loopback（5 次请求）及 `git diff --check` 通过。初次全量执行有一项本地授权控制组 fixture 的非阻塞 socket `WouldBlock` 偶发失败，单项复跑与随后全量复跑均通过；保留稳定性观察。真实目标没有因此被触达，也未部署安装包。

2026-09-25 工具调用回看增量：任务详情“执行”页按 attempt 从真实 `tool_invocations` 读取只读分页（前端每页 50、后端硬上限 100），用 `started_at,id` 保证同一时刻的记录也能稳定读取。只返回所属运行的 role、工具名、策略决议、状态、起止时间、错误分类（再次脱敏）和请求/响应 artifact 引用是否存在；不返回 `input_summary_json`、认证身份、原始输入、响应正文或证据文件。切换任务/轮次丢弃旧响应。已登记引用并不证明证据质量、Reviewer 审核或可重放；历史无工具记录时不伪造。定向回归测试覆盖跨轮次隔离、同时间戳分页、无效轮次和输入/错误敏感信息遮蔽。验证：`npm run build` 通过（保留大 chunk 提示）；`cargo test --offline --all-targets --all-features -- --test-threads=1` 全部通过（lib 561、历史 importer 22）；`cargo clippy --offline --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check`、`node tools/test_native_runtime.cjs`（本地回环 5 次请求）及 `git diff --check` 全通过。未触达外部授权 URL、未做安装包视觉验收。

2026-09-25 本地质量门禁稳定性：授权控制组 fixture 的监听 socket 为非阻塞，某些平台上其接受的连接仍可能非阻塞，过去的 `read().unwrap()` 偶发 `WouldBlock`。接受连接后显式设为阻塞，仍保留 2 秒读取超时；定向测试连续 15 次通过、全量 lib 561 / importer 22 重跑通过，Clippy `-D warnings`、fmt 与 diff check 通过。只修 fixture，不放宽实际授权或请求门禁。

2026-09-25 EvidenceGap 重放完整性增量：已完成的只读 Deep Investigator 协商在同一证据 revision 重入时，不再仅凭“提案/评估各有一条 ack”就当作成功。现在检查 child 终态、消息数量、根链/assignment、来去 run/角色、关联键、revision、投递及确认状态、候选、缺口、提案类型和评估决议；正文损坏或路由错乱时保留原 assignment 并返回需恢复，不再次调用模型，也不擅自访问目标。新 assignment 保存脱敏缺口快照；旧任务没有该字段仍按已确认提案比对，保留历史重放兼容。先写失败测试，证明篡改评估正文过去会被误判为成功。**这只是恢复门禁加固，不是自动补证：Coordinator 目前仍不签发可执行补证合同，也不生成新事实 revision。**

任务中心的本轮通信回看也明示“提案/评估不等于补证，新证据版本和 Reviewer 复审才算数”。本次验证：`cargo test --offline --all-targets --all-features -- --test-threads=1` 通过（lib 562、历史 importer 22）；补充旧 task slice 兼容后两项 EvidenceGap 定向回归通过，`cargo clippy --offline --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check`、`npm run build`（UI 文案修改后重新运行）、`node tools/test_native_runtime.cjs`（本地回环 5 次请求）、`git diff --check` 通过。前端仍有 >500 kB chunk 提示。这次改动涉及的 Rust/进度文档是原工作树中的未跟踪文件，`git diff --check` 不覆盖它们；另外逐文件检查了尾随空白。未部署，未触达两个外部目标，未验证真实账号矩阵。

2026-09-25 mailbox 发送边界修复：此前仅持有有效 Coordinator lease 即可写入消息，错误的发送/接收 run 或伪造角色未在写入处绑定 assignment。现在 `send` 同时检查 assignment、child、根链、scan/attempt/target、证据 revision 和双方真实角色，插入 SQL 也复核这些条件及 lease fencing；不更改旧消息结构和历史读取，幂等重放仍逐字段比对原消息，失败不新建消息或重派目标请求。失败优先回归覆盖根发给自己、外部 run 冒充 child、child 冒充 Coordinator、接收角色冒充，正确路由及幂等回放仍通过。这是消息入口的信任边界修复，不意味着 EvidenceGap 自动补证或 Stage 10 专家已实现。

本轮验证：`cargo fmt --all -- --check`、`cargo test --offline --all-targets --all-features -- --test-threads=1`（lib 563、历史 importer 22）、`cargo clippy --offline --all-targets --all-features -- -D warnings`、`npm run build`、`node tools/test_native_runtime.cjs`（本地回环 5 次请求）、`git diff --check` 均通过。未跟踪的 mailbox、测试与本文另查尾随空白，均无异常。前端仍有 >500 kB chunk 提示；未部署安装包、未触达两个外部目标、未做真实账号矩阵验收。

2026-09-25 EvidenceGap 子 run 终态完整性：恢复时的 `completed_gap_round_valid` 现在同时要求 Deep Investigator 的真实角色、只读 lane、`status='terminal'` 与 `terminal_state='completed'`。此前仅检查 terminal 状态，失败终态仍能在消息已确认时错误重放为完成；新增回归先复现原代码返回 `Ok(())`，再证明修复后返回 `gap_assessment_incomplete_requires_recovery`，不新建 assignment 或重花模型预算。全量串行 Rust lib 563、历史 importer 22、Clippy `-D warnings`、fmt、前端 build、本地 Native 回环（5 请求）、`git diff --check` 均通过。仍未实现自动补证合同、新事实 revision 和 Reviewer 复审闭环，不能因此宣称 Stage 9 完成；外部目标未触达。

2026-09-25 Web HTTP 观察入图增量：真实 Web Executor 的 Broker 请求在私有 `agent-http/*.json` 元数据和响应体成功落盘后、覆盖账本计分前，新增 `RequestRecord/observed` 图节点。节点以 child run＋元数据 slot 为自然键，绑定 root、assignment 的 evidence revision、真实 request ID、状态、响应摘要及元数据哈希；artifact 引用只含受限相对路径，不把原始 body、凭据、URL 查询或模型正文写入图。晋升时校验落盘元数据和 body 文件、视图一致性，并在数据库写事务内重新核验运行中的 Web Executor assignment、角色/lane、capability、lease/fencing 和 revision；失败不计为覆盖事实，单 Agent 路径保持原语义。回归覆盖同记录幂等、缺失/伪造元数据、错误工具、无效 revision 和被撤销租约。验证：串行 Rust lib 564、历史 importer 22、Clippy `--all-targets -D warnings`、`cargo fmt --all -- --check`、前端 build、本地 Native 回环（5 请求）和 `git diff --check` 通过；前端仍有 >500 kB chunk 提示。注意这只消除了 Web 请求无法自然成为持久新事实的一个断点；尚未实现 Deep Investigator 的可执行补证合同、目标补证执行、与旧候选的证据关系及 Reviewer 自动复审，不能将 Stage 9 或整套 Master Plan 标为完成。未部署，未触达用户给出的外部 URL。

同日入审防伪补充：原 `review_fact_refs` 只检查非空 artifact 引用，旧回归甚至用 `fixture-artifact:2` 触发第二轮 Reviewer 并发布 finding；现在 Web Executor 的 `request_record` 必须在冻结 Reviewer 事实列表时重新核对私有 HTTP 元数据文件的 SHA-256、请求编号、状态、响应摘要和 body 文件大小。不存在、被改写或仅有任意非空引用的节点不会开放第二轮审查。回归先证实旧测试的伪引用不再触发 revision 2，再证明真实 Broker 格式的记录可入审、删除文件后不再入审。跨来源证据验真和完整 EvidenceGap 合同仍是待办；这不是一个新 Reviewer 闭环的完成声明。

同日跨角色复审事实边界修复：进一步的失败优先回归发现，只读 Mapper 可以提交 `source_derived` 端点节点并附任意非空 artifact 字符串，旧逻辑即把它计为新事实，开启第二轮 Reviewer，测试甚至把原先 `insufficient_evidence` 的 finding 发布。现在用于**重新打开证据不足复审**的事实索引仅接纳可重新核验私有 HTTP 文件的 Web Broker `observed/request_record`；Mapper 等其他来源的节点仍可在证据图展示，但在实现其各自的来源、文件哈希、root/attempt 和授权验真器之前不得开启新 Reviewer。回归先在旧代码得到 `published/revision 2`，修复后保留 `insufficient_evidence/revision 1`。这不会自动补证，也不等于可以拿任意已验证 HTTP 记录认定补齐特定缺口；仍需独立 Reviewer 判断语义对应。

同日协作时间线增量读取：此前 `native_scan_status_after` 即使收到序号游标，仍逐表读取和解码任务的全部草案、指令、消息、run、assignment 与 review，最后才在内存过滤；长任务每个实时事件都重复此开销。现在六类实体查询在 SQLite 内以当前 scan/attempt、实体类型/id 和事件序号选择变更行，序号映射也只读游标之后的事件；首次无游标读取仍保留全部历史。回归覆盖同轮两个 run 仅其中一个更新、无新事件返回空增量和完整初始视图。此优化不等于完整历史分页：首次打开超长任务仍可能较重，且 Stage 9 补证合同未实现。

该增量改动的本地门禁：`cargo fmt --all -- --check`、`cargo test --offline --all-targets --all-features -- --test-threads=1`（lib 564、历史 importer 22）、`cargo clippy --offline --all-targets --all-features -- -D warnings`、`npm run build`、`node tools/test_native_runtime.cjs`（本地回环 5 次请求）及 `git diff --check` 均通过。相关未跟踪文件另查尾随空白；前端仍提示 >500 kB chunk。未部署、未触达外部 URL。

同日复审事实归属加固：失败优先回归证明真实 Broker HTTP 文件仍可能被一个篡改了 `revision` 或自然键的图节点借用，旧 `review_fact_refs` 会将其计入下一次 Reviewer 的新事实。现在冻结事实时除重新核验文件外，还要求图节点、Web Executor child、assignment 和 Coordinator 属于同一个 scan/attempt/target，assignment 的 evidence revision 与节点相同，并重算 child run＋artifact slot 的自然键与节点 ID。不符合任一条件的事实不会开启证据不足后的复审；原 Broker 写入的真实节点仍可读取。定向回归先红后绿；全量 Rust lib 564、历史 importer 22、Clippy `-D warnings`、fmt、前端 build 和 Native 本地回环（5 请求）通过。此项是复审防伪，不会授予 Deep Investigator 补证权限，Stage 9 合同、目标补证与人工授权仍待实现；未触达外部 URL。

2026-09-25 HTTP 证据字节完整性修复：Web Broker 的 `bodySha256` 原本取自响应原始字节，但私有 `.body` 文件写入的是 `String::from_utf8_lossy` 后的文本；非 UTF-8 响应的哈希与存储内容不一致。现在 `agent_write_http_record` 接受原始字节并逐字节保存；入图时和 Reviewer 冻结事实时都重新计算 `.body` 的 SHA-256，而非只对文件长度。回归先证明同长度替换响应会被旧复审路径错误接受，再覆盖篡改拒绝、原始非 UTF-8 字节留存和合法 Broker 事实正常读取。授权控制组的独立记录调用也适配字节参数，但其响应尚无 Web Broker 同等的 `request_record` 入图合同，不应借此宣称完整跨角色证据验证。验证：Rust lib 564、历史 importer 22、Clippy `-D warnings`、fmt、前端 build、Native 本地回环（5 请求）及 `git diff --check` 通过；构建仍提示 >500 kB chunk。没有部署或重新触达外部 URL。完整 EvidenceGap 补证和资产/学习治理仍待办。

2026-09-25 任务中心增量：新增 `sentinel_scans.archived_at`（旧库通过 `ensure_column` 幂等升级；任务状态、`updated_at`、尝试和结果不受归档影响），仅已结束的任务可按当前项目 ID 归档/恢复。任务中心搜索现在通过服务端按项目、视图和稳定游标查询整个任务库，目标公司/URL、Finding URL 和尝试停止原因也可检索；SQL LIKE 通配符作为普通字符处理。搜索结果与任务读取分离，异步响应有序号防旧请求覆盖，UI 显示已归档/恢复和搜索失败，数量在尚有更多页时标记 `+` 而非冒充全库精确总数。先红后绿测试覆盖跨项目、分页、目标检索、通配符、活动任务拒绝归档、恢复及旧库升级。当前仍没有更细粒度的结果深链/统一频道、完整 EvidenceGap 补证和资产/学习治理；此项不能代表 Stage A–F 全部完成。

本轮本地验证：Rust lib 566、历史 importer 22、Clippy `-D warnings`、fmt、前端 build、Native 回环 5 请求、`git diff --check` 均通过（最后增加的通配符断言另行定向通过）；仍有 >500 kB chunk 警告。沙箱/工具供应的 §5.1 已写入实施蓝图，**尚未实现**；Kali 是可选兼容环境，不是默认依赖。未部署、未再次触达用户提供的外部 URL，真实目标验收需先录入授权范围、时窗、速率和动作白名单。

2026-09-25 追加：文字/无工具回复后的停止归类先红后绿修复。此前同一分支把硬 Token、模型请求数、最大轮数统统写成 `NO_PROGRESS`；现在先识别最大轮数，再以统一预算函数判定硬/软预算，最后才累计无进展检查，硬边界不改变 stall 计数。环境页的手动依赖安装在队列/运行/暂停中任务存在时拒绝，macOS 缺 Homebrew 时不再自动下载并执行远程安装脚本；这只是入口检查，尚未解决安装与新任务启动的竞态，也没有完成签名包/只读缓存/浏览器进程隔离。新增详细的 `NEST_LIGHTWEIGHT_SANDBOX_TOOL_SUPPLY_2026-09-25.md`，明确工具按角色供应、扫描期不拉取、Kali 可选及逐阶段验收；既有源码分析器的固定镜像分支仅算局部已有能力。验证：Rust lib 568、全目标含历史 importer 22、Clippy `-D warnings`、fmt、前端 build、Native 本地回环 5 请求、`git diff --check` 均通过；前端仍提示 >500 kB chunk。未部署或触达外部目标。Master Plan 其余 EvidenceGap 闭环、停机快照、资产画像、学习治理、沙箱实施和真实目标验收均仍待做。

同日浏览器出站补丁：本地两个回环端口的夹具先红，证明页面自动加载的跨源 GET 可以在仅按 HTTP 方法判读的 CDP helper 中实际触达另一个端口。现在 `requestSafetyDecision` 在方法分类前要求请求为目标的**精确 origin**，直接跨源资源和同源 302 后跨源资源均被拦截并写入 `blockedRequests`；请求拦截与禁止下载命令未建立时，浏览器采集直接失败而非继续导航。夹具增加直接/重定向两个反例及停止审计断言，Native 回环通过（8 条观察请求）。这只收紧了浏览器 helper 的一层出站策略，不能证明 DNS 重绑定、全部子进程/Service Worker、路径级授权或内核网络隔离；仍须实现真正的逐请求 Broker 与沙箱适配器。未触达用户外部 URL。

同日静态脚本出站补丁：`static_frontend_intelligence` 不再抓取 HTML 引用的跨源脚本，将其记为 `outside_target_origin` 的 deferred 项；前端入口/同源脚本的静态 HTTP 客户端在 302 等跳转到跨源时停止跟随。独立本地双端口回归验证跨源脚本与 302 的目标端口均未收到请求。全目标 Rust 测试 570 + 历史 importer 22、Clippy 全目标/全特性 `-D warnings`、fmt、前端 build、Native 本地回环（8 条观察请求）和 `git diff --check` 均通过；Vite 仍提示 >500 kB chunk。这里仍然没有 DNS/IP pinning、内核级网络阻断、独立 worker 能力包和平台沙箱，不可宣称工具供应合同完成。未触达或部署到用户提供的外部 URL。

2026-09-25 轻量沙箱准备阶段进展：环境安装认领跨进程 OS 锁 + SQLite 持久租约，`sentinel_scans` 触发器在安装期间拒绝任务入队/激活；双向并发测试覆盖至多一方成功。显式成功完成才释放租约；失败、异常退出或崩溃保持锁定，避免子安装器仍运行时扫描启动。运行环境页新增 `installing/requires_manual_recovery` 状态、人工核对安装进程后的精确确认语、匹配 owner 的恢复命令及审计事件；运行中的 OS 锁拒绝恢复，旧库可幂等创建新表/触发器。**这只解决准备/扫描互斥，不是完整轻量沙箱**：旧 `brew`/`winget` 仍联网安装、没有固定签名能力包/只读缓存/worker 隔离/统一网络 Broker，也未验证真实授权目标。验证：Rust 574、历史导入器 22、Clippy `-D warnings`、fmt、前端 build、Native 本地双端口回环（8 个观察请求）和 `git diff --check` 通过；前端仍提示 >500 kB chunk。未部署或触达外部目标。

2026-09-25 随包 JS 供应增量：先以缺失的 `verify_bundled_worker` 回归获得编译红，再实现对浏览器/AST worker 与 Babel parser 的编译版本字节摘要校验；首次资源定位和正式执行前拒绝缺失、篡改、未注册 worker，AST 不再回退读取宿主 `@babel/parser`。环境页及远程 Worker 环境报告分别显示浏览器脚本、AST+parser 的实际校验状态，并把尚未实现的进程/网络隔离明确标为 `unsupported_sandbox`，不与脚本完整性混淆。正式运行的未注册 worker 被拒绝，已有临时浏览器测试替身仅在测试编译中开放；最终验证 Rust 577、历史导入器 22、Clippy 全目标全特性 `-D warnings`、fmt、前端 build、Native 双端口回环（8 条观察请求）、`git diff --check` 均通过，Vite 仍警告主 chunk >500 kB。此增量未固定 Node/Chrome 二进制、未消除校验/执行竞态、未引入能力包签名或进程/网络隔离；其余 Stage 9–11、资产画像、学习治理与真实授权目标验收仍待完成。没有部署或触达外部 URL。

2026-09-25 EvidenceGap 提案结构化增量：Deep Investigator 的单轮只读建议改为严格 schema，含 `gapCode`、已验证事实引用、缺失证据、前置条件、允许的合同类型、信息增益、影响上限、成本、只读副作用级别、重叠键、反证与停止条件；Coordinator 对事实引用做当前 root 的 Broker 观察再验证，拒绝未知合同、写操作和越界成本。提案及确定性暂缓原因入双向 mailbox，重入时检查结构、引用归属及 assessment 对应关系；历史 v1 消息仍可按旧格式读取。新增结构边界、篡改重入回归。验证：Rust 全目标全特性 lib 579 与 importer 22 项、Clippy 全目标全特性 `-D warnings`、fmt、前端 build、本地 Native 回环（8 请求）、`git diff --check` 和相关未跟踪文件尾随空白检查通过；Vite 仍提示主 JS chunk >500 kB。**这仍不是补证合同签发**：没有自动目标补证、事实更新或 Reviewer 复审，Stage 9 未完成。沙箱供应仍遵守 `NEST_LIGHTWEIGHT_SANDBOX_TOOL_SUPPLY_2026-09-25.md`，绝不能因 JSON 提案而授予目标访问；本轮未访问外部 URL。

2026-09-25 协作对话补证可见性：任务的持久 mailbox `gap_proposed` / `proposal_assessed` 在状态投影中只选取并脱敏有界的提案字段、成本上限及确定性暂缓原因；对话气泡明示“建议未授权/未执行”，逐项展示缺失证据、前置条件、建议合同与事实引用。旧版无结构消息继续显示摘要，不伪造结构；未知合同或伪造 `approved` 决策不产生详情。失败优先回归验证此前详情为 Null，新增投影回归还证明未知字段和密钥不外泄。此 UI 不代表已经签发补证合同、自动重审或 Stage 9 完成。

2026-09-25 沙箱网络边界增量：多智能体 WebExecutor 的 `browser_action` 在无逐请求 Broker 的独立浏览器沙箱时不再启动宿主 CDP worker；有已定位只读 URL 时可走原有单次 HTTP Broker，结果明确标 `browserDriven=false` 与 `unsupported_sandbox:browser_network_broker_unavailable`，不伪称 DOM 动作和网络增量。新增回归证明目标只收到该 Broker GET。Rust 全目标全特性 lib 581 与 importer 22 项、Clippy `-D warnings`、fmt、前端 build、本地 Native 回环（8 条观察请求）及空白检查通过。此项不覆盖旧的前端预侦察/单 Agent 路径，也不提供锁定 Node/Chrome、通用进程/网络沙箱、签名能力包或浏览器子请求代理；沙箱与 Stage 9–11 均未完成。未触达外部目标 URL。

2026-09-25 协作线程/人工指令绑定增量：过去对话页的协作线程仅过滤消息，输入框发出的草案仍没有线程归属。现在发送时传入所选线程，服务端只接受团队频道、当前 root 的目标、当前 root 的 mailbox correlation 或 assignment；未知/跨 root 线程拒绝。线程键进入草案哈希、确认后的指令记录与 payload、状态时间线，并在确认及 Coordinator claim 前重新核验；已落库草案线程被改写则拒绝。旧库自动补 `team` 列，旧版无线程草案的原哈希仍可只在 `team` 频道校验。回归先红后绿，覆盖伪造线程、字段改写、旧库加列、刷新后时间线；Rust 582 项及历史导入器 22 项、Clippy 严格检查、前端 build、fmt、空白检查通过（串行离线全门禁与 Native 回环仍需在本条之后复核）。这只是频道归属与安全确认，不代表独立频道摘要、完整群聊编排、EvidenceGap 自动补证、Stage 10 专家或沙箱已完成。

2026-09-25 线程兼容与最终门禁复核：补测数据库里真实存在的其他 root 消息关联键不能被当前任务当作聊天线程；模拟迁移前的无线程草案哈希仍能在 `team` 确认，迁到目标线程则因完整性校验被拒。串行全门禁首次发现授权控制的本地回环测试在 macOS 上因 accepted socket 继承非阻塞模式而偶发 `WouldBlock`；测试服务端现显式切回阻塞读，未改生产目标访问逻辑。最终 `cargo test --offline --all-targets --all-features -- --test-threads=1` 为 lib 584、历史 importer 23 全通过；严格 Clippy、fmt、前端 build、Native 本地双端口回环（8 条观察请求）、`git diff --check` 均通过。Vite 主 chunk 仍大于 500 kB。沙箱工具供应按 `NEST_LIGHTWEIGHT_SANDBOX_TOOL_SUPPLY_2026-09-25.md`：扫描时不得临时拉包，Kali 仅可选兼容 lane；本轮未实现完整隔离适配器、能力包签名/固定浏览器二进制或全部旧浏览器入口迁移，也未部署或访问用户的外部授权 URL。

2026-09-25 浏览器降级覆盖账本修正：多 Agent 无隔离浏览器时，`browser_action` 仍只允许 Broker 对已定位只读 URL 发一次实际 HTTP 请求；历史动作图里的 `triggeredApis` 只作为 `attributedApis` 展示，不再产生本次新端点/参数或浏览器动作覆盖。新增回归同时验证多 Agent 实际仅观察到导航 GET、旧 API 不入账，以及无浏览器的单 Agent 降级不会领取浏览器覆盖。定向测试先红后绿；最终串行 Rust 全目标全特性 lib 584、历史 importer 23 通过，Clippy `-D warnings`、fmt、前端 build、Native 回环（8 条观察请求）和 `git diff --check` 均通过。Vite 主 chunk 仍提示超过 500 kB。这是虚假进度修正，不代表浏览器沙箱、旧浏览器入口迁移、Stage 9 自动补证或真实授权目标验收完成；未访问外部 URL。

2026-09-25 授权控制逐侧断连回归：本地回环在 owner、cross、tester 各侧分别发送残缺响应，证明请求先持久认领，即使目标已收到但响应未知，同一 attempt 不会补发；该侧无可用 artifact、无待审候选或公开 finding，失败释放目标 lane 和预留请求预算。先发现测试 SQL 误用不存在的 `id` 列并修正，仅增加测试/文档，未改目标请求生产路径。串行 Rust 全目标全特性 lib 587、历史 importer 23 全通过；Clippy `-D warnings`、fmt、前端 build、本地 Native 回环（8 请求）和 `git diff --check` 均通过。仍未覆盖进程崩溃恢复、证据落盘竞态、真实目标账号矩阵和完整沙箱适配器；本轮未访问外部 URL。

发布/暂停竞态续修：授权漏洞候选写入改为在 `BEGIN IMMEDIATE` 内检查当前 scan attempt 的 `scanning` 状态，并在提交前复核；Reviewer 启动前和已确认 finding 发布事务中也复核 attempt。新增事务内模拟暂停的触发器回归：即使 Reviewer 已完成并且决策已 ack，暂停使发布回滚，既无公开 finding，也无半成品候选状态；另有候选快照被修改时仍拒绝整批发布。旧多 Agent 夹具曾把活动状态错误写为 `running`，现改为真实的 `scanning`。最终串行全目标全特性 Rust lib 589、历史 importer 23 全通过；严格 Clippy、fmt、前端 build、本地 Native 回环（8 请求）、`git diff --check` 通过。Vite 主 chunk 759.93 kB，仍有拆包警告。本项只封住已测的应用内暂停竞态，不证明进程崩溃恢复、数据库外部并发写入、完整工具沙箱或真实授权目标验收完成；未访问外部 URL。

轻量沙箱下一实施关口仍按 `NEST_LIGHTWEIGHT_SANDBOX_TOOL_SUPPLY_2026-09-25.md`：先实现能力包的签名/摘要/平台 manifest 与任务外原子导入，再锁定 Node/浏览器实际二进制、增加逐 run 隔离适配器和浏览器所有子请求 Broker 约束。当前仅固定随包 JS worker 与 parser 摘要、多 Agent 无隔离浏览器时 fail closed；不可把 `capability_bundles/*.json` 角色能力目录、手动环境安装或 `core` 的 Rust Broker 误称为通用 worker 沙箱。Kali VM 只是需要特殊工具时审批启用的兼容 lane，普通 HTTP/协作/Reviewer 不依赖它。

离线工具供应第一道验签原语：新增 `tool_supply.rs`，对严格 schema 的 manifest 信封使用**外部提供且未撤销的信任公钥**验 Ed25519，固定签名域，校验包字节 SHA-256、平台、入口路径、限制值以及 browser/source 的网络策略。4 条本地回归覆盖可信包、摘要/签名篡改、撤销、未知字段、跨平台、直接联网策略和路径越界。串行全目标全特性 Rust lib 593、历史 importer 23 全通过；Clippy `-D warnings`、fmt、前端 build、Native 回环（8 请求）、`git diff --check` 通过，未跟踪源文件与文档无尾随空白。Vite 主 chunk 仍 759.93 kB。**此模块尚未接入任务外导入或任何扫描执行**：没有应用可信根管理、归档安全解包、只读原子缓存、Node/浏览器二进制锁定、隔离启动/网络出口或再校验；不能将 4 条测试称为完整工具供应链验收。本轮未访问外部 URL。

离线候选缓存下一增量：`tool_supply.rs` 新增仅供内部调用的任务外 `stage_offline_tool_candidate`，验签/摘要通过后才取得现有跨进程准备租约；以 64 MiB 上限将不透明包写入私有临时目录，落盘再验，按 id/digest 原子发布并记录 `tool_candidate_staged` 审计，重复导入重新验证缓存。无效包不会占住租约；活跃任务拒绝导入，已发布缓存被篡改则拒绝且保留持久租约供人工核对。Windows 路径尚未审核，预先返回 `unsupported_tool_cache_platform`。新增 2 条回归后，串行全目标全特性 Rust lib 595、历史 importer 23 通过；严格 Clippy、fmt、前端 build、Native 回环（8 请求）和 `git diff --check` 通过。**不开放 UI/Agent 命令，也不声称工具已安装可运行**：尚无应用独立可信根/管理员认证/批准撤销、流式大包、归档解包、跨平台 TOCTOU 加固、Node/浏览器固定二进制或进程与网络隔离。Vite 主 chunk 759.93 kB 仍有拆包警告；未访问外部 URL。

候选缓存复验加固：先以新增测试证明额外目录项可被误接受，再在缓存命中与落盘复验时要求候选目录恰好包含 manifest/blob 两项，Unix 包文件不得有硬链接；额外文件或与外部路径硬链接的缓存拒绝，并保留准备租约以待人工核对。工具供应合同补充了能力申请→任务外管理员准备→固定摘要批准→后续任务使用的决策，以及按能力缺失降级规则。定向 `tool_supply` 7 项、串行全目标全特性 Rust lib 596 和历史 importer 23 项、严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求和空白检查均通过；Vite 主 chunk 759.93 kB 的拆包警告仍在。仍未提供可信管理员根、安全解包/不可变执行句柄、完整隔离适配器或浏览器逐请求 Broker，不能称沙箱已上线；未访问外部授权 URL。

用户指令证据引用补强：原 `referencedFactIds` 将聊天里的 `G-17` 等编号误认作证据、却忽略真实 `ev-<32 hex>` 节点。现在聊天编号仍保留在脱敏文本中，但结构化 fact refs 仅收录真实格式；草案创建、用户确认和 Coordinator claim 都会核验节点存在、属于当前 root、不是 inferred/假设/合同/候选结论节点，且尚未被后续节点取代。不存在或跨 root 的引用成为带 reason code 的拒绝草案；确认后才失效的引用不再 claim，持久指令带明确拒绝码。旧草案/JSON/SQLite 不作内容重写；其过往非图引用可读，但不会变成当前执行权。回归先红后绿覆盖缺失、跨 root、假设节点、有效引用、确认前 supersession 和 claim 前 supersession。最终串行全目标全特性 Rust lib 597、历史 importer 23 项、严格 Clippy、fmt、前端 build、Native 本地回环 8 请求及空白检查均通过；Vite 主 chunk 759.93 kB 的拆包警告仍在。此项只是聊天证据引用闭环的一段，Stage 9 的补证签发、证据 revision 重审及完整沙箱仍未完成；未访问外部授权 URL。

工具供应流式候选增量：在原 ≤64 MiB 内存字节候选之外，新增**仅内部调用**的本地文件流式导入，签名 manifest 验证后两遍 64 KiB 分块摘要校验，单包上限 1 GiB；首遍失败不认领环境准备租约，认领后复制、缓存复验或原子发布失败保留租约待人工排查。缓存命中也改为分块摘要复核；拒绝末级 symlink、非普通文件和 Unix 硬链接。失败回归先红，新增大于 64 MiB 的稀疏包、坏摘要及链接拒绝测试。串行全目标全特性 Rust lib 599、历史 importer 23 全通过；Clippy `-D warnings`、fmt、前端 build、本地 Native 回环 8 请求及空白检查通过，主 JS chunk 759.93 kB 警告仍在。此增量**没有** UI/Agent 导入入口、可信管理员根/审批、解包、固定 Node/Chrome、真正执行沙箱或浏览器逐请求 Broker，仍不得把候选包显示为可用工具；旧前端侦察仍可能用宿主运行时。未访问外部授权 URL。

Reviewer 事实版本增量：先红后绿新增同 root 的证据版本回归。`review_fact_refs` 冻结清单现在先复验 Web Broker artifact 字节、node 自然键和 child/assignment 归属，再仅允许**更高 revision 的已验证事实**取代旧事实；推断节点、无效 artifact 或同 revision 声称不能退役真实观察。证据图写入侧也拒绝同/更低 revision 或不同 kind 的 supersession，保持版本链单向且语义一致。这样只有新事实才有资格触发 insufficiency 后的下一次审查，模型改写和虚假 successor 不会构成证据。单元测试用独立 run/assignment 与独立 Broker 格式 artifact 模拟后续版本，**没有向真实目标发送第二次请求，也没有实现自动补证调度**。串行全目标全特性 Rust lib 600、历史 importer 23、严格 Clippy、fmt、前端 build、Native 本地回环 8 请求和空白检查通过；Vite 主 chunk 759.93 kB 警告仍在。工具供应文档补充本地优先能力解析和 Kali/raw-socket 工具只可离线分析的约束。Stage 9 真正的 EvidenceGap 合同签发/新 revision 调度、管理员可信根与审批、可执行隔离适配器和浏览器全子请求 Broker 仍未完成；未访问外部授权 URL。

工具供应决策补充（文档限定，无生产代码变更）：`NEST_LIGHTWEIGHT_SANDBOX_TOOL_SUPPLY_2026-09-25.md` 将管理员任务外准备、任务前只读能力解析、run 内隔离执行拆成互不代用的接口；明确静态 `CapabilityBundle` 不是安装状态，现有签名缓存只是不可执行候选。增补注册表主键/审计字段、attempt/assignment/lease/digest 冻结键、恢复复验、缺能力只关闭对应 lane、从无网络静态 worker 到浏览器的纵向切片顺序和逐项失败回归。**这些仍是实施合同，不代表可信管理员根、审批、worker 隔离或浏览器逐请求 Broker 已上线。**本条仅核对文档和 `git diff --check`，未重跑 Rust/前端测试；未部署或访问外部授权 URL。

Evidence revision 祖先链增量：新增 `agent_evidence_revisions` 元数据和自动登记，重开旧库时保留原 node/edge 并事务性替换旧“端点必须同 revision”触发器；边只能连接同 root 的本 revision 或真实父链节点，未来和迟到的非祖先 revision 在仓库层与直接 SQL 层均拒绝。新增不可凭数字大小冒充祖先的快照读取，先红后绿覆盖旧库幂等升级、跨版本边、版本取代和分叉；Reviewer 的已验证事实退役也必须确认父链。全量离线串行 Rust lib 603、历史导入器 23、严格 Clippy、fmt、前端 build、Native 本地回环 8 请求通过（读侧父链加强后仍须再次全量复核）。这仍未签发 EvidenceGap 补证合同，`manifest_hash` 还没有冻结，未实现动态目标补证与自动复审；浏览器隔离和管理员工具批准同样未完成。未访问外部授权 URL。

证据快照/复审分叉加固：新增先红后绿回归，禁止 `inferred` 取代 `source_derived`/`observed`，也禁止 `source_derived` 取代 `observed`；写入侧拒绝降低事实来源强度，读取侧对直接 SQL 遗留的降级取代报错而非默默退役事实。Reviewer 冻结已验证 HTTP 事实时只读取当前最高 revision 的真实父链，迟到的较小 revision 即使拥有真实 artifact、child 和 assignment，也不进入当前候选 manifest、不触发错误复审。全量离线串行 Rust lib 604、历史导入器 23 通过；严格 Clippy、fmt、前端 build、Native 本地回环 8 请求和 `git diff --check` 通过。此项仅加固数据/复审边界，不等于 EvidenceGap 合同签发、manifest hash 封存、受控新 revision 调度或工具沙箱上线。仍未部署或访问用户提供的外部授权 URL。

Reviewer 请求封存增量：`agent_review_requests` 新增独立于 candidate revision 的 `evidence_revision` 和 `manifest_hash`，旧库加列但历史未封存请求不允许当成新决策重放。创建请求的写事务内把 candidate 原文、证据 revision 列表、节点/边字段和重新核验的 Broker HTTP fact refs 纳入摘要；决策、重放与 finding 发布事务复验，变动即拒绝沿用旧确认。旧的 `agent_evidence_revisions.manifest_hash` 暂不冒充候选摘要：多个候选可能共享一个图 revision。新增重启后复验、候选篡改和新 revision 阻止旧决策回归；一次全量测试发现旧“仅追加不可信节点、事实仍不足”路径被过度拦截，修为只保留未完成状态、不发布也不自动再次调用 Reviewer，真正新增可验证事实仍允许新的 candidate revision。离线串行全目标全特性 Rust lib 605、历史导入器 23、严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求与空白检查通过；主 JS chunk 759.93 kB 警告仍在。此摘要是请求级防旧结论复用，不是完整的不可变 artifact 封存/崩溃恢复，也未签发 EvidenceGap 补证合同或实现隔离工具供应；外部授权 URL 未访问。

工具候选缓存身份修复：`tool_supply.rs` 原以 `id/digest` 定位，两个签名版本复用同一包字节时发生目录冲突。现在内存和流式两条导入路径均按 `id + version + platform + digest` 的域分隔哈希目录定位，旧目录保留但不自动重用或批准；准备审计也记录这个身份键，不再把同摘要不同版本记成同一事件。候选包仍为不可执行、不可信任的本地缓存，**没有**管理员独立可信根、审批/撤销注册表、能力冻结或隔离 worker。同包不同版本及审计区分回归通过；完整离线串行全目标全特性 Rust lib 606、历史导入器 23、严格 Clippy、fmt、前端 build、Native 本地回环 8 请求与空白检查通过；主 JS chunk 759.93 kB 警告仍在。未部署，未访问用户提供的外部 URL。

证据文件读取边界增量：多智能体 Broker HTTP 事实在写入图和 Reviewer 冻结时使用固定的 `agent-http` 目录句柄及 `openat`、`O_NOFOLLOW`、有界普通文件读取；拒绝同内容硬链接和替换为符号链接的证据目录。另发现身份差异记录仍通过普通路径读取，补上同一有界目录句柄规则；先红后绿的回归验证了硬链接与目录替换均不可进入确认判断。非 Unix 平台尚无审核后的等价实现，当前拒绝这种 artifact-backed 事实提升；`target_dir` 更上层的完整不可变封存和写入侧目录句柄化仍未完成。完整离线串行全目标全特性 Rust lib 607、历史导入器 23、严格 Clippy、fmt、前端 build、Native 本地回环 8 请求及 `git diff --check` 通过；前端主 JS chunk 仍为 759.93 kB 并有体积警告。此加固不等于 Stage 9 补证调度、管理员工具批准、隔离 worker、浏览器逐请求 Broker 或完整发布验收；未部署或访问外部授权 URL。

证据写入边界增量：Unix 上 HTTP 原始请求/响应和身份差异 artifact 的子目录经目标目录句柄 `mkdirat/openat`、`O_DIRECTORY|O_NOFOLLOW` 打开，文件经固定目录句柄 `openat(O_EXCL|O_NOFOLLOW)` 创建并同步；老目录收紧至 0700。响应体与可选请求内容先写，metadata JSON 最后发布；崩溃后仅残留 `.body/.request` 的槽位不复用。先红后绿回归证明 `target_dir` 或其 `agent-http/agent-diff` 子目录改为符号链接时不会写入外部目录，并验证残留负载不冒充完整记录。Windows 等非 Unix 平台没有审核后的安全写入适配器，当前明确报 `unsupported_agent_artifact_write_platform`，该平台的 Native HTTP lane 因而**尚不可用**，不能将其标成跨平台完成。更高层目录组件、并发替换后的完整不可变封存、密钥加密与跨平台适配仍待做。完整离线串行 Rust lib 609、历史导入器 23、严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求和空白检查通过；主 JS chunk 759.93 kB 警告未变。仍未部署或对外部授权 URL 启动扫描。

证据父目录与槽位加固增量：Unix 上 Broker 写入与 Reviewer/差异记录复验现从文件系统根逐组件使用固定目录句柄 `openat(O_DIRECTORY|O_NOFOLLOW)` 遍历，写入仅通过 `mkdirat` 创建不存在的组件；任一上层符号链接都拒绝，不能借一个真实 `target_dir` 绕过最后一层的 no-follow。HTTP 和差异记录的空槽判断也改为同一目录句柄的 `fstatat(AT_SYMLINK_NOFOLLOW)`，不再经另一路径判断后写入。新增先红后绿的上层符号链接回归；测试夹具明确使用 macOS 临时目录的真实路径，避免系统 `/var` 别名被误当成产品必须允许的应用目录。离线串行 Rust lib 610、历史导入器 23、严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求和空白检查通过。仍缺任务根目录身份封存、目录被并发重命名时的事务级不变性、敏感原始文件任务密钥加密，以及 Windows 安全适配；这不等于 Stage 9 补证合同、隔离 worker 或发布验收。未部署、未访问外部授权 URL。

轻量沙箱/工具供应说明增量：实施合同新增按 Coordinator、Reviewer/Deep Investigator、WebExecutor、Code/CI Analyzer 的能力矩阵，以及 Core/Browser/Code 安装档次和任务外准备→独立审批→attempt 冻结→恢复复验流程。环境 UI 文案现在明确旧远程 Worker 宿主依赖不是 Native HTTP 的统一前置条件，检测到 Node/Docker 不能冒充“Agent 工具已批准且隔离”；Windows Native HTTP artifact 写入适配器仍不可用。本轮只改设计与误导性 UI 文案，**未**建立管理员信任根、批准/撤销注册表、SandboxAdapter 或浏览器逐请求 Broker；扫描中动态拉工具仍不允许。前端 `npm run build` 与 `git diff --check` 通过，主 JS chunk 760.27 kB 警告仍在；未部署或访问外部授权 URL。

沙箱生命周期合同增量：明确“同任务共享”仅限固定只读包、Broker 和已封存证据，实际 worker/browser/profile/身份按 child run 隔离；同一 run 的多次调用可温态复用，跨任务仅复用不可变缓存，暂停/终态回收执行实例并以持久证据恢复。补充浏览器、任意代码执行与文件处理的独立能力门槛、缺工具支线降级、惰性启动、资源排队、冷热启动 P50/P95 与清理收据验收。此项仍是设计约束，**没有**实现进程/浏览器沙箱或测得平台性能；不应将文档写入视为隔离完成。未访问外部授权 URL。

Reviewer 补证封存边界增量：Deep Investigator 现在只接受数据库中已完成、决策消息已送达并确认的独立 Reviewer `insufficient_evidence` 请求；调度前核对缺口和候选、事实图/文件 manifest，模型输入改取封存 `candidate_json`，不再把可变 `context.evidence` 冒充 `frozenEvidence`。模型返回后、记录补证提案前再次核对封存；请求缺失、缺口不符或 manifest 篡改均不派发新的 Deep Investigator。回归覆盖可变内存证据、无封存请求、缺口错配和候选篡改。离线全目标全特性 Rust 主库 611、历史导入器 23、严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求及 `git diff --check` 通过；主 JS chunk 760.27 kB 仍有体积警告。这尚不等于自动签发新的 EvidenceGap 补证合同或验证真实授权目标；未部署或访问外部 URL。

调度暂停边界增量：Coordinator lease 的有效性不再单独允许签发、启动 child 或在暂停后续租运行中 executor。Scheduler 在 `IMMEDIATE` 写事务内核对当前 scan 仍是相同 attempt、`scanning` 且未被软删除；暂停、attempt 替换或删除后不产生新 assignment，也不延长已暂停 child 的 capability 租约。若 child 已租用但尚未启动，暂停后的 `start_child_or_release` 会将它标为失败并释放 lane、capability 和预留预算。回归先证明原实现会在暂停后继续调度/启动/续租，现覆盖暂停、替换和删除。此门禁不替代运行中模型的取消令牌、目标请求 Broker 逐次校验，也没有完成 Stage 9 的新补证执行合同。离线全目标全特性 Rust 主库 615、历史导入器 23、严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求及空白检查通过；主 JS chunk 760.27 kB 警告仍在。未部署或访问外部授权 URL。

Mailbox 暂停边界增量：有效租约不能在已暂停/替换/删除的 attempt 上继续发布、投递或确认协作消息。`send` 和 `acknowledge` 将当前 attempt 条件放进原子写入谓词，投递在 `IMMEDIATE` 事务里复核相同条件；已存在的 dedup 消息也不能在暂停后冒充新一轮成功。共同的活跃 attempt 检查移入 lease 模块供 Scheduler 和 mailbox 复用。回归先证明暂停后仍可写入一条协作消息，现确保暂停后消息总量、待投递与已投递未确认状态不变。授权控制组途中暂停时不再发送正常 `authorization_result`，失败原因仍由 child 终态记录，lane 和预算照常释放；3 条中途暂停回归已覆盖。运行中的模型取消、预算结算和 child 失败清理仍独立运作；这不是完整的动态补证合同或主机层 Agent。离线全目标全特性 Rust 主库 616、历史导入器 23、严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求及空白检查通过；主 JS chunk 760.27 kB 警告仍在。未部署或访问外部授权 URL。

Web→主机聊天入口增量：先红后绿证明 Web 任务内“SSH 登录服务器”、通过 Web 参数执行 OS 命令等指令原先会被归为普通只读草案；现在这类明确越界请求以 `host_boundary_not_supported` 失败关闭，不可确认入队，协作 UI 明示当前没有主机审批后端/执行器。正常 Web 证据优先级调整保持可用。这只是早期自然语言入口拦截，**不是**语义完整的主机意图识别、后端 `web_only` 执行面强制、HTTP/浏览器逐请求 Broker 判别、`host_boundary_candidate` 落库或独立主机审批链；模型与网页内容仍必须受结构化工具和 Broker 硬边界约束，不能用关键词门禁宣称主机隔离已完成。离线全目标全特性 Rust 主库 617、历史导入器 23、严格 Clippy、fmt、前端 build、Native 本地回环 8 次请求及空白检查通过；主 JS chunk 760.78 kB 提示仍在。未部署或访问外部授权 URL。

Broker 停止边界增量：先红后绿证明暂停、attempt 替换或软删除后，未过期的 WebExecutor capability 仍能通过 `agent_authorize_tool`，且旧 HTTP artifact 仍能被提升为 Reviewer 可见的 request fact。现在单 Coordinator 与多智能体工具授权都先核对 scan 当前 attempt 仍为 `scanning` 且未软删除；HTTP 事实提升的 `IMMEDIATE` 写事务也将同一条件纳入写入谓词。已暂停的工具调用仍以 `cancelled` 留下原有拒绝审计，不会误归为无权限；实际 HTTP 发送边界会再次授权。回归覆盖暂停、替换、删除以及单 Coordinator 快速路径。逐次状态检查不能消除状态检查与真实网络发送之间所有竞态，也不能替代执行沙箱、主机边界或 Stage 9 动态补证合同。离线全目标全特性 Rust 主库 619、历史导入器 23、严格 Clippy、fmt、前端 build、Native 本地回环 8 次请求及空白检查通过；主 JS chunk 760.78 kB 警告仍在。未部署或访问外部授权 URL。

冻结执行计划修复：`agent_runs` 中同 scan/attempt/target 的 Coordinator 计划现在经 SQLite `IMMEDIATE` 事务比对。相同 JSON、哈希与后端可幂等重放；后端、预算等任何冻结字段变化均以 `agent_attempt_plan_frozen_conflict` 拒绝，不覆盖历史 `LegacyRemoved` 行。首次记录用原子 INSERT，不经可更新既存 run 的通用 upsert；数据库打开、锁定、插入、提交失败均向调用方传播，权威计划写入成功前不更新旧 checkpoint 投影。新增回归覆盖冲突保持原样、数据库写失败不推进投影、两连接并发冻结只有一个胜出，以及历史/新 attempt 的后端选择。离线串行全目标全特性 Rust 主库 622、历史导入器 23 项通过；严格 Clippy、fmt、前端 build、Native 本地回环 8 次请求及空白检查通过。主 JS chunk 760.78 kB 的拆包警告仍在。该修复不等于 Stage 9 补证合同、`web_only` 后端执行面、完整 worker 隔离和真实目标验收已完成；未部署或访问外部授权 URL。

跨 attempt 恢复与详情一致性增量：先红后绿复现父 attempt 缺失权威计划时会误继承另一 attempt 的 URL checkpoint，以及旧 attempt 幂等重放倒写最新任务详情。现在仅在 checkpoint 内嵌 attempt 编号与所需父 attempt 完全相同时允许历史 fallback；无法确认归属的旧投影安全拒绝继续。权威计划和旧详情投影在同一个 `IMMEDIATE` 事务中提交，仅最新已记录 attempt 更新投影；投影失败回滚新权威计划，不留下两份不同真相。新增三个回归覆盖误继承、倒写和投影失败原子性。Stage 9 的 EvidenceGap 提案目前仍固定为 `deferred_requires_new_evidence_revision`，不签发动态补证合同，不能称完整协同闭环。离线串行全目标全特性 Rust 主库 625、历史导入器 23 项通过；严格 Clippy、fmt、前端 build、Native 本地回环 8 次请求及空白检查通过。主 JS chunk 760.78 kB 警告仍在；未部署或访问外部授权 URL。

Web 执行面冻结增量：新建的 Web attempt 写入 v2 执行计划，显式 `executionSurface=web_only`，与记录网站类型的 `surface` 分离；历史 v1 计划保留原 JSON/哈希继续恢复，v1 伪造执行面、v2 非 `web_only` 和未知版本均拒绝解析。工具 Broker 每次调用按 scan/attempt/target 重读权威计划并与运行上下文比较；缺失或变化拒绝，HTTP 实际发送前复用此校验，越界时不消费请求预算。回归覆盖旧计划兼容、未知执行面、权威计划篡改/缺失以及直接 HTTP 发送拦截。离线串行全目标全特性 Rust 主库 627、历史导入器 23 项通过；严格 Clippy、fmt、前端 build、Native 本地回环 8 次请求及空白检查通过；前端主 JS chunk 760.78 kB 拆包警告仍在。**这只落实了执行面冻结与工具入口复核**：仍无结构化的逐请求主机意图分类、`host_boundary_candidate` 落库、独立主机审批合同/沙箱，不能宣称能够识别任意 Web 参数中的命令执行意图，也未部署或访问外部目标。

EvidenceGap 提案绑定加固：Deep Investigator 的 `missingEvidence` 现在必须逐项等于已封存的 Reviewer 缺口；`nextStep`、合同类型和目标请求估算也必须一致（人工审查/现有证据审查均为零目标请求，单个已批准控制组为三侧 GET）。首次发送与历史 mailbox 重放都重新核对，避免外层消息保持原缺口而内部提案偷换问题；测试覆盖替换缺口、无效合同成本及重放篡改。离线串行全目标全特性 Rust 主库 628、历史导入器 23 项通过；严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求及空白检查均通过；主 JS chunk 760.78 kB 警告仍在。**这仅是提案真实性与自洽性门禁**，Assessment 仍暂缓，不签发动态补证合同，也没有自动事实采集和 Reviewer 再审闭环。主机边界候选落库、隔离沙箱和真实授权目标验收同样未完成。

Authorization 专用 GET 执行面加固：三侧控制组在原子领取请求的事务内复用通用工具的权威冻结计划检查，实际发送前再次复核；计划缺失、已变化、未知执行面均不发送，领取前拒绝不占用请求预算。测试在本地回环服务器上逐一覆盖 owner/cross/tester 三侧的上述失败及恢复有效 `web_only` 计划后的正常三次 GET；测试夹具现持有真实冻结计划，不再用占位 plan hash 绕过校验。离线串行全目标全特性 Rust 主库 628、历史导入器 23 项通过，严格 Clippy、fmt、前端 build、Native 本地回环 8 请求与 `git diff --check` 通过；主 JS chunk 760.78 kB 拆包警告仍在。该修复不等于 Stage 9 动态补证合同、逐请求主机意图识别、独立沙箱或真实授权目标验收；尚未部署或访问外部 URL。

EvidenceGap 未来合同语义修复：新的 Deep Investigator V3 提案不再把尚不存在的补证控制组叫作 `operator_approved_control_group`，而是明确建议 `new_attempt_control_group_request`。Coordinator 仍固定 deferred，Assessment 写入 `newAttemptRequired` 和 `targetRequestsGranted=0`，不会从当前 Web attempt 签发三侧 GET。mailbox 重放核对版本、原因和新 attempt/零请求约束；已确认的 V2 提案仍按历史语义只读重放，不会重派发。聊天时间线的白名单投影同步支持 V3，展示人工新任务前提及本轮零授权，拒绝把篡改成“已批准”的 V3 消息投影为正常卡片。专项测试覆盖冒称已批准合同、篡改授予请求数、篡改新 attempt 标记、V2 历史消息兼容及 UI 投影。最终修改后的离线串行全目标全特性 Rust 主库 629、历史导入器 23 项、严格 Clippy、fmt、前端 build、Native 本地回环 8 请求和 `git diff --check` 均通过；JS chunk 761.42 kB 警告仍在。**这只是提案/时间线的安全语义，不是实际的补证合同签发、事实 revision 更新或独立 Reviewer 再审**；主机边界候选落库、隔离沙箱、安装包和真实目标验收未完成。

2026-09-25 历史 JSON 只读预览增量：canonical `import_*` 当前 membership 可以在扫描任务“证据”页展示有限的历史摘要，后端用 SQLite 只读连接，要求 `historical_external/unreviewed/readOnly/non-executable` claim；旧 `strix-<run id>` UI 任务关联时同时验证来源目录，避免只按前缀串线。预览不返回原始 envelope/CAS，不晋升为新的 Finding 或执行授权，删除标记与非当前 membership 不显示。回归使用原有 JSON 夹具验证上述边界。最终离线串行 Rust 主库 630、历史导入器 23 项通过，严格 Clippy、fmt、前端 build、本地 Native 回环 8 次请求和空白检查通过；主 JS chunk 762.76 kB 警告仍在。**此时 legacy `sync_strix_results` 仍自动写入 `sentinel_*`，其他结果页/统计仍读旧表；不能宣称 Strix 历史写入及读取路径完全剥离。**未部署，未访问外部授权 URL。

同日主机边界复核：当前 Web attempt 的 `web_only` 冻结与工具 Broker 检查已存在；聊天中明确的 SSH/主机指令会拒绝，但这只是早期文本防线，不是任意 Web 参数中主机命令意图的可靠分类。Master Plan §Web → 主机边界规定当前不实现可执行主机 Agent；默认关闭的“允许申请”只可形成非执行的候选卡，不能签发主机能力。`host_boundary_candidate` 持久化、独立 Linux 主机 scope/审批合同、逐请求主机意图复核和隔离执行器仍未实现，UI 不得暗示开启开关后可继续主机测试。

同日主机边界候选增量：**明确的聊天主机请求**在拒绝执行草案的同一事务中写入 `agent_host_boundary_candidates`，数据库限制其状态只能为 `recorded_only`、`execution_eligible=0` 且更新不可变；失败时连草案和协作事件一起回滚。候选通过既有协作序号进入团队时间线，重连/增量读取均可恢复，UI 没有批准按钮。候选只保留静态脱敏摘要和已核验的图事实引用，伪造引用不被提升为证据；普通 Web 建议不生成候选，也不产生主机 capability lease。已有数据库在初始化时补表；回归覆盖迁移、原子失败、伪造引用、时间线及零授权。最终离线全特性全目标 Rust 主库 632、历史导入器 24 项、严格 Clippy、fmt、前端 build、本地 Native 回环 8 请求及 `git diff --check` 通过；主 JS chunk 763.36 kB 告警仍在。**仅聊天中的显式词组得到候选记录**，Web 观察中的 RCE/主机线索自动归类、结构化逐请求主机意图拦截、独立主机授权后端和执行器仍未实现；既有 Strix 历史 writer 仍在。未部署或访问外部 URL。

2026-09-26 任务 trace 与历史导入增量：1.6.2 `coverage.json` 中的 `risk_area/outcome` 及未测 gap 在 canonical `import_*` envelope 中留存，不能生成原生漏洞或 checkpoint；过时的“旧 importer 仍写 `sentinel_*`”说明已更正。Native 任务 trace 现在优先读原生 `agent_runs/agent_events/agent_messages`，按最新 attempt 展示模型轮次、工具调用/结果、独立角色事件及 mailbox，任务路径为空但有 Native run 时仍列在 trace 列表。历史 trace 仍有读取旧 `run.json/.state/agents.db` 的路径，尚未迁到隔离 importer；该路径的详情现在逐层脱敏，避免把原始 Cookie 或工具参数直接返回给 UI。提示词 full 模式与 trace UI 已改为脱敏展示，不再误称原文；这不代表历史源文件在磁盘上已清理。本轮离线串行全目标全特性 Rust 主库 630、历史导入工具 24 项通过；严格 Clippy、fmt、前端 build、本地 Native 回环 8 次请求及 `git diff --check` 通过。主 JS chunk 768.11 kB 告警仍在。**Stage 5 的历史 trace 隔离、静态 allowlist、真实环境与安装包验收未完成，不得据此宣称整体完成。**

2026-09-26 历史 trace 隔离后续增量（更新上一段的未完成项）：历史 trace 的展示路径已改为只读取 canonical `import_*` 当前投影，不再打开外部 `run.json`、`.state/agents.db` 或 Hook JSONL；Native trace 仍优先读取原生账本。SQLite 消息、空会话、Hook 和旧提示词审计均由隔离 importer 接入。消息身份改为来源文件与行号，session 使用来源绑定的哈希，修复两个文件共 8 条消息曾被合并成 2 条的问题。adapter 升至 v2，导入签名包含 adapter 版本；旧包在再次显式导入/历史同步时会重新解析，不覆盖不可变 revision。旧 v1 投影若缺少来源/session 信息，需要从原始来源重新导入；不能将尚未重导的数据视为已经恢复了完整 trace，也不恢复直接读外部源文件的兜底。

本次补充的边界回归覆盖：旧任务别名必须匹配来源目录、非当前/tombstone/删除标记不可展示、claim 必须是历史未复核且严格 JSON 布尔只读/不可执行、新 attempt 不回退旧证据、SQLite 坏行明确报错而非静默丢失、同名 session/call/request ID 不跨来源串线、多 run 的目标与工具结果正确关联、源目录离线后仍能展示。用量按 Hook 文件和 run 分组，避免一个 run 的 Hook 覆盖整个任务；若同一 run 的 Hook 比快照少，保留快照并标为估算。历史提示词审计在最终接口返回时也保持 `exactModelRequest=false`，UI 区分历史 Hook 脱敏记录与原生账本，不再称为精确请求。

最终修改后的离线串行全目标全特性 Rust 主库 **637 项**、历史导入工具 **24 项**、严格 Clippy、fmt、前端 build、Native 本地浏览器回环 **8 次请求**及 `git diff --check` 全部通过。主 JS chunk **768.71 kB** 告警仍在。**这完成的是历史 trace 读取路径隔离及上述回归，不是整个 Master Plan 验收；Stage 5 静态 allowlist/REM 总验收、后续多智能体执行链逐项核验、隔离沙箱、安装包和真实环境验收仍待继续。** 未部署，未访问外部授权 URL，未增加主机执行权限。

若后续继续修改 Rust、前端、schema 或文档中的可执行约束，必须重新运行：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm run build
node tools/test_native_runtime.cjs
git diff --check
```

### 2026-09-26 配置边界归一化与资产接口修复

本轮先复现了配置界面写 `agentLocalFullPower` / `agentPromptAuditMode`、运行时却读另一组字段导致设置失效的问题。现由 `settings_migration.rs` 集中处理历史别名，DB 启动迁移、配置 API 读写、模型环境、预算和历史目录读取使用统一归一化；配置界面只消费中性字段，移除重复的旧字段迁移逻辑。旧 `strixRunsDirectory` 提升为 `historicalImportDirectories`，自定义归档目录不会因删除执行器设置而丢失。模型清单和纯 flat 配置均兼容，但显式中性空值、null、false、零值不允许被旧字段覆盖；未知自定义 JSON 字段保留。

迁移的读取和更新在同一事务中，错误不再被忽略。故障注入证明第二条配置写入失败时全部迁移回滚，原密钥和路径仍在；配置 API 保存失败、配置 ID 已不存在和非对象 settings 均不会清空现有默认配置。纯历史模型转换保留原 `legacy-default` 身份，不为通过测试而更改历史预期。

另一个先红后绿的回归证明：选择新模型后，清空本地密钥仍可能重新使用旧 flat 缓存中的密钥。现在活动 profile 独占自身模型名、部署类型、端点和凭据，不再借用上一模型的 flat 缓存；清空模型清单也不能继续用旧缓存启动。此项不宣称已取消所有云端环境变量兼容行为。资产统计接口的 Rust 字段从 `sent_to_strix` 修复为 `sent_to_agent`，序列化结果与前端现有 `sentToAgent` 一致，避免统计卡读取未定义字段；工作空间删除提示同步使用中性“扫描任务”文案。

已有活动残留规则的 baseline 全部归零，并禁止重新添加豁免；扫描器通过逐模式嵌套正例验证，不再要求生产源码必须留一处残留来证明扫描器有效，读取失败也不再被静默忽略。移除的是不相关测试里的真实公共 API 域名，改用保留测试域名，不影响跨域来源验证语义。**这些是已有规则的零残留检查，不是覆盖所有 Strix 拼写/动态调用的完整 REM-012 allowlist；不能据此宣布 Stage 5 或整个 Master Plan 完成。** 后续仍需逐项核验全文残留分类、真实多智能体闭环、沙箱、安装包和真实环境。未部署、未访问外部授权 URL、未扩大 `web_only` 权限。

最终验证：离线串行全目标全特性 Rust 主库 **648 项**、历史导入工具 **29 项**通过；严格 Clippy（`-D warnings`）、fmt、前端 build、Native 本地浏览器回环（**8 次请求**、身份隔离通过）及 `git diff --check` 全绿。主 JS chunk **767.07 kB** 告警仍在。全量 Rust 日志为 `/tmp/oviraptor-20260926-config-final.log`，其余本轮最终日志为 `/tmp/oviraptor-20260926-config-{clippy,build,native}.log`。这记录的是本轮修改后的本地验证，不是安装包、外部 URL 或完整 Master Plan 验收。

### 2026-09-26 旧恢复协议退出、正式配置字段与全文残留审核

详细审核记录见 `NEST_BACKEND_RETIREMENT_AUDIT_2026-09-26.md`。本轮先红测试确认旧恢复文案仍会把 34 个工具删到 1 个，并影响未超上下文的短请求；删除该识别/过滤逻辑后两种请求均通过回归，上下文压力触发的中性压缩保持有效。历史更新说明移入默认折叠的归档，并明确不代表当前功能。

纠正上一节的字段名：Master Plan §11.6 要求正式键为 `legacyArtifactDirectories`，不是 `historicalImportDirectories`。本次按正式键优先、其次中间键、最后最旧键迁移，兼容已经保存的中间版本 JSON；显式正式空值/null 不被覆盖。删除前端后端选择器、默认配置和迁移输出中的 `agentBackendPolicy`。代理/前端证据预算的旧别名只在统一迁移层处理。

新增 REM-012 全文字面量与路径守卫，逐文件记录类别、理由和完整上下文摘要：**59 个文件（importer 14、migration 6、fixture 34、historical_label 5）**。扫描涵盖源码和构建/运行资源，不按目录中文件扩展名跳过；新增、失效登记、内容变更、缺失/不可读输入、符号链接均失败。守卫自身有大小写、二进制、路径、上下文改变和错误输入正反例，并在本轮后续测试文件修改后实际失败，证明不是只检查一份自报成功的清单。已有活动符号 baseline 继续保持零豁免。

同时纠正 REM-001 验收证据不足：原假 CLI 没放入 PATH，哨兵不存在不能证明未尝试调用。新的 POSIX 隔离子进程先用真实 CLI 名称在 PATH 中完成正向触发，再执行 Web 每目标入口和 Code/Greybox/CI 源码分支，检查无旧 CLI 调用，并验证持久化 Native 计划和子进程完成回执。这里不把源码缺分析器分支冒充实际镜像验收，也不把独立分支测试冒充完整 Greybox UI 端到端测试。

最终代码验证：Rust 主库 **655 项**、历史导入工具 **30 项**，严格 Clippy、fmt、前端 build、本地浏览器回环 **8 次请求**和 `git diff --check` 全部通过。日志为 `/tmp/oviraptor-20260926-retirement-verified.log` 及 `/tmp/oviraptor-20260926-retirement-{clippy,build,native}.log`。主 JS chunk **767.50 kB** 告警仍在。

**本轮完成的是上述具体修复和本地验收，不是整个 Master Plan。** REM 全量证据矩阵、真实多智能体与人工指令/聊天闭环逐项核验、隔离沙箱、安装包、Linux 真实运行和外部授权 URL 测试仍需继续。未部署、未访问外部目标，未新增主机执行或扩大 `web_only` 权限。

### 2026-09-26 聊天异步状态、任务切换与时间线审核

详见 `NEST_CHAT_ASYNC_AUDIT_2026-09-26.md`。对实际 Vue SFC 增加可控异步回归，先分别取得 8/9、8/19、3/23 项失败证据，再修复：跨任务/项目回包污染、旧操作 finally 释放新操作、编辑中内容被清空、旧 attempt 草案提交、请求序号误当 DB 版本、时间线回滚、最近列表刷新丢弃历史页、分页耗尽状态重开，以及读取/监听异常未捕获。人工动作、状态、列表/分页和监听错误分离，拒绝原因不再被正常轮询清掉；监听失败可实际轮询补齐 DB 消息。未发送文字只按项目/任务/线程在当前组件内存隔离，不能称为跨刷新草稿恢复。

最终新增组件逻辑测试 **29 项**通过，命令 `npm run test:agent-dialog`；运行真实 SFC setup 与 Vue 生命周期，mock API/事件/宿主节点，不渲染真实 DOM、不替代 Tauri IPC 或视觉验收。Rust 主库 **655 项**、导入工具 **30 项**，严格 Clippy、fmt、前端 build、本地 Native 浏览器回环 **8 次请求**及空白检查通过。日志为 `/tmp/oviraptor-20260926-dialog-green.log` 和 `/tmp/oviraptor-20260926-dialog-{rust,clippy,build,native}.log`；主 JS chunk **769.64 kB** 警告仍在。

**目标保持未完成。** 多智能体执行链逐项验收、Stage 9 实际补证再审、Stage 10 剩余专家、线程/未读持久化、实际沙箱、安装包与真实环境仍需继续核对和实现。本轮未部署、未访问外部 URL，也未增加主机权限。

### 2026-09-26 人工指令并发、恢复与真实模型送达

详见 `NEST_DIRECTIVE_DELIVERY_AUDIT_2026-09-26.md`。先复现并修复双目标误拒绝、等待 SQLite 写锁期间旧租约仍可更新状态、非活动 attempt 继续领取/推进指令；写入与 fencing 校验使用同一 IMMEDIATE 事务。暂停/取消后的失败收尾不等同启动新工作。另复现 Native 在模型请求前把指令标为 applied，以及 claim 中断后无法恢复。

当前每轮从 DB 恢复同一有效租约下的 claimed/accepted 指令，保留确认约束原文且不重复累加 history。根 run/数据库/租约错误不再伪装成空 inbox。旧租约指令可见地 deferred 并要求重新确认；恢复时重查草案与证据完整性。成功模型响应的 `ModelRoundCompleted`、`model_received` 回执及聊天增量事件同事务提交或回滚；模型认证失败不产生送达成功。聊天明确提示送达不是动作执行，整体 Reviewer 通过不会把仅 accepted 的新指令批量变成 completed。

新增 6 项并发测试与 8 项送达测试。完整 Rust 主库 **669 项**、历史导入 **30 项**，聊天组件逻辑 **29 项**、严格 Clippy、fmt、前端 build、本地浏览器回环 **8 次请求**及空白检查通过。模型请求测试最终扩展到连续两轮、每轮恰好一份指令后，指令定向 **19 项**再次通过。日志前缀 `/tmp/oviraptor-20260926-directive-final-`；主 JS chunk **770.21 kB** 告警仍在。

**整体目标仍未完成。** 本轮解决送达与状态真实性，不把文本上下文冒充真实 priority change/proposal/Assignment。结构化调度动作、逐动作执行回执、任务结束时未落实指令的收口、长期约束上下文管理、旧租约重确认 UI，以及原 Master Plan 的其余验收仍需继续。未部署、未访问外部目标、未增加主机执行能力。

### 2026-09-26 已确认指令的实际队列优先级动作

详见 `NEST_DIRECTIVE_QUEUE_ACTION_AUDIT_2026-09-26.md`。先以失败测试证明明确优先级请求没有冻结动作，再实现 `prioritize_family` 分支：确认卡显示具体动作，Coordinator 在有效租约和活动 attempt 下稳定调整 Native pending queue，动作回执、状态转换及聊天事件同事务提交；只有提交成功才发布新内存队列。已完成规则在相同 scan/attempt/target/root 下逐轮重放，新发现的匹配工作也按优先级排序，不依赖 checkpoint 已保存顺序。每类只重放最后一条规则，保留全部历史回执。

不新增或删除待办、不扩大 scope、不改变预算；没有匹配工作则 `deferred` 并给出 `priority_no_matching_pending_work`，不能静默等将来激活。已在前的匹配项显示“优先级已保存”，不谎称改过顺序。完成态只代表调度规则已落实，不代表扫描、漏洞验证或 Reviewer 完成。已完成动作不再进入待落实的模型指令上下文；其他人工约束继续保留。

新增 **14 项**动作测试，覆盖事务回滚、幂等恢复、确认顺序、否定/复合表达、真实 SQLite 写锁竞争、目标隔离、预算不变、真实 Native 请求中的队列顺序及真实 WebExecutor child 读取 Coordinator inbox。聊天测试增加实际 SFC 模板输出校验，共 **30 项**通过；不是浏览器视觉或 Tauri IPC 验收。

最终 Rust 主库 **683 项**、历史导入工具 **30 项**、严格 Clippy、fmt、前端 build、本地浏览器回环 **8 次请求**和空白检查通过。日志前缀 `/tmp/oviraptor-20260926-actions-`；主 JS chunk **771.97 kB** 警告仍在。

**只完成了明确 priority change 分支，原整体目标仍未完成。** 复杂指令拆分、局部暂停/禁用、角色提案与 Assignment/child-run/mailbox/result 的完整连接、Stage 9 补证再审及原有其余验收继续保留。未部署、未访问外部目标，未增加主机执行或改变 `web_only`。

### 2026-09-26 独立只读角色提案与真实模型请求

详见 `NEST_DIRECTIVE_PROPOSAL_AUDIT_2026-09-26.md`。单角色 Mapper/Investigator 人工提案现已连接真实 Assignment、child run、预算、独立模型调用和请求/结果 mailbox；结果先落盘再结算收尾。实际 HTTP 回归发现一次预算可能被通用客户端的 503 自动重试消耗两次，本路径已改为单次传输。聊天明确区分评估已接收、建议未执行、失败和未知结果。Reviewer 预算底线不被评估挤占。

新增定向测试 12 项通过；全量 Rust 主库 **695 项**、历史导入 **30 项**、聊天 **31 项**、严格 Clippy、前端构建和 Native 本地浏览器回环 **8 次请求**通过。主 JS chunk **773.68 kB** 警告仍在。全量日志 `/tmp/oviraptor-20260926-proposals-full-rust.log`。

**整体仍未完成。** 审核文档明确保留 mailbox 全入口并发消费窗口、未知执行结果对账、旧租约重确认、多角色/Reviewer 再审、动态预算及真实沙箱/部署验收缺口。未访问外部 URL，未实现或启用主机 Agent；主机申请开关仅是未来设计，不能绕过 `web_only`。

### 2026-09-26 提案消息并发消费与原子收尾

上一节的提案 mailbox 并发窗口已通过失败测试复现并修复，同时发现并修复 ack 失败残留 delivery、已 ack 重放跳过 fencing/活动任务校验，以及已完成提案接受不匹配结果 ID 的问题。请求消费在一个事务中验证完整关联并写入投递/确认；预算结算、child 结束、结果 mailbox 与指令终态共用另一个事务，不持锁调用模型或目标。合法并发恢复幂等，源、租约、路由或回执不匹配仍拒绝。

新增 **6 项**测试，提案定向共 **18 项**；四工作线程运行完整入口、已落盘有效/无效响应恢复，以及收尾四类故障回滚均通过。全量 Rust 主库 **701 项**、历史导入 **30 项**、聊天组件 **31 项**、严格 Clippy、fmt、前端构建、Native 本地浏览器回环 **8 次请求**和空白检查通过。全量日志 `/tmp/oviraptor-20260926-proposal-concurrency-full.log`，修复前失败日志 `/tmp/oviraptor-20260926-proposal-concurrency-red.log`。主 JS chunk **773.68 kB** 告警仍在。

此修复仅关闭上述提案路径的并发缺口，不能推广为其他角色或完整 Master Plan 已通过。接下来仍须处理任务结束时未落实指令、结果未知调用和旧租约对账，再继续 Reviewer 补证再审、剩余专家及沙箱/部署验收。整体目标保持未完成。

### Strix 双守卫与历史兼容续核

本轮重新检查 REM-001–012 的相关代码与已有回归，未将上一轮产品设计讨论算成实现进展。发现活动符号扫描器只接受部分后缀、只接受目录入口，且自身的 scope 未包括生产 resources 和构建入口。**已有全文件字面量/哈希守卫没有这些遗漏**，因此这是双守卫覆盖不一致，不是发现生产仍能执行旧后端。

修改 `agent_tests_backend_residual.rs`：支持现有 `.mjs` 及常见脚本/构建格式、显式文件 scope，拒绝符号链接和特殊文件；新增格式覆盖、单文件入口、固定必要 scope 三个反例。`residual_baseline.json` 增加 resources、package/Vite/Cargo/Tauri/build 入口，全部活动 baseline 继续为空。完整复核测试文件后，仅更新 `backend_retirement_allowlist.json` 中既有该文件的说明和摘要；保留 16 处历史测试字面量，不增文件豁免，不删除或重写历史 JSON。固定必要 scope 用例不以登记表自证登记表。

| 当前证据 | 结果 | 本地日志 |
| --- | --- | --- |
| 修复前活动守卫 | 新增 3 项失败，原有 3 项通过 | `/tmp/oviraptor-retirement-guard-red.log` |
| 全特性双守卫、PATH/网络哨兵、Native 默认、旧谱系拒绝恢复、新目录与知识导入 | 18 项通过 | `/tmp/oviraptor-retirement-guards-and-neutral.log` |
| 历史/中性设置/导入后端筛选回归 | 主库 163 项通过；同次导入器筛选 13 项通过 | `/tmp/oviraptor-retirement-history-regression.log` |
| 数据库测试及旧 run 封口/幂等/拒绝再激活 | 25 项通过 | `/tmp/oviraptor-retirement-migration-sealing.log` |
| 历史导入 CLI 完整回归 | 30 项通过 | `/tmp/oviraptor-retirement-importer-full.log` |
| 历史导入 UI | 6 项通过 | `/tmp/oviraptor-retirement-history-ui.log` |
| localhost 浏览器集成 | 通过；8 个观察请求、匿名/比较采集 complete、身份隔离和跨源拦截断言通过 | `/tmp/oviraptor-retirement-native-browser.log` |
| 严格 all-targets/all-features Clippy、fmt | 通过 | `/tmp/oviraptor-retirement-clippy.log`、`/tmp/oviraptor-retirement-fmt.log` |

所有已列测试进程已取得退出码 0；红测退出码 101。曾用文件名作测试过滤器得到零匹配（`/tmp/oviraptor-retirement-guard-green.log`），**该次不计验收**；已改用真实测试名称并核对上述非零数量。各集合有重叠，不累加；本轮没有重跑主库全量或前端构建。生产代码与前端未改，上一轮全量/构建证据保留原时间与范围。

可复现命令（Rust 命令在 `src-tauri` 内串行执行；Node 命令在仓库根目录执行）：

```sh
nice -n 15 cargo test --offline -j 1 --all-features --lib -- residual_ retirement_ retired_cli_ the_native_web_target neither_the_native new_scan_work_dirs new_agent_runs_default resuming_a_strix an_unspecified_backend import_sec_skill --test-threads=1
nice -n 15 cargo test --offline -j 1 --all-targets --all-features -- legacy neutral historical settings_migration artifact_import --test-threads=1
nice -n 15 cargo test --offline -j 1 --all-features --lib -- db::tests open_strix_runs sealing_is_idempotent a_sealed_run --test-threads=1
nice -n 15 cargo test --offline -j 1 --all-features --bin import-existing-results -- --test-threads=1
nice -n 15 cargo clippy --offline -j 1 --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
node --test tools/test_historical_json_import.cjs
node tools/test_native_runtime.cjs
```

边界：PATH 哨兵覆盖 Native Web 每目标入口与 Code/Greybox/CI 源码执行分支，不代替完整灰盒 UI 编排；本地代理哨兵不等同于已打包桌面启动流量审计。安装包/跨平台/真实桌面 IPC 和恢复验收、其余专家与完整指令动作矩阵仍未闭环，不能据此将整个 Master Plan 或发布标记完成。未部署，未访问外部目标，未启用主机能力，未降低授权/资源门禁。
