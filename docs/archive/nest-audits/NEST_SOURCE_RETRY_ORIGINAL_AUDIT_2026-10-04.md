# Source新attempt原归属与真实重试核验（2026-10-04）

本批将最后一项旧无原子任务授权的Source结果context迁为两轮真实执行：实际source分支领取先于分析器/SDK，原Source Root与独立Mapper/Analyst/Reviewer/coverage执行，第一轮原结果经真实branch消费后进入只读retry basis及真实签名重试发布；不手工INSERT第二轮、scope或CI权限。两轮canonical key/revision相同，analysisResultsDigest及Root/原SDK回执独立；实际脚本SDK分别8/9次、总17，原预算和SDK/结果材料typed rows/rowid保留，两原退出证明均有效。第二轮先读本轮结果，再实际导入迟到历史，历史key result_not_found、原详情完整JSON不变；旧attempt source_runtime_attempt_inactive，旧成功/失败回调在新轮scanning及闭合后均零typed rows/rowid变化、零追加SDK。两轮分支/attempt均真实消费为completed_with_gaps，不造全覆盖。

首次exact1/1（编译48.48秒/测试42.81秒）；随后收紧旧轮精确停止码并补两轮attempt状态/闭合后迟到失败负向，最终同源码关联26/26（249.66秒测试/281.58秒阶段，选择=报告=通过集合），严格all-targets Clippy0（10.16秒阶段）、退役46/46（5.29秒，含exact及当前Native JSON原字节回环）、两执行叶局部fmt/diff--check0。关联包含scope9、Source结果全15及工作台重试发布/准备basis变更拒绝2。前轮14/1仅保留为历史，不再作为当前结果。

3代码路径2已有/1新逐文件保护、三最终增量全文审查；其他Source结果函数/helper原字节与全部15结果测试名保持，文件均小于400行。1336原范围外源码SHA及HEAD59be3d86不变，1339源码集合SHA c0d29b70dc40e9d93b66ed7f0c96cf2fde52c074b007059836422032a84e4616。生产准入/预算/Native JSON/allowlist零修改。只用临时Git/SQLite/CAS/localhost、分析器回调与脚本SDK，未写真实DB/CAS/资产、不自动提交、未安装/外测或修改UI；脚本用量不是实际供应商美元对账，也不能证明真实模型脑力。

## 保留的结果与权限合同

1. 同一分析器回调为两个attempt产生相同SARIF；原导入canonical去重保持，key/revision相同，但冻结结果摘要属于各自attempt，结果回执总数2。
2. 第一轮source branch领取先于副作用，实际独立角色/SDK/审核/原Root退出后，合并原报告经finish_native_branch消费。释放实际branch owner，再读取retry input/basis和真实工作台发布，第一轮task.json原字节保留。
3. 第二轮领取自己的branch，签名Source runtime、Source/CI scope、冻结材料与新生原Root正常生产。两轮Root不同，工具回执按对应Root与child归属读取；第一次详情attemptNumber=2，digest为第二轮冻结值。
4. 第二轮真实SourceAnalyst读取后才导入迟到历史；导入不能修改冻结receipt JSON；历史key拒绝为result_not_found，同一个原详情JSON前后完全一致，list仍只有本轮候选。
5. 重试发布后旧执行精确source_runtime_attempt_inactive，不追加SDK，所有数据库表typed rows/rowid保持。旧轮成功回调在新轮scanning时拒绝、旧轮失败回调在新轮完成后拒绝，两个边界全typed rows/rowid不变。
6. 两轮7个已核算请求/token/工具维度分别核对，消费/预留/未决为各自已知数值且无串账；预算/SDK/原model round/结果/导入revision表全部既有typed rows和rowid保留，两个原FinalClock退出证明有效。新轮闭合后同入口重复执行拒绝，无追加SDK或任意行变化。美元定价及完整十维动态分配仍属Master剩余。
7. 两轮分支和attempt分别真实completed_with_gaps；不能把不足覆盖、测试provider或原15单元全部通过作为功能完成。

## 最小修改与证据

- tests_source_broker_scope.rs：仅移出这一项旧手工插入attempt和无原子任务授权context的测试体，其他函数/helper原字节保持；所有15个结果测试名保留。
- tests_source_broker_original_fixture.rs：保留既有接口默认生命周期；可选continuation才实际领取/消费首分支、释放OS owner，再在同一活测试provider下继续下一轮。Mapper根据当前实际消息有无工具回执选择inventory/finish，支持独立第二轮；不生成授权/费用/结果证据。
- tests_source_result_original_retry.rs：两轮真实重试发布及工具回执、canonical去重、独立费用/退出和迟到拒绝。

保护证据统一前缀/tmp/oviraptor-source-retry-original-：baseline.json、before.json、prior-diffs/、initial.patch、tighten.patch、reviewed-merge-diffs/、scope-final.json、code-snapshot.json、semantic-review.json。1336范围外原文件及HEAD保持。首次exact日志first.log；收紧后最终验证related.log、clippy.log、retirement.log，具名选择selected.json、结果checks.json；没有拼接前批通过数，没有ignore。所有Cargo离线/锁定/单作业串行，当前对应句柄均终态。

## 未完成与下一步

继续三项Source scope历史拒绝夹具、Source分支异常结果消费与原Root/费用/退出绑定、其余删除及残余活路径；再依完整14类剩余推进十维动态预算/对账/恢复、Root六类监督、全15角色与真正并行、聊天/逐路实时日志/整体UI、非Web及知识资产生命周期。框架收口后才完整门禁、同源安装App打开、两授权URL匿名只读及真实模型质量；登录凭据待用户提供。真实Nest写入/CAS已授权，真实清理先精确盘点备份，不删除asset。InputParser原拒绝不绕过。Master仍未完成，Goal active。

Source生产launcher仍需实际错误/未完成结果的消费一致性核验，尤其不能只根据报告gaps数量推定Root已完成；本批只验证真实成功消费与旧轮回调拒绝，未完成所有Root状态的消费/删除合同。另三scope历史拒绝夹具仍是旧facade单元负向。完整Source、动态预算、六监督/全角色/实际并行、真实模型质量、聊天/日志/UI、非Web、真实数据生命周期和安装/URL验收均未销项。
