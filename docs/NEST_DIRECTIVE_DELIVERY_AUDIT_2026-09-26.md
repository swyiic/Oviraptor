# 人工指令并发、恢复与模型送达审核（2026-09-26）

## 结论与范围

本轮修复的是人工指令的隔离、持久化恢复和真实模型送达记录，不等于完成 Master Plan §7.5.3 的结构化动作闭环。模型收到文本，不证明队列已经调整、角色已经派发或用户要求的动作已经完成。

当前 Native 路径不再通过“把文本加入 history”自动执行 `accepted → applied`。用户指令保持 `accepted`，模型返回成功后另外记录 `model_received`。聊天界面明确提示尚无动作落实回执。整体 Reviewer 通过也不能把这些仅送达的指令自动变成 `completed`。

未部署、未访问外部授权 URL、未增加主机执行器或改变 `web_only` 权限。Linux 工具沙箱与目标主机授权继续分离。

## 已复现并修复的问题

| 问题 | 修复前的可复现结果 | 修复与边界 |
| --- | --- | --- |
| 同一扫描有两个目标 | A 领取时误拒绝 B 的合法待处理指令 | 隔离无效指令的 UPDATE 限定当前 target；不能用 A 的租约判断 B 的确认 |
| 等待 SQLite 写锁期间租约更换 | 旧 worker 仍能接受指令或写完成态 | 获取 IMMEDIATE 写事务后再次核对权威租约；更新精确绑定 scan、attempt、target、root、epoch、token |
| 扫描暂停、取消、替换 attempt 或删除 | 尚存活的租约仍可领取、接受或应用指令 | 推进工作同时校验当前 active attempt；已执行工作的失败收尾与新派发分开 |
| 模型请求尚未发出 | 指令已显示 `applied` | 领取、接受与模型送达分离；移除提前应用 |
| claim 后中断 | 重启只读取 pending，claimed 永远无法恢复 | 每轮从 DB 恢复当前租约下的 claimed/accepted；恢复 claimed 时使用同一状态转换校验 |
| history 重建或压缩 | 已接受约束没有可靠恢复依据 | 每轮单独重建已确认指令上下文，不重复追加到长历史；使用现有 keep 标记避免改变约束原文 |
| root/DB/租约读取失败 | 返回空数组，伪装成无新消息 | 传播错误至可见 persistence stop；只有明确无 run 的独立诊断模式返回空 inbox |
| 送达记录与事件分开提交 | 可能只出现其中一项 | 模型事件、送达记录、聊天增量事件位于同一写事务；任一写失败全部回滚 |

## 状态语义与恢复契约

1. `pending`：用户已确认草案，尚未领取。
2. `claimed`：当前 Coordinator 已持久化领取；崩溃后可恢复，不表示模型已收到。
3. `accepted`：指令可进入当前冻结范围下的模型评估。它仍不是动作执行证明。
4. `payload_json.modelDelivery.state = model_received`：某次模型请求成功返回，且该请求包含这条指令。附带 `runId`、`eventSequence`、`receivedAt`；对应 `ModelRoundCompleted` 的 `deliveredDirectiveIds` 可追溯到实际模型轮次。
5. `assigned/applied/completed`：本轮不以文本送达代替这些动作状态。后续必须由实际 Assignment、队列变更或其他结构化动作回执推进。

模型网络错误、认证错误、协议错误或取消不产生成功送达回执。远端可能收到请求但本地没有成功响应时，不宣称 exactly-once，也不声称已经落实动作。恢复会重新携带仍然有效的 accepted 请求，执行工具本身的幂等、未知结果与副作用审批仍归原有执行链负责。

同一有效租约下恢复 claimed/accepted；租约 epoch/token 或 root 被替换时，不静默继承旧批准。旧 claimed/accepted 转成 `deferred`，reason code 为 `directive_reconfirmation_required`，并进入聊天增量事件。当前用户需要重新提交并确认草案；尚无一键重确认旧草案的专用 UI。

恢复还会复核草案完整性、文本、确认版本、thread、冻结绑定与引用证据时效。无效记录显示 `rejected / directive_context_invalid`，不重新注入模型。

## 持久化兼容性与代码位置

不改变历史 JSON 导入契约，不改 `agent_user_directives` 的状态枚举。送达信息是已有 `payload_json` 下的附加字段，原有草案和指令字段保留。

- `src-tauri/src/agent_runtime/multi_agent/directive.rs`：领取隔离、事务内 fencing、active-attempt 检查、上下文恢复与送达写入。
- `src-tauri/src/commands/agent_native.rs`：严格读取 inbox、逐轮恢复约束、成功响应后写回执。
- `src-tauri/src/commands/agent_runtime.rs`：同事务写 `ModelRoundCompleted` 与模型送达回执。
- `src-tauri/src/collaboration_events.rs`：新增独立 delivery trigger；已有数据库无需替换旧 status trigger 即可安装。
- `src-tauri/src/commands/native_scan_branches.rs`：增量时间线显示真实 `deliveryState` 与 reason codes。
- `src/features/sentinel/components/AgentDialog.vue`：明确区分接受/送达与动作落实。

## 回归证据

### 先失败后修复

- `/tmp/oviraptor-20260926-directive-race-complete-red.log`：双目标误拒绝、失效租约写入、暂停后领取三类失败。
- `/tmp/oviraptor-20260926-directive-delivery-red.log`：提前 applied、领取后无法恢复两项失败。

### 新增测试覆盖

`agent_tests_directive_concurrency.rs` 新增 6 项，覆盖双目标隔离、真实 SQLite writer 竞争、非活动任务、替换/删除 attempt、失败收尾和状态/事件原子性。

`agent_tests_directive_delivery.rs` 新增 8 项，覆盖提前应用、claim 中断恢复、accepted 恢复及压缩约束、错误传播、模型事件/送达/时间线回滚、旧租约拒绝及可见 defer、上下文完整性、真实 Native 模型请求成功与认证失败。

真实模型请求测试只使用 loopback HTTP 服务：检验每轮请求包含且只包含一份已确认指令；成功响应落送达回执，401 不落；两者均不把指令改为 applied。

最终验收结果：

| 验收 | 结果 | 日志 |
| --- | --- | --- |
| Rust 全 targets / 全 features，离线、单测试线程 | 主库 669 项、导入工具 30 项通过；含退役字面量及假 CLI 正反对照 | `/tmp/oviraptor-20260926-directive-final-rust.log` |
| 全 targets / 全 features 严格 Clippy | `-D warnings` 通过 | `/tmp/oviraptor-20260926-directive-final-clippy.log` |
| 指令定向测试，含最终扩充的两轮模型请求断言 | 19 项通过 | `/tmp/oviraptor-20260926-directive-final-contracts.log` |
| 真实 Vue setup 逻辑回归 | 29 项通过；不是 DOM/IPC/视觉验收 | `/tmp/oviraptor-20260926-directive-final-dialog.log` |
| 前端构建 | 通过；主 JS chunk 770.21 kB，仍有体积告警 | `/tmp/oviraptor-20260926-directive-final-build.log` |
| 本地 Native 浏览器回环 | 8 次请求；匿名及隔离身份采集通过 | `/tmp/oviraptor-20260926-directive-final-native.log` |
| Rust fmt / `git diff --check` | 通过 | 命令直接返回 0 |

全量 Rust 运行后只扩充了模型请求测试的连续两轮断言；该最终断言随后在 19 项指令测试中执行通过，严格 Clippy 也已读取该版本测试文件。日志位于本机临时目录，不等于可长期保留的安装包或跨平台验收产物。

## 明确未完成，后续实现不得绕过

1. **结构化动作闭环**：把草案转换成真实 priority change、proposal request、Assignment；维护 directive → action → child run → result 的关联。现在仍主要是受约束的模型上下文，不得宣传成已完成调度控制。
2. **逐动作完成判定**：新指令不再因整体审核结果误报完成，但旧 applied 记录的历史语义及按整体 review 批量收尾仍需专门迁移/核验。
3. **任务结束的未落实指令**：需要明确 abandoned/deferred/未覆盖等收口策略，不能把 accepted 当 completed，也不能因任务暂停而丢弃可恢复请求。
4. **长期上下文管理**：当前保留有效 accepted 约束原文；大量积累时需按结构化动作状态退役/合并或明确提示上下文容量，不能静默截断否定条件。每轮新领取最多 50 项，不宣称无限吞吐或无限上下文。
5. **恢复与 UI 验收**：同租约的 DB 恢复有测试，不等同完整桌面重启/Tauri IPC/视觉端到端；旧租约重新确认需要产品化入口。
6. **整个 Master Plan**：Stage 9 真实补证再审、Stage 10 剩余专家、聊天 thread/未读跨刷新持久化、真实隔离沙箱、安装包与 Linux 实机验收等仍保持原范围，未因本轮修复而缩减。

只有这些要求分别取得真实执行证据，才能宣称用户指令与多智能体闭环完成。不得通过重命名状态、增加模拟聊天或放宽测试来替代。
