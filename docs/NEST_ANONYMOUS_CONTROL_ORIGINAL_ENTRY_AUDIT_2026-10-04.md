# 登录身份与匿名对照原入口审计（2026-10-04）

## 问题与数据作用域

证据合同已有 authenticated_plus_anonymous/anonymous_control，但 ordinary creator/一个合成有效草稿会话/startup/HMAC/原 Single Root/run_agent_target 的真正 SDK 比较只能得到零目标 HTTP。新正向回归首次0/1，测试3.00秒，日志 `/tmp/oviraptor-anonymous-control-original-red.log`。此前旧无 Root 的混合身份夹具不能证明此能力。

本批仅临时 SQLite、localhost 站点和脚本模型 provider，认证数据为合成 Cookie/Bearer；没有真实数据库/CAS/资产写入，没有外部 URL、安装 App 或真实模型推理质量验收。实际 SDK 传输、真实入口、原 Root/费用/结果消费是验证范围，provider 的推理决策由脚本给定。

## 最小生产修改

`agent_backend.rs` 在完整捕获登录身份集合后附加固定无凭据 anonymous；身份顺序及默认登录身份保持。`agent_tools_dispatch.rs` 仍先实时验证全部捕获会话的项目、任务、状态、期限、主机、独立认证材料；完整原账号集合可附带一个精确匿名 control，不能替代账号、接受别名、重复 control 或任何凭据。扫描 mode/认证会话 ID/绑定行保持，单账号不是双登录账号。原 Source、Multi capability/fencing、冻结计划、Root 和费用保护保留。

首 HTTP 收到时让同一临时会话过期，实际1次 SDK/1次 HTTP 后已停止第二侧，却误报 persistence_failure。三新增与原五撤权测试首批7过/1失败（8.55秒），日志 `/tmp/oviraptor-anonymous-control-original-cases.log`；费用和 owned 断言在该失败点之后，不能称首红已验证。`agent_tools_http/exchange.rs` 仅将观察登记错误交给既有 `agent_http_claim_error`，保留稳定授权码；未知文件/存储错误仍默认为 evidence_write_failed，不按文字猜授权、不更改消费者断言。

## 最终证明与门禁

登录与匿名正向实际各1次 GET，登录 Cookie/Bearer 成对，匿名不带 Cookie/Authorization，模型不含凭据。仅一个绑定账号，SingleIdentity保持，标签是当前身份/匿名会话，原 owned 消费、零漏洞、覆盖与账单吻合。直接 Broker 的8种伪造/缺失身份负向拒绝，原全应用行、rowid及费用保持、零 SDK/HTTP；此项是 Broker 负向，不冒充完整模型任务。

在途撤权从实际 SDK/HTTP 执行：首个已收到的登录 HTTP 后过期，第二侧零请求、无后续 SDK，结果 execution_authorization_denied。原 Root Single exit 可读，model_requests/target_requests各 consumed=1，reserved/indeterminate=0；owned 消费将目标暂停，零漏洞。重复 owned 消费保持原全部应用行/rowid/费用、tally=1及实际 SDK/HTTP 各1，不重发。

| 核验 | 最终结果 | 日志 |
|---|---|---|
| 三新增＋五原撤权 | 8/8，测试8.69秒；编译20.12秒 | /tmp/oviraptor-anonymous-control-original-cases-fixed.log |
| 逐名受影响集合 | 134/134，含编译总173.24秒，选择=通过，无ignore | /tmp/oviraptor-anonymous-control-affected-final.log |
| 严格 all-targets/all-features Clippy | exit0，阶段21.04秒 | /tmp/oviraptor-anonymous-control-clippy-final.log |
| retirement_（含exact与当前Native JSON原字节roundtrip） | 46/46，阶段5.81秒 | /tmp/oviraptor-anonymous-control-retirement-final.log |
| 原14独立复核 | 7过/7失败，阶段26.93秒，无ignore | /tmp/oviraptor-anonymous-control-original-debt-final.log |

当前 lib 2306项已编译，本批134相关和46退役的通过不是全部2306项通过，不是全量门禁或整体功能验收。

## 原14仍失败的七项

- `commands::agent_tests::bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure`
- `commands::agent_tests::e2e_missing_side_stops_at_insufficient_evidence`
- `commands::agent_tests::e2e_no_finding_closes_normally`
- `commands::agent_tests::every_target_ends_in_exactly_one_terminal_state`
- `commands::agent_tests::http_journal_execution_stop_survives_target_run_and_checkpoint_projection`
- `commands::agent_tests::policy_native_coverage_only_is_ready_without_a_reviewer`
- `commands::agent_tests::policy_native_routes_without_runtime_adapter`

以上不是全项目只剩七项。需逐项迁真实 creator/原 Root/owned 结果消费或证明当前生产合同问题；原四项历史债、其他 E2E/结构债未全盘清零。缺失/损坏捕获材料、请求取消/强杀、全通道恢复、实际登录和安装 UI 仍须独立验证。预算/六监督触发/全15角色与并行/聊天/逐路日志/整体UI等 Master 14大项最终条件仍未全部满足。InputParser原自动审批拒绝保持，完整原因不可见，不绕过。

## 未提交改动保护

基线1321文件 ffe432c5462d8610ca2c8ef3e2fa42280a21ade93e698ff0e930190e1ff55812；最终1322文件 `7434713cec17748152175954b1845a1f5abe617a634c046e0a5d51e4a4dfa372`。5路径4已有/1新（已有包括此前未跟踪文件），1317原范围外逐文件SHA保持，无范围外新增。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 保持，diff--check0。修改前完整原字节/prior Git差异保存在 `/tmp/oviraptor-anonymous-control-original-before.json` 和 `-prior-diffs`；backend原1202行差异分段完整阅读，全部5最终增量全文审查，新leaf局部rustfmt检查通过。没有reset/批量覆盖/自动提交。

主文档更新前原全字节备份在 `/tmp/oviraptor-anonymous-control-completed-docs`，写前SHA核验、更新后原全文后缀精确保持。主文档顶部为本批当前状态，旧批次保留为历史，不以旧测试快照冒充当前通过。
