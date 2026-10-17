# Source 原 Root 已关闭后用户暂停的收口审计（2026-10-05）

Master 未完成，Goal active。本文件记录本批开发证据和限制；完整剩余范围在 Master 顶部，局部测试不算整体验收。

本批实际 SDK/Source 恢复入口先关闭 Root、随后用户请求暂停，仍得到 paused:pending（0/1；编译42.59秒/测试2.77秒）。这不是仅旧数据库标签问题，而是当前生产顺序会再次产生。现选择同一原已知耗尽失败的 pending 分支，在私有暂停事务重新读取 Root 的实际阶段，复用既有终态纯重放校验原截止、费用及退出后消费 partial；不重关 Root、不创建费用/退出/日志，不重发 SDK。已消费的终态原结果不再选为新的暂停写入。

实际 Mapper/后续 Analyst 分别 SDK5/7，Root 先关闭后暂停，除扫描/attempt/分支/目标四投影表外全部 typed rows/rowid 精确保持；原 Root、时钟、退出、费用、日志及临时 CAS 保留，paid 删除、重开库冷审计及重放成立。四新增具名回归另含12类实际原事实损坏、8类投影静默忽略/恶意 Root费用业务资产写入、已消费结果的纯选择零写拒绝。最后一项只证明选择器，不当作 Greybox 混合分支或整体验收。

阶段证据：初次生产修复1/1（编译22.84秒/测试11.98秒）；扩大首轮5/7中两个新夹具错误地把终态后重新占用的 SDK probe 当作仍活 SDK，改为持有原 Source 父调用 inode，不改变生产退出判定。另实际已消费原结果被再次选择0/1，增加终态仅选择 pending 限制。最终同源码关联35/35（测试198.46秒/阶段218.24秒，选择=报告=通过）、严格 all-features/all-targets Clippy0（18.33秒）、退役50/50（5.45秒，含literal/当前Native JSON）、三叶局部fmt/范围diff0。没有 ignore；失败日志保留，仍不是当前2350项全量门禁。

4代码路径2已有/2新逐文件前像、原差异和最终增量审查；生产仅改原暂停选择器/阶段重读，原终态 writer、冻结材料 proof、私有权限、费用/退出生产者、全部旧测试原字节保持。1360原范围外源码和HEAD59be3d86保持，1364源码集合SHA c90c23f9a3c3f1461c22458f3531321a2c8465b1de3ca059a2ed8bba2337ede6。仅临时 Git/SQLite/CAS/localhost 脚本 SDK，无真实 DB/CAS/资产/UI 改动、安装/URL测试或自动提交；脚本 SDK 不证明真实模型推理或供应商美元。

仅修复仍 pausing 的真实顺序；已 paused+pending 不自动变更。原 C 过期/替换、撤权、未知费用、缺事实及其他失败/取消/保护/历史 attempt/角色仍保留义务，恢复未完成；框架、整体 UI、安装 App 打开与授权 URL/真实模型质量未验收。

## 问题、生产入口与范围

原生产者 `source_exhausted_pending_root_using` 经签名发布和实际 Source SDK/工具产生已知失败 worker 与已付费用。原 SDK 解锁后调用真实 `run_native_source_assessments`，由生产终态 writer 关闭 Root；原分支尚待结果消费时调用真实 `request_sentinel_pause`，随后实际 branch guard Drop。原始错误为 paused:pending，paid Root 未被重新制造。

`known_source_pause_root_in` 现在只对原已知耗尽失败选择尚未关闭 Root 或已终态且 Source 分支 pending 的 Root；在预检和私有写事务分别重读实际阶段，保持原 C/TTL、scope/runtime/CI/材料/唯一失败 worker/回执/两账本 proof。既有 `finish_coordinator_run_in_transaction` 的终态路径纯校验原截止、原事件、原费用和原 Exit；不改该 writer、不放宽私有列权限，不补缺回执或延租。

成功测试在实际 Root 关闭、暂停请求之前捕获所有表 typed rows/rowid；最终只有扫描/attempt/分支/目标四投影表允许变化，后两表还逐单元核验有限列/原 owned 行。所有其他表（含 Root、事件、时钟、退出、两账本、费用、worker、SDK日志和序列、材料/修订/资产）保持原物理行。临时 CAS 非空且全部文件字节保持。

12损坏分别为原C过期/替换、失败worker类别、已付粗账本、Root原reason/cutoff、原Exit缺失、冲突成功审核报告、源码路径、未来目标attempt、未领取dispatch、原runtime不符。缺原Exit测试仅临时库移除原行后恢复原不可变trigger DDL，不能以缺schema拒绝代替缺原事实拒绝。8事务故障含分支/目标静默忽略、scan ABORT及后置改Root cutoff/费用/资产/业务/报告；全部事务拒绝并全库typed rows/rowid/CAS保持。成功和拒绝均不发新 SDK。

已消费结果选择器回归使用实际 SDK5 和完整原已消费结果，纯事务检查不再被选中、全库零写；该单元回归不等于真实 Greybox 多分支执行验收。

## 原始失败与验证文件

| 证据 | 文件/结果 |
|---|---|
| 真实顺序负向 | `/tmp/oviraptor-source-pause-closed-root-red.log`，0/1，paused:pending。 |
| 首个生产修复 | 同前缀`-first-green.log`，1/1。 |
| 七项初次扩大 | 同前缀`-expanded.log`，5/7；两夹具使用终态后SDK probe阻挡是假设失准，改原父调用inode，不改生产判定。 |
| 已消费结果重复选择 | 同前缀`-consumed-red.log`，0/1，source_pause_original_state_conflict；终态pending选择条件修复。 |
| 最终35 | 同前缀`-final-result.json`/`-final.log`/`-selected.json`；实际Cargo JSON binary来自`-final-build.jsonl`/`-final-binary.json`，实际测试清单`-final-test-list.log`；集合一致，源码前后同hash。 |
| 严格Clippy/退役50 | 同前缀`-clippy.log`、`-retirement.log`/`-retirement-result.json`，均同最终源码。 |
| 原前像/既有差异/最终增量 | 同前缀`-before.json`/`-baseline.json`/`-prior-diffs/`/`-reviewed-merge-diffs/`/`-scope-final.json`/`-code-snapshot.json`。 |
| 文档保护 | `/tmp/oviraptor-source-pause-closed-root-docs/`，原后缀/hash保持、逐文件增量。 |

最初应用测试补丁时手录校验SHA失准，保护工具在写入之前拒绝；按实际生成SHA复核后应用，未改任何既有文件。该保护错误不算业务负向。

## 未完成与后续

下一先以原生产入口盘点已 paused+pending 的原结果及证据，完成显式本地恢复入口，严禁用“继续扫描”重发来代替原结果消费；然后推进十维预算/对账、六监督、全15角色/真并行、聊天/逐路日志/整体UI，结合回归债/非Web/数据知识；框架后完整门禁、同源码安装App实际打开、两授权URL匿名只读与真实模型质量。真实Nest/业务/CAS读写授权保持，清理先精确盘点备份、不删除asset；保护未提交改动，不自动提交。 已 paused+pending 的数据仍未被本批自动修复；其它原义务和最新十四类详见[Master顶部](NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md)。
