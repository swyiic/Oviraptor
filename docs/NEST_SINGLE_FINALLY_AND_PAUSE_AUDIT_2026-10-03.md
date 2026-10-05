# Single 原费用 finally、终态发布与实际暂停边界审计（2026-10-03）

本批相关门禁通过。完整预算、暂停后继续、Master与最终功能验收未完成。

## 已证明的问题与数据作用域

真正 NativeAgentBackend.execute 的模型配置早退及旧 attempt 返回漏记最后 elapsed；实际 reporter 八类终态及暂停均漏记；普通 Failed 被纯 reducer 缺省分支判为 Completed，终态 UPDATE 被 IGNORE 丢弃时实际 pipeline 仍计成功。首修后进一步实际证明 reducer Incomplete 被 tally 计完成、暂停写入 cancelled agent_terminal，以及真实 localhost SDK 的 unknown usage 可报 completed、已付 token 分类被 completion 零摘要替代。

只用临时完整应用 SQLite，实际 Native 执行入口、原 RootModelCall 财务接口、实际 pipeline/reporting，以及 localhost 模型/目标监听器。三项执行早退、八类终态注入、故障和 typed late invoice 不代表各角色真实完成；额外 paid 两项是真 SDK；live Native pause 真两次 provider 调用、一次 localhost GET。没有外网目标、真实业务库/CAS/资产、Oviraptor安装、重置或提交。金融夹具明确使用既有 cfg(test) accounting issuer，不是新 Root 创建验收，生产没有 initializer fallback。

## 财务 finally

原 Single Root/control/Native policy/frozen Web plan 在执行前只读捕获，所有 run_native_agent 返回经过同一 finally。不可变 agent_single_exit_receipts 保留原 owner、origin、首次 cutoff、hard、完整 elapsed、原 journaled/unsettled、原身份与物理费用来源。under-hard 仅补原 control 的 wall reserve/consume 差额，无 child 或 C；超 hard 保存完整差额，不 clip、不扩额、不改原失败原因。完成结果耗尽 hard 转 Limited。保存失败返回 PersistenceFailure 并保留原原因，所有金融/业务副作用回滚。

私有连接 IMMEDIATE writer 仅直插原 budget entries/exit receipt；trigger 额外写任何业务/other Root 被拒，IGNORE 精确 readback 与changed复核。三唯一键、UPDATE/DELETE/rowid 与recursive OFF REPLACE保护。原 cutoff 重放不再收费，晚到原账单仍可记录，但 receipt 不是续跑授权。此金融API的实际调用者均使用私有连接；没有宣称任意外部连接/嵌套 authorizer 调用已经验收。

## 原终态与暂停发布

reporter 改为 checked consumer，查唯一原 coordinator，不能通过 open_run 重建已终态 Root；完整函数 preimage 留存，薄 include 接新叶，删除不再使用的弱 latest lookup。Single publication 自己开私有连接，在金融事实已提交后用独立 IMMEDIATE 事务写原 Root、单一 Native event、单一 canonical collaboration event、精确 snapshot、不可变 agent_single_projection_receipts。只许声明列和该已核 exact SQL 的内置 emitter，旧全事件/other Root/other snapshot物理行保持；业务/foreign trigger写拒绝。所有行、scope、cursor、payload、snapshot、金融源及最后schema回验后提交；publication失败保留先前实际金融事实、回滚全部发布行，并传到 pipeline。

Rust typed exit 通过 reducer 保留非成功原因；普通 Failed/Incomplete 不由零pending自动变 completed。完成义务仍由原 reducer 判定；tally和agent_terminal状态/code消费该真实结果。原十维账本未结清时不得completed，original committed model input/cached/output/requests 供 snapshot，不用 completion 摘要替代，不在报告阶段收费。首次终态/暂停 receipt 重放不新写Root/event/snapshot、不多收费；损坏原 event/snapshot/Root 拒绝回放，不修复或收编历史。

暂停只认 exact 原 Single Root、scan、attempt、target 与明确 pausing；实际 SDK received/uncertain 原费用先提交，再返回 yield。晚输出不能产生 semantic model event或tool invocation；暂停发布 status=paused、terminal/finished留空、Native JSON字节不改，pipeline不写cancelled agent_terminal。receipt继续挡住新工作，不能据此说“安全续跑”完成。

## 实际红绿及边界

S1模型配置/失败保存两项0/2，旧attempt单独0/1；正确接线且writer Result类型修正后3/3。四项存储补测全部通过。首次扩大7/16，九项projection/pause在wall=0首断言失败，不能声称那次已进入末尾replay断言。普通Failed与pipeline IGNORE真正0/2；发布首实现22/22，包括5个写面×5个故障的完整物理rollback、canonical emitter、四unique和损坏receipt回放。两个pipeline消费者0/2→24/24。

真实SDK两项首次测试编译因错误unwrap的Debug要求失败，不算功能red；修正后真实0/2（runtime0.65秒）：known invoice snapshot.modelRequests=0而原费用1，unknown usage被completed。改用原账本与settlement检查后26/26（19.00秒）。两个SDK各一次真实localhost请求，不等同完整Native任务/审查完成。

扩大原reporter四项1/4：两个无owner旧摘要夹具及live loop早拒绝。前两改为明确 no-backfill 负向，全应用物理行与当前Native字节保持，已付正向由真实SDK原invoice合同证明；不把旧960/1500摘要补为账单。live fixture迁到显式原财务setup后3/4，实际暂停仍返回execution_authorization_denied，这是生产缺口；修复exact pausing mapping后4/4（1.68秒）。真实loop有两次SDK成本（known consumed或明确indeterminate合计2）、一次GET、仅一条语义model event；旧“第二输出产生拒绝tool”的期望移除，因为晚输出不获使用权。该合同接真实checked publisher，最终Root paused、snapshot无terminal、Native原字节保持。

S1原patch检查因当前include漂移拒绝；一次shell未set-e而继续fmt/Cargo，误名s1-green实际0/3，不计通过。此后脚本遇错停止。正确S1先有E0282/E0283，S2先有Native backend String/enum比较错误；都不是行为red。一次类型补丁SHA误填被断言拒绝、未写仓库，核实后应用。初次scope信息打印Python转义失败，仅展示命令失败；seal/HEAD/范围外/diff检查先已成功，随后正确读全12已有文件增量。

## 最后门禁与保护

最终 alltargets/allfeatures严格Clippy退出0；相关115/115（runtime92.67秒），导入39/39，exact退役allowlist1/1。精确wall/log见/tmp/oviraptor-single-finally-final-gates.json。之前pre-paid四门禁另存/tmp/oviraptor-single-finally-pre-paid-gates.json，109通过属于该旧快照，不与115相加。全部Cargo串行nice15/offline/locked/-j1/testthreads1。未跑全Rust/安装/授权URL，不作整体功能验收。

32代码路径12已有/20新；18新Rust叶fmt/check0，最大248行，不格式化旧大文件。1084收集路径SHA6c9e53c320c6fcb08a981c14a031e4a68575c57ff1425f8c6d6ba47b8d3cf286；1052范围外原路径不变，HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b和diff check通过。collection同前批，不含docs/build.rs/icons/capabilities/dist/安装包全仓保证。原before/dirty差异/32文件增量/最终snapshot在/tmp/oviraptor-single-finally-*；原reporter函数SHAf59d34965c4eafa21fcbaf7c634cf6516b4c6133786e7e0f2e6a3e56f3caa3c4，函数搬移及类型/纯读和publisher接线逐hunk，不覆盖已有改动。

## 未完成与下一批

仍需production reporter在读取active attempt之前携带不可变原执行身份，legacy target/agent_terminal与Root发布的同事务或明确失败回执、scan_quiescence的完整旧金融范围证明；当前外层pipeline有attempt fence、publisher也复核，但不把它们当完整race证明。显式原Single暂停后continuation、人工对账/动态预算、unsettled UI、Source全outcome finally仍缺。SDK阶段日志与Root后置消费者、Web helper日志下一批；完整local ReAct/全部触发/全角色/Broker/实际并发/有序人工执行/用户聊天恢复继续，最后全量、当前安装态、两个授权URL。InputParser先前自动审批拒绝仍未交付、不绕过。Goal active，不标Master完成。
