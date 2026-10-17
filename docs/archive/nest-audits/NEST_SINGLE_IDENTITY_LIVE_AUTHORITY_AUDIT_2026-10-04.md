# Single实时身份授权及双身份原结果链审计（2026-10-04）

Master继续，Goal active；这是REM-A01/REM-A11局部修复，不是框架或安装验收。本批使用实际localhost SDK传输、HTTP和SQLite；provider返回脚本响应，登录材料为合成fixture，不代表真实LLM推理、真实登录采集或外部计费质量。

## 问题、作用域与最小修改

实际ordinary Single新creator将同一UUID草稿的两个会话原子绑定到任务，经startup/HMAC、原mode/Root预算、冻结计划、run_agent_target及captured owned消费。仅test-only任务ID生成器覆盖为agent-scan以保留断言，未UPDATE补造Root/mode/正向policy。模型首轮SDK到达后，仅撤销这一个临时任务的authSessionIds/authSessionId，保留原会话及owner；模型返回replay_http。修复前目标实际收到1次HTTP；两正向通过、撤权新增项失败，3项2过/1失败（11.07秒）。日志/tmp/oviraptor-single-identity-withdrawal-red.log。此为真实生产路径缺口，不是仅旧夹具失败。

生产agent_authorize_tool_on的Single分支原来仅查活attempt/coordinator即返回，跳过Multi已有的validated_scan_identities与全句柄集合校验。被选会话的owner仍有效，所以policy撤回不能阻止凭据使用。现在仅抽取原完整校验为agent_require_bound_identities：Single在原coordinator准入之后调用，Multi仍在原capability/lane/fencing之后调用。Source、生产无Root拒绝、冻结计划及活attempt保持；无新schema、权限、续租、账单重签或旧格式正向兼容。

原Root transport已有发送前、在途与响应后的权限核验。接入Single后，收到的模型用量先保存，再因身份失效拒绝发布响应，工具invocation和HTTP均0、后续SDK0。五种失效分别为policy撤销、另一未选身份过期、跨任务owner、主机变化、两会话凭据相同；各实际SDK恰好1，原模型输入/输出账单为正、target费用0、reserved/indeterminate为0、原Single退出可只读核验。owned消费目标为paused、计数1，无漏洞、无该轮checkpoint；不补造可续跑状态。活Root时精确身份拒绝，已关闭Root再准入因coordinator关闭拒绝。

两个旧完整身份E2E迁原入口：同权与个性化权限差异均两次实际GET /api/profile；逐个核验Cookie/Bearer配对，不混另一身份，所有模型请求不含合成凭据。原覆盖、未验证对象归属不得确认IDOR、零漏洞、checkpoint/原账单/目标/owned断言保留。cfg(test) startup按既有canonical启动写auth-sessions.json再签HMAC；首次缺此文件0/2，补后2/2（7.74秒），未跳过签名或修改生产启动合同。

新增测试在首fix后错误要求checkpoint，随后误要求已拒绝响应产生tool行，最后误在Root关闭后期待身份优先拒绝。全文核验Root/executor/consumer后改为实际合同，失败日志均保留，这些断言错误不算新增生产漏洞。五项最终5/5（3.94秒）。首次Clippy仅报新helper冗余闭包，改关联函数引用后通过，未添加allow。

## 当前验证

| 检查 | 实际结果 | 日志 |
|---|---|---|
| 生产撤权红 | 两正向通过/撤权失败，目标收到1请求 | /tmp/oviraptor-single-identity-withdrawal-red.log |
| 五失效负向 | 5/5，3.94秒 | /tmp/oviraptor-single-identity-revocation-final-r3.log |
| 最终相关逐名 | 116/116，测试143.45秒、含编译总163.92秒，选择=通过集合 | /tmp/oviraptor-single-identity-affected-final.log |
| 原14独立复核 | 7过/7失败，阶段27.75秒，无ignore | /tmp/oviraptor-single-identity-original-debt-final.log |
| 最终all-targets/all-features严格Clippy | exit0，8.12秒 | /tmp/oviraptor-single-identity-final-clippy-r2.log |
| 最终退役相关 | 46/46，阶段5.63秒，含exact、当前Native JSON原字节roundtrip及包装/上下文负向 | /tmp/oviraptor-single-identity-retirement-final.log |

首次116/116（164.81秒）在Clippy等价闭包整理前，最终116在当前快照重跑；不相加。2303个lib测试已编译，未全跑；fmt仅本批helper局部，diff--check0，不是全仓fmt、UI/build/安装/URL门禁。allowlist、退役扫描范围和门禁均未修改。

## 原14当前具名状态

| 用例（commands::agent_tests::） | 当前结果 |
|---|---|
| bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure | 失败，继续保留 |
| child_completion_executor_publishes_one_acknowledged_result_on_success | 通过 |
| child_completion_executor_result_is_not_published_before_resource_commit | 通过 |
| e2e_anonymous_spa_completes_without_strix | 通过 |
| e2e_confirmed_challenge_fuses_target | 通过 |
| e2e_missing_side_stops_at_insufficient_evidence | 失败，继续保留 |
| e2e_no_finding_closes_normally | 失败，继续保留 |
| e2e_personalized_permission_difference_does_not_confirm_idor | 通过 |
| e2e_two_equal_accounts_close_without_a_finding | 通过 |
| every_target_ends_in_exactly_one_terminal_state | 失败，继续保留 |
| http_journal_execution_stop_survives_target_run_and_checkpoint_projection | 失败，继续保留 |
| policy_native_coverage_only_is_ready_without_a_reviewer | 失败，继续保留 |
| policy_native_routes_without_runtime_adapter | 失败，继续保留 |
| public_surface_prepare_hands_real_observation_to_web_executor_without_replay | 通过 |

剩余7项不是全项目剩余量；原四项具名债、其他E2E/结构债仍须核对。旧missing-side/无发现、reducer/bootstrap/http_journal消费者与policy入口需逐项迁真实入口或证明生产合同问题，不恢复无原Root的raw回调换绿。额外只读发现：investigation_surface宣称authenticated_plus_anonymous/anonymous_control，而scan_identity_keys与当前完整绑定只生成登录身份集合。旧无发现夹具混anonymous+session不能算此能力真实通过；下一批须用真实creator/SDK证明匿名对照缺口，保持完整身份/权限校验。缺失捕获材料、HTTP已在途撤权、取消/强杀、实际登录与安装UI仍须独立验收，不能从五项推成全身份生命周期完成。

## 未提交保护与继续顺序

原基线1321文件7b69582beb658c601060853cd06a46dbd2ae60da8f81c424ed69678b2918cd20；最终同1321文件ffe432c5462d8610ca2c8ef3e2fa42280a21ade93e698ff0e930190e1ff55812，HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b保持。4已有路径（含此前未跟踪文件），1317原范围外SHA保持，无范围外新增。原bytes/prior git diff在/tmp/oviraptor-single-identity-original-before.json和-prior-diffs；四最终增量全文阅读，范围与hash在-scope-final.json/-code-snapshot.json。

- src-tauri/src/commands/agent_tests_fresh_single_production.rs
- src-tauri/src/commands/agent_tests_identity_e2e.rs
- src-tauri/src/commands/web_mode_test_support.rs
- src-tauri/src/commands/agent_tools_dispatch.rs

三原文档写前精确SHA及全字节备份在/tmp/oviraptor-single-identity-docs，新前缀后原完整suffix保持。无reset、批量覆盖、自动提交/推送；无真实DB/CAS/资产改写、安装App或外部URL访问。

继续REM-A01其他角色/Source/人工/失败删除、历史schema与审计UI/规模/残余活路径和Native JSON；再十维动态分配/精确对账/未知确认显式续跑、六Root触发、全15角色/General ReAct/Broker/Reviewer/实际并行、聊天/逐路日志/整体UI。非Web/数据依依赖推进，完整门禁、安装App打开、两授权URL匿名只读和真实模型质量最后。InputParser原自动审批拒绝、不可见完整原因保持，不绕过；Goal不标完成。
