# 已知耗尽失败 Source 删除与冷审计（2026-10-05）

Master框架未完成，Goal active。本文只结算本批REM-A01开发切口，局部回归不是整体功能验收。

本批先用实际Source入口/原SDK退出复现已知耗尽、费用结清且Root paused/分支partial仍被删除审计拒绝（0/1）。现新增独立只读失败审计，核验确切原终态、唯一失败worker、完整前序/三轮模型与工具回执、十维及粗账本、原材料/分支报告、原退出和实际SDK闲置；CI与原runtime冻结政策精确比对。完成态审计原函数保持，不发SDK、不补Root/C/权限/费用，不伪造成功或独立审核。

三新增具名回归覆盖Mapper/后续Analyst实际失败生产者（SDK总5/7），删除、重开库和重放保留原财务/worker/日志/快照/接受修订/退出及CAS字节；15类原事实损坏、三类原执行锁忙、两类冷材料损坏、受保护业务FK及五类写入故障拒绝或全库typed rows/rowid回滚。另实证CI合法阈值错配被接受（0/1），补原政策比对后纳入最终回归。首次测试类型错误、空晚到费用表导致注入未改行、归档不可变触发器阻止测试注入及扩大首轮62/63全部保留；修正夹具并要求损坏实际原行，未放宽生产保护。

最终同源码关联63/63（测试461.04秒/阶段483.69秒，选择=报告=通过）、严格all-targets/all-features Clippy0（23.54秒）、退役50/50（5.63秒，含literal与当前Native JSON）、四叶局部fmt及范围diff检查0。6代码路径3已有/3新，原完成态审计正文及既有费用断言保持；launcher helper仅迁最后删除预期，普通完成proof仍拒绝，其他原测试字节保持；1349原范围外源码SHA及HEAD59be3d86保持，1355源码集合SHA f8df0b8ade372d97aa546ce5c0095affa5288440c0dc9d83bd1fa2ee115af279。仅临时Git/SQLite/CAS/localhost脚本SDK，无真实DB/CAS/资产写入、UI改动、安装/URL或真实模型质量验收、无提交。

## 作用域与原证明

先盘点实际临时生产者：Source scope/runtime/view/results/CI政策各1，snapshot1、接受revision1；分析器run/container为0，工具轮次/回执为Mapper3或后续Analyst5，原Multi退出1。真实源码分析器及真实模型质量未由此证明。常规已知回执在原SDK表及费用账本，晚到费用fact表本夹具为空，未制造其原记录。

删除复用已有有限Source材料归档清单，不扩表、不删除资产或CAS。原十维/粗账本、物理rowid、worker、模型回执/工具/邮箱、原C及退出保留或无损归档；所有保留财务/日志/材料表（含本夹具空表）及资产逐表比较，全部CAS文件逐字节比较。冷审计仅在私有query_only内存库恢复原列值/rowid，不恢复归档SQL，不产生可执行RootOwner或授权；原活Root加载失败。原父SDK/round SDK/specialist SDK实inode锁继续持有到删除COMMIT，拒绝在途或缺原退出。

既有实际launcher SDK7已知费用/failed子任务/无lane权限断言保留，删除准备改为独立失败proof只读接受；普通Source completion proof仍必须拒绝，Root/分支保持paused/partial。本批不重分类未完成业务，不制造CI通过或独立Reviewer。

## 验证与保护

- 原拒绝：编译50.07秒，0/1测试2.81秒；首次独立修复1/1测试10.97秒（两实际场景）。
- 新负向首次编译类型错误（连接/事务）；修正后2/3测试23.09秒，其中空表故障未改行。改为实际原行并要求changes>0。
- 扩大首轮63项：62通过/1失败，测试424.21秒/阶段439.61秒；归档不可变触发器正确阻止测试注入，临时savepoint内显式故障后回滚schema，不放宽真实schema。
- CI错配负向：实际合法max_high+1被接受，0/1，编译19.16秒/测试14.68秒；生产仅补与原runtime.operatorPolicy精确比较。
- 最终63/63，测试461.04秒/阶段483.69秒，精确选择集合与实际报告/通过一致，无ignore；最终retirement50/50 5.63秒；严格Clippy见最终日志；四叶fmt和范围diff检查0。
- 源码前像/已有git差异逐文件保留，最终六增量全文审阅；既有completed审计正文（仅外加paused分派/include）、旧费用/fault测试及helper前261行原字节一致；其余1349原源码不变、HEAD不变、Native JSON/allowlist/UI不变。工作树无重置、批量覆盖或自动提交。

本批日志及SHA证据：

- `/tmp/oviraptor-source-failed-deletion-red.log`
- `/tmp/oviraptor-source-failed-deletion-green.log`
- `/tmp/oviraptor-source-failed-deletion-negative-run.log`
- `/tmp/oviraptor-source-failed-deletion-negative-fixed.log`
- `/tmp/oviraptor-source-failed-deletion-ci-red.log`
- `/tmp/oviraptor-source-failed-deletion-first-expanded-final.log`
- `/tmp/oviraptor-source-failed-deletion-first-expanded-final-result.json`
- `/tmp/oviraptor-source-failed-deletion-final.log`
- `/tmp/oviraptor-source-failed-deletion-final-result.json`
- `/tmp/oviraptor-source-failed-deletion-clippy.log`
- `/tmp/oviraptor-source-failed-deletion-retirement.log`
- `/tmp/oviraptor-source-failed-deletion-retirement-result.json`
- `/tmp/oviraptor-source-failed-deletion-baseline.json`
- `/tmp/oviraptor-source-failed-deletion-before.json`
- `/tmp/oviraptor-source-failed-deletion-scope-final.json`
- `/tmp/oviraptor-source-failed-deletion-code-snapshot.json`

## 剩余边界

只接受原已知三轮均未finish的确切耗尽失败；其他失败/保护/取消、原C失效、未知或缺事实、异常elapsed、历史attempt/其他角色仍需独立原义务合同，不能套用本批放行。冷审计依赖保留的冻结view文件，缺文件拒绝，不补运行凭据。真实规模/未来schema未验收。当前SDK为localhost脚本、分析器为受控回调，未证明真实模型推理、供应商美元或Docker/Windows执行。

完整十四类剩余已更新[Master顶部](NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md)。继续A01其余删除与残余活路径，再A02–A07、A08–A10，结合A11–A13，最后A14；UI/聊天/日志、安装App打开、授权URL及真实模型质量仍未完成。
