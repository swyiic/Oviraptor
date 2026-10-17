# 缺失对照身份原入口与停止码投影审计（2026-10-04）

Master仍未完成，Goal active。本批收口一项原回归及两处已证明的生产问题；localhost脚本provider走实际SDK，不能视作真实模型质量或整体验收。

## 问题、负向证据与修改

原missing-side夹具缺普通creator/startup/Root授权；旧断言允许只发身份A的HTTP、再让模型解释缺失B，违背完整身份绑定。迁移至实际新任务creator、捕获会话绑定、startup/HMAC、冻结Single原Root、run_agent_target、owned消费。A/B先合法绑定，仅在临时作用域删除B制造负向，不补造权限或正向Root。原测试名保留，无ignore。

正式生产修改前负向2/2失败，日志/tmp/oviraptor-missing-side-original-canonical-negative.log：①实际SDK返回后B缺失，执行正确拒绝且零HTTP，原Single回执保留授权停止；传入Completed时top code仍正确，但nested stop.code为finish_target，原checkpoint被改写；②SDK前B缺失，冻结计划正确拒绝却已把queued目标写成scanning，全原typed row/rowid比较失败。编译18.37秒，测试1.41秒。

最小生产修改：agent_backend仅把scanning路由写入移到context.plan_rejection检查后；agent_terminal_projection仅补execution_authorization_denied、request_reconciliation_required两项canonical停止码映射。没有授予会话、填补Root归属、改预算、重发请求或新增旧格式兼容。

修改后两项2/2（编译18.31秒，测试1.54秒），原政策和A原材料字节保持、B未恢复、模型无cookie/bearer；SDK后模型调用1/目标HTTP0，已付SDK用量留在原Root、十维reserved/indeterminate清零且原Single退出有效，不造覆盖/漏洞/Native可续checkpoint。owned消费paused且tally计1；owned重复和传入Completed均按原回执确认，tally不重计、全部应用typed row/rowid/费用/调用不变。返回true是原canonical回执重放的确认，不代表接受传入完成覆盖。

SDK前负向仍按真实生产分类Failed/persistence_failure、detail agent_attempt_plan_frozen_conflict；原owned Root尚未获取为None，不能推断或补入已有Root。零SDK/HTTP、十维全部零、没有伪造退出或投影；全部原typed row/rowid保持。未将这一早期拒绝宣称为用户聊天/恢复闭环；错误分类、用户可见终态仍归剩余框架核验。SRC/OAST临时文件准备仍在拒绝前，本批只证明数据库/网络/费用作用域，不宣称文件系统完全无副作用。

新增真正fresh Single SDK收到响应但无usage的负向，1/1（编译17.88秒，测试1.19秒）：原model_requests消费1、三token维度indeterminate>0、HTTP0，无假覆盖/漏洞/可续checkpoint；原退出证明存在，owned发布paused。重复owned及伪完成报告均保持整个checkpoint、所有原typed row/rowid/未知债务/调用，不退款、不继续SDK。传入报告不能清除待对账状态。

## 保护与验证

5路径3已有/2新逐文件原bytes/Gitdiff备份；全部五最终增量全文审查。identity_e2e574→491，仅旧missing-side正文替换include，其他字节保持；新叶347及114行。既有backend1037行及identity491行结构债没有批量格式化/重构，保留后续拆分任务。1322原范围外SHA保持，1327源码集合SHA 4a35ed89f8be36f21c6943940cb2be9b6e91ecb80da9dd8647eb45a140376356，HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b保持。未修改退役allowlist、Native JSON或生产权限规则，未reset/自动提交；仅临时SQLite/localhost/合成身份，无真实DB/CAS/资产写入、App安装或授权URL访问。

| 阶段 | 结果 | 日志 |
|---|---|---|
| related | exit 0；60.42秒；47过/0失败 | /tmp/oviraptor-missing-side-original-related.log |
| original14 | exit 101；41.5秒；12过/2失败 | /tmp/oviraptor-missing-side-original-original14.log |
| clippy | exit 0；15.5秒；0过/0失败 | /tmp/oviraptor-missing-side-original-clippy.log |
| retirement | exit 0；5.27秒；46过/0失败 | /tmp/oviraptor-missing-side-original-retirement.log |

related具名选择=reported=passed47，含新三项、八类实际会话撤权/材料失效、原Single退出/投影故障回滚及重放、原Multi发布和Native policy；original14选择=reported14，12过/2失败，没有跳过。严格all-targets Clippy/-D warnings通过，退役46含exact及当前Native JSON原字节回环；两个新Rust叶局部rustfmt、diff--check通过。本批初次迁移仍有错误期望（early Root未获取、canonical重放返回true）及完整行变化，日志first/second/proof均保留，不计通过。其后明确红测两处生产问题再实施修复。

剩余原失败：

- commands::agent_tests::bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure
- commands::agent_tests::every_target_ends_in_exactly_one_terminal_state

6项Source Broker已知基线失败仍待迁真实原始Source子任务，本批未重跑或修复；扩大325曾中断91过/6失败/228无结果，保留为历史，不能推断全绿或全项目只剩2+6项。Master14项依然覆盖删除/残余活路径、动态十维预算/原对账/恢复、Root六类真实监督、全15角色/真正并行、证据与独立审核、全通道退出恢复、用户聊天、逐路实时日志、整体UI、非Web及真实数据规模。完整门禁、同源安装App实际打开、两授权URL匿名只读及真实模型质量最后。InputParser原拒绝不绕过。

下一步以实际生产者核验三种bootstrap fresh-attempt拒绝、七终态原Root/owned入库及Source Broker原任务权限；保留所有费用/负向，不用裸回调或伪造正向夹具销项。
