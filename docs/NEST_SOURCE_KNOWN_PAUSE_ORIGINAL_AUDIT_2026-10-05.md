# Source已知失败用户暂停的原Root/业务原子收口审计（2026-10-05）

本批属于REM-A07原执行义务与A01/A02交界。Master未完成，Goal active。本文件记录实际开发证据和限制；脚本模型不是模型脑力验收。

## 问题与真实作用域

已从签名Source发布、实际原SDK和实际暂停API复现：5次请求已知结清、原失败worker已退出，扫描paused但Root仍running（0/1）。现仅对完整原回执/材料/worker/十维与粗账本、同一有效原C及实际退出均可核验的确切耗尽失败，在受限私有事务内同时收口Root和业务暂停；原本地所有者持有到提交，不发SDK、不续租/补授权、不退款、不造成功审核。Mapper/后续Analyst两真实入口分别保留5/7请求及100/140 tokens，Root原终态paused，业务paused，闭合重放全库零写。

五新增具名开发回归覆盖上述两生产路径、12类实际原行损坏/撤权、5类预检后改变原事实、3类实际SDK忙锁和原父inode缺失，以及8类Root/暂停/原退出写入忽略或恶意业务/资产/费用触发器；拒绝时全库typed rows/rowid与临时CAS字节不变，Root/费用时钟/退出与业务暂停共同回滚。首轮扩大新增3/4的失败来自成功路径错误地要求本Root正常日志序列也不变；仅允许原Root日志对应序列推进并保持其他序列后，新增5/5。原失败日志保留，不放宽生产保护。

原生产者是`source_exhausted_pending_root_using`：签名Source发布、实际分支领取、原SDK轮次/工具回执、实际失败worker及预算结清均来自生产入口。SDK原锁在首次Root闭合时真实忙，原Root保持running；用户调用原`request_sentinel_pause`，实际原分支guard Drop后调用真实finalizer。没有手工INSERT失败worker、账本、Root终态或SDK退出。

原始负向：编译50.76秒，exact0/1、2.49秒，Root `("running","")` 与预期 `("terminal","paused")` 不符。日志`/tmp/oviraptor-source-known-pause-red.log`。首个生产修复exact1/1，编译39.74秒/测试6.91秒。五新增回归最后5/5，编译35.53秒/测试59.36秒；首轮新增3/4和其原日志保留，未ignore。

## 最小生产变更

1. 原aggregate quiescence持有实际原Source父调用与scan所有者，只读选择本attempt唯一确切已知耗尽失败Root。预检不发布Root、费用或暂停。
2. 预检事务退出后仍持有OS所有者；独立私有writer取得SQLite写锁，重新验证同一原C/TTL、scan及attempt pausing、原Root/分支dispatch、冻结Source/runtime/CI、唯一原失败worker、前序及三轮已知回执、十维与粗账本。
3. 复用原Root终态/最终时钟/原退出publication；Root仍canonical incomplete/paused，再写原scan/attempt paused；完成后再次核验原材料、费用、C和原退出。任何失败均回滚Root、wall、Exit及业务暂停。
4. 普通`closure_write`的财务列权限保持。仅`source_pause_write`开放scan三列与attempt十一暂停投影列，禁止创建任务/执行grant、改SDK费用、分支结果、业务/资产和删除；保持原可信日志emitter验证及范围检查。

8代码路径4已有/4新，逐文件前像/原差异和最终增量已审查，均小于400行；原Source执行失败writer、原耗尽审计、全部既有生产者/helper/测试字节保持。普通Root闭合writer保留原列权限，只有专用Source暂停入口有限开放scan/attempt暂停投影列，不开放分支结果、执行、SDK费用改写、业务/资产或删除。1351原范围外源码SHA及HEAD59be3d86保持；1359源码集合SHA 1e443ff7097236540304c160cffe98ba69d4fb3a16776d7d8c097eda6276ec82。

## 开发验证

最终同源码关联64/64（测试353.59秒/阶段354.23秒，选择=报告=通过）、严格all-targets/all-features Clippy0（18.15秒）、退役50/50（5.29秒，含literal及当前Native JSON），四叶局部fmt和范围diff检查0。严格离线、locked、all-features、单线程测试；Cargo JSON取得实际本轮binary，再从本轮实际test list精确选择64项。选择=实际报告=通过，源码前后hash相同。关联覆盖新暂停、原Source暂停/未知用量、失败Root初次与无重发恢复、成功/失败Source冷审计、Web/Root/specialist真实SDK退出及共享私有writer权限/故障，未构成当前2342项全量门禁。

- 新增五具名测试；两实际生产路径分别SDK5/7，12损坏/撤权与5事务间变化各实际SDK5；三个实际锁忙/缺原父inode拒绝。
- 八Root/scan/attempt/Exit写入故障及恶意业务/资产/费用触发器全库typed rows/rowid回滚。临时实际CAS非空且文件字节保持，模拟保护业务与asset物理行保持。
- 本Root正常collaboration日志会推动其自己的`sqlite_sequence`。首轮成功路径错误要求该序列完全不变；修正为仅该序列可推进到实际最大sequence，其他表/序列保持。没有修改生产schema保护。
- 原Source远端响应未返回的真实取消回归仍保留原未知费用及非终态Root；本批没有把未知占用当已知退款。

机器证据：

| 证据 | 路径 |
|---|---|
| 源码原前像/集合 | `/tmp/oviraptor-source-known-pause-before.json`、`-baseline.json`、`-code-snapshot.json` |
| 本批逐文件增量与范围 | `/tmp/oviraptor-source-known-pause-reviewed-merge-diffs/`、`-scope-final.json` |
| 当前binary及实际选择 | `/tmp/oviraptor-source-known-pause-final-build.jsonl`、`-final-binary.json`、`-selected.json` |
| 最终64 | `/tmp/oviraptor-source-known-pause-final.log`、`-final-result.json` |
| Clippy | `/tmp/oviraptor-source-known-pause-clippy.log` |
| 退役50 | `/tmp/oviraptor-source-known-pause-retirement.log`、`-retirement-result.json` |
| 本批失败/局部验证 | `/tmp/oviraptor-source-known-pause-red.log`、`-first-green.log`、`-first-negative.log`、`-refined-negative.log` |
| 文档前像与增量 | `/tmp/oviraptor-source-known-pause-docs/` |

## 明确未完成

边界：本批Root收口后Source分支仍pending，其结果消费及暂停删除合同尚未完成；不能当Source整体终态完成。原C过期/替换、撤权、未知用量、缺原事实、其他失败/保护/取消/历史attempt和全通道进程后代退出/清理/重启仍保留义务，恢复待完。真实模型推理/供应商美元、完整框架、UI、安装App及授权URL均未验收。仅临时Git/SQLite/CAS/localhost脚本SDK，无真实DB/CAS/资产/UI写入或自动提交。

下一先用原生产者证明暂停后的Source分支pending如何影响原结果消费/删除，确保仅原事实消费而不重新执行；再收口其有限writer与负向。A02动态十维预算/全费用对账、A03六真实监督、A04–A06全部角色与真实推理/独立审核/并行、A08–A10聊天/日志/整体UI、A11回归债、A12非Web/沙箱、A13数据知识和A14最终安装/URL等完整剩余见[Master顶部](NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md)。真实清理先盘点备份、不删除asset，不绕过InputParser拒绝。
