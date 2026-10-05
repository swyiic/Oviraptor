# Source 未发出、标准收尾与观察者审计（2026-10-02）

Master 未完成。本批补 Source 工具模型的 typed 未发出证明、首轮零费用资源收尾及重复观察者保护。没有真实 DB/CAS/资产/安装包/外部 URL 或自动提交。

## 问题和作用域

HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 未改变。基线为 worker 账本批844路径，当前13文件（9已有、4新增）逐文件保存文本并保留其他未提交改动。所有合同使用临时库；真实生产入口通过 localhost 初次两个请求取得 Source 评估结果，随后以续租 IGNORE 故障使真实 gateway 在 I/O 前取消，Source SDK 请求数没有增加。

## 修改

- Source complete_once_observed 保留 BeforeTransport 和 TransportOutcomeUnknown。后者继续保存原未知费用；前者只接受 user_cancelled，保存绑定 root/assignment/child/attempt/round/Coordinator fence/request hash 的 canonical unsent 事实。事务只准新增一行事实，幂等重放零写入；IGNORE/ABORT/触发器附带写入回滚。
- 开始、received、uncertain 不得跨过已有 terminal cost fact 恢复业务执行；Source 原轮次的 unsent update guard 拒绝修改。费用保存和执行权限仍独立。停止 admission 匹配原 assignment，并验证 canonical 未发出原因，损坏恢复事实不能解除阻断。
- audit_unsent_first 核验完整 SourceTools 首轮/pristine request、精确 worker/fence/scope、零模型费用、无工具/事件/快照/消息/证据副作用。标准 failed_specialist_error 仅凭此证明使用原零费用资源收尾；最后写入再次核验。后续轮已收费时不释放旧资源，也不声称完成结果。
- 三种 Source 重入观察错误不拥有 worker，标准错误处理只读返回，不暂停/撤销原执行者。未泛化 binding 错误：真实原 owner 发生权限失效仍须暂停。

## 红绿和最后验证

1. `/tmp/oviraptor-source-unsent-production-red.log` 0/1：实际取消产生7,293未决token。第一修复1/1。
2. `/tmp/oviraptor-source-unsent-negative-red.log` 2/3：恢复库的错误 assignment 未发出事实误解除 stopped admission；严格绑定后4/4。
3. 早期192/192属于标准收尾改动前快照，不作为本批最终结果。
4. `/tmp/oviraptor-source-unsent-cleanup-red.log` 0/1：实际标准收尾滞留199,990token；首轮证明接线后5/5。
5. `/tmp/oviraptor-source-unsent-cleanup-negative-red.log` 2/3：重复观察者错误改写原worker；修复后 `/tmp/oviraptor-source-unsent-final-contracts.log` 8/8（5.50秒）。七项新增Source合同加一项既有专家合同，不能与扩大集合相加。
6. 最后格式后 `/tmp/oviraptor-source-unsent-final-affected.log` 205/205、exit 0（132.06秒）；包括Source/专家/worker存储升级、attempt、预算、child错误路径及退役字面量登记。库现1688项，未跑最终全库。
7. `/tmp/oviraptor-source-unsent-clippy.log` 全目标/全特性严格Clippy exit0（12.60秒）； `/tmp/oviraptor-source-unsent-importer.log` 39/39（2.29秒）；13文件作用域fmt、git diff --check及848路径最终哈希一致。

最后运行快照 `/tmp/oviraptor-source-unsent-code-snapshot-final.json` 摘要 `88b126f0f7d88cb7b72476d89d26409106a5f18dda38c894e7791a84eb4b22e3`。旧847路径中间快照已被覆盖；修改前文本 `/tmp/oviraptor-source-unsent-before.json`，scope `/tmp/oviraptor-source-unsent-scoped-files.json`，逐文件增量 `/tmp/oviraptor-source-unsent-scoped.diff`。model_facts 修改前备份通过反向删除唯一新增常量恢复且与基线哈希核对相同。

## 文件体量

| 文件 | 行数 |
|---|---:|
| `src-tauri/src/commands/tests.rs` | 97 |
| `src-tauri/src/commands/native_source_tool_execution.rs` | 183 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds.rs` | 316 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds/dispatch.rs` | 372 |
| `src-tauri/src/agent_runtime/multi_agent/budget/model.rs` | 321 |
| `src-tauri/src/agent_runtime/multi_agent/budget/model_facts_schema.sql` | 31 |
| `src-tauri/src/agent_runtime/multi_agent/budget/model_facts.rs` | 169 |
| `src-tauri/src/agent_runtime/multi_agent/budget/admission.rs` | 89 |
| `src-tauri/src/commands/multi_agent/gap_validation.rs` | 344 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds/unsent.rs` | 107 |
| `src-tauri/src/commands/tests_source_unsent.rs` | 142 |
| `src-tauri/src/commands/tests_source_unsent_production.rs` | 116 |
| `src-tauri/src/commands/tests_source_unsent_cleanup.rs` | 148 |

## 未完成与风险

- Source 后续已知收费前缀加未发出轮次仍保守暂停；未实现自动已知费用闭合或重新派发。原未知费用始终不退款/重试。
- scheduler 尚只能首次签发。生产新worker派发、不可变替换证明、Reviewer canonical恢复、Source全局计数与后台监督/真实强杀需继续实现。
- Root/Single完整账本与grant/精确对账、真实全角色/模式、复杂用户聊天/工具闭环、AST/浏览器/进程逐路日志、旧数据精确盘点备份清理和最终全量/安装态/授权URL均尚未交付。局部自动化通过不构成整体功能验收。
