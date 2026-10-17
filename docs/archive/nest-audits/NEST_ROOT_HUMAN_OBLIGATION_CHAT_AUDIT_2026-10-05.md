# 原人工评估待核对聊天投影审计（2026-10-05）

Master未完成，Goal仍active；暂停确认未收到。此批完成只读待核对快照与聊天显示，并完成本批关联门禁，不是框架或整体验收。保护未提交改动、不自动提交，当前Native JSON保持可读；未操作真实DB/CAS/asset、安装App或授权URL。

原人工评估未知费用或未发布结果原先不出现在付费投影中。现从原冻结请求、原确认上下文和原财务记录投影只读humanAssessmentObligations，按原线程显示确认版本、未知派发/费用状态、供应商报告用量（未对账）；无报告时不补零。坏快照不替换已验证状态或推进游标，同游标刷新/清除不制造消息、已读回执、付费摘要、执行权限或自动恢复；父退出/当前指令损坏仍读取原上下文。卡片复用原主题变量，视觉和实际App仍待验。

同源码关联362/362（测试605.88秒/阶段618.82秒，完整保留前356及新增6，无ignore），严格all-features/all-targets Clippy0（22.76秒），同二进制退役50/50（5.22秒，含literal/当前Native JSON），三新Rust叶fmt及17路径diff0。聊天Vue模拟IPC350/350（测试17.80秒/阶段18.42秒），vue-tsc/Vite构建0（6.77秒）。当前2433项Rust全量、整体UI、安装App打开/历史崩溃、授权URL与真实模型质量/美元对账仍未验收。

17代码路径12已有/5新逐文件前像、原Git差异及增量保留；1397原范围外源码与HEAD59be3d86保持，1414源码SHA 49f7c366cc21f02d8d9ee539d6772fb9892fd42375619b7c1033a4c437d41c0c。最初后端3/3红灯、新UI0/5红灯；新增财务篡改/作用域测试与原3合计6/6。首轮整组UI342/350的8项旧手工loader不认识新模块，现加载实际生产校验代码，保留原断言与监听规则；首轮Clippy cmp_owned修为显式保存规范JSON字符串，未削弱字节核验。为必要源码修复中止首轮Rust（75项完成/1在途、其余未执行），不把在途列作原测试失败；保存首次证据后从头完整重跑362/362。所有SDK均临时localhost响应的实际生产链路，非真实供应商推理质量验收。

仍未完成：unsent/无终态/已知费用但摘要无效的完整实际后端状态、重复/截断边界；无已保存决定的received回执缺少原终态物理costProof，结构核验不能替代或补造原证明。显式对账、授权恢复、重启和全部角色监督闭环仍待实现。以下REM-A01—A14全部剩余继续有效，不将局部读取或UI模拟等同于整体完成。

证据作用域为临时SQLite/CAS、真实localhost SDK生产入口、Vue实际setup/render及模拟IPC；不含真实数据库、业务资产、安装App、目标URL或真实模型质量。不能据此关闭Master十四类要求。

已验证的后端新增六测试：

- `commands::agent_tests::root_human_unknown_assessment_original_thread_snapshot_retains_usage_without_cursor_or_repayment`
- `commands::agent_tests::root_human_unknown_assessment_snapshot_keeps_original_thread_after_parent_exit_and_current_damage`
- `commands::agent_tests::root_human_unknown_assessment_snapshot_rejects_original_request_tamper_without_write`
- `commands::agent_tests::root_human_unpublished_received_rejects_original_invoice_and_physical_cost_damage`
- `commands::agent_tests::root_human_unpublished_snapshot_isolates_scan_and_attempt_without_granting_authority`
- `commands::agent_tests::root_human_unpublished_uncertain_rejects_terminal_cost_and_original_reserve_damage`

篡改只施加于完成实际原SDK请求的临时夹具，比较拒绝读取前后的typed rows与物理行；没有修改真实账本或恢复执行。后端未知/超额实际调用从原Root三次+Mapper一次到Human一次，共五次SDK；显示和拒绝读取没有第六次请求或目标I/O。received原100000输入用量保留为未对账报告，HTTP503未知保持null。父退出/当前指令损坏只影响现在权限，不改变原上下文。

五新增UI测试：原四线程独立显示；未知/未发送/等待/未发布只读提示；坏envelope/回执上下文/用量/日期/重复/越界数组拒绝且游标不前进；同游标更新、清除、去重且不保存已读；供应商报告不一致用量仍标未对账，缺省可选元数据保留原Native读取。其余后三种后端状态的UI覆盖使用模拟IPC，不可称为已验证实际后端生产者。状态校验模块由两手工harness加载实际源码，不用恒true替身。

代码作用域（相对仓库根目录）：

- `src-tauri/src/commands/agent_tests_root_human_obligation.rs`：新增
- `src-tauri/src/commands/agent_tests.rs`：已有文件最小增量
- `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/unpublished_projection.rs`：新增
- `src-tauri/src/commands/native_scan_branches/status_human_assessments.rs`：新增
- `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick.rs`：已有文件最小增量
- `src-tauri/src/commands/native_scan_branches.rs`：已有文件最小增量
- `src-tauri/src/commands/native_scan_branches/status.rs`：已有文件最小增量
- `tools/test_root_human_obligation_ui.cjs`：新增
- `src/features/sentinel/timeline/humanAssessmentContract.ts`：新增
- `src/types.ts`：已有文件最小增量
- `src/features/sentinel/composables/useAgentDialogStatus.ts`：已有文件最小增量
- `src/features/sentinel/composables/useAgentDialogReading.ts`：已有文件最小增量
- `src/features/sentinel/components/AgentDialog.vue`：已有文件最小增量
- `src/features/sentinel/components/agentDialogPresentation.css`：已有文件最小增量
- `tools/agent_dialog_harness.cjs`：已有文件最小增量
- `package.json`：已有文件最小增量
- `tools/agent_dialog/status_lifecycle.cjs`：已有文件最小增量

复核命令与证据：`npm run test:agent-dialog`；`npm run build`；src-tauri目录下`cargo clippy --offline --locked --all-features --all-targets -j 1 -- -D warnings`；三新增Rust叶`rustfmt --edition 2021 --check`；完整关联集合由`/tmp/oviraptor-human-obligation-final-run.py`按前356原集合加六测试选择，cargo offline/locked/all-features/lib构建同一二进制后逐个exact运行，无ignore。最终选择/报告/通过集合均362；退役50原集合在同二进制exact运行。2433列出项中未选2071项，不声称全量通过。

前像、原Git差异、最终增量、源码集合、日志和结果：`/tmp/oviraptor-human-obligation-*`；首次3项通过保存在`/tmp/oviraptor-human-obligation-initial-3/`；首次UI失败、Clippy失败、主动中止Rust及当时源码保存在`/tmp/oviraptor-human-obligation-first-gates/`。中止时75完成/1在途，不将runner把未完成用例列为failure的机械结果当实际断言失败。最终完整重跑已取代此中止结果，不拼接局部通过。文档前像及SHA收据：`/tmp/oviraptor-human-obligation-final-docs/`。

后续最优先缺口是原终态物理证明/缺决定回执、剩余实际状态、显式对账恢复；不从当前账本后补冒称原证明，不自动重发，不以原聊天线程选择授予新收件人权限。其余Master14保持完整，参见最新版Master顶部。
