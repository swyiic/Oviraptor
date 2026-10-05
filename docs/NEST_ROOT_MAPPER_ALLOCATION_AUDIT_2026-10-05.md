# Root Mapper 原余额分配开发审计（2026-10-05）

Master与Goal均未完成。本批仅新建Web Multi Mapper的模型额度有限动态分配与原合同重放；完整十四类剩余见最新版Master顶部。

## 问题与数据作用域

- 原22,000 tokens合同实际Root付费20后余额21,980；固定Mapper8,000使余13,980，挤占原Reviewer15,000。负向1/1实际失败。
- 新建版本2冻结 `current_balance_after_reviewer_floor`；同一IMMEDIATE事务读取原Gross总余额，留Reviewer15,000tokens/1request，Mapper最多8,000tokens/1request。不生成目标权限。
- 同一paidRoot原回执/事件/summary与basis/C/原监督者核验在发放前后进行；原worker/lane/capability有效，子任务、预留、发放和start同事务提交。
- 原版本1当前Native字段/JSON和固定分配分支保持；未声明Bootstrap原合同也保持。没有Strix正向兼容或旧Root自动升级。
- 本批临时Git/SQLite/CAS和localhost脚本SDK，无真实DB/CAS/asset、UI/安装/URL操作或提交。旧未提交改动逐文件保护。

## 实际执行与有限恢复

- 新22,000场景实际Root/Mapper SDK各一次；Mapper6,980，发放后审核余额15,000，结算后总余额21,960、剩余requests18，仅实际40tokens/2requests被消费。原终态释放生产者未改。
- 完成后原预留投影列已归零，第一版新重放错误拒绝，日志 `oviraptor-root-mapper-allocation-settled-replay-red.log`。现用第一worker的四维不可变原预留凭证/键/source固定原额度，并核验原活/已结算投影；重放不重新分配、不扩额、不重发。
- 原readonly查询在撤权后仍返回活Mapper，负向日志 `oviraptor-root-mapper-allocation-revoked-replay-red.log`；新分配路径复用原调度准入，撤权/过期/缺lane等拒绝，无续租/补权限。
- 两轮Root（预算工具后最终选择）实际付费40再分配8,000，余额51,960/17requests，不把预dispatch观察当当前账本。
- 两线程同一paidRoot并发准入只产生一个原worker与一组四维model原预留；这是发放竞争，不是两个角色SDK实际重叠。
- 实际Mapper503保留原未知request债务1，拒绝再分配/重发，无退款；完整未知费用人工确认/恢复仍待其它Master任务。

## 负向与验证结果

- 七新入口测试与一合同测试：两不足场景、八损坏/失效、三原写入故障；全库typed rows/rowid精确保持或回滚，原paidRoot保留。
- 八损坏/失效为预留tokens/requests、任务floor、原账本不可变guard、撤权能力、过期worker、缺lane、Root paused。
- 三故障为缓存token凭证插入忽略、发放触发器试图写业务项目、start投影忽略；不扩大原writer权限。
- 初次扩大4/6，除撤权缺口，23,000两轮Root夹具被原Reviewer保护在第二SDK前拒绝，换新建60,000夹具。私有路径/闭包错误类型与接线失败修正，不计为通过。
- 最终关联57/57，选择=报告=通过，测试106.63秒/阶段122.98秒。同源码覆盖原Root/预算/Native合同、Identity/Client回传、原Mapper重放、worker replacement和实际Source paid恢复；不是全量。
- 严格all-features/all-targets Clippy exit0，Cargo报告21.29秒。五叶rustfmt与范围diff检查exit0。
- 同二进制退役50/50，集合一致，5.07秒，包含literal与当前Native JSON；无ignore。当前2364项Rust全量未做。
- 没有UI、安装IPC、App打开、授权URL或真实模型质量验收。本批没有前端改动，所以没有新UI构建或视觉验收证据。

## 修改边界与原像

- 9代码路径5已有/4新，全部增量已审查；共享调度器原Readonly角色/Source/Client限制、callback重放回滚与旧Mapper固定分支保持。原Native SDK、费用/释放生产者、原writer权限和全部已有测试未改；测试清单仅加两行。
- 1365原范围外代码SHA保持，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 保持。
- 1374源码集合SHA `c2b78b2c637939ada55f448f1bde7aa3caa54cddb438a6f4acef2e40ac4f199a`。
- 临时审计在 `/tmp/oviraptor-root-mapper-allocation-{before,baseline,scope-final,code-snapshot,final-result,retirement-result}.json` 与 `-reviewed-merge-diffs/`；关键red/green、Clippy、原二进制列表和选集分别保存。
- Master/progress/交接原完整字节备份 `/tmp/oviraptor-root-mapper-allocation-docs/*.before`，最终仅新增本轮前缀，旧全文后缀精确保持。

## 尚未完成

脚本SDK验证生产路径，不证明真实供应商推理/美元。22,000场景只证明Root/Mapper与审核余额，未证明低额度下后续全任务可完成；Reviewer仍需独立准入。其他角色/十维动态分配、六类Root监督、15角色真正推理/受限工具及实际并行、聊天/逐路日志/整体UI、其余删除/恢复与数据/非Web/回归债、框架后全量门禁/安装App/授权URL全部保持Master未完成。不得把本批57/50局部集合相加或拼接历史通过数当功能验收。
