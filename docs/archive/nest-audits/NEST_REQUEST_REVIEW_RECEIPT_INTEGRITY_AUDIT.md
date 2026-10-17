# 人工请求核对回执与真实聊天一致性审计

状态：已修复回执校验与聊天缓存缺口，最终全量门禁已通过，见第 4 节。不是未知效果结案、恢复执行、独立新任务交接或整个 Master Plan 完成的声明。衔接 `NEST_REQUEST_OPERATOR_REVIEW_AUDIT.md`。

## 1. 复现的问题

原实现首次写入人工核对时仅用 SQL 查询部分聊天事件字段；读取历史和幂等重试没有复核事件。团队时间线从核对表独立投影，增量查询可能直接跳过已经缺失的事件。前端保存只比对操作 UUID，聊天刷新失败则保留缓存。

这些路径组合后存在以下缺口：

- 核对事件被删除、重复或改到其他任务/attempt，历史与同操作重试仍可能被当成有效回执。
- SQL `json_extract(...)=0` 同时接受 JSON 数字 0 与布尔 false，也没有拒绝额外的矛盾权限字段。
- 团队聊天可显示没有对应事件的“已保存”，增量游标已越过该事件时问题更隐蔽。
- 同一 UUID 但判断、依据或来源不同的 IPC 返回值也能让核对表单显示成功、丢弃待重试提交。
- 即使后端开始拒绝坏回执，已缓存的聊天仍留在界面；迟到的旧成功响应也可能重新填回。

## 2. 已实现的修复

### 后端回执

`agent_request_review.rs` 的公共历史校验路径现在同时验证回执链和真实协作事件：

- 按唯一核对 ID 查找全部关联事件，必须恰好一条。
- 精确核对 scan、attempt、entity type、event type。
- JSON 对象必须与 `{disposition, executionUnlocked: false}` 合同一致；额外字段、错误类型、无效 JSON 和重复事件拒绝。
- 首次提交、幂等重试、历史读取、追加更正均经过相同校验。
- 失败只报告数据无法确认，不补造事件、不删除旧核对、不修改请求占用或执行状态。

协作事件表新增 `entity_id` 索引，避免每条回执校验全表扫描。通过既有 `ensure_schema` 为新旧数据库创建，不改已有回执 JSON 或摘要算法。

### 团队时间线

`native_scan_status_after` 在应用增量游标之前验证当前 attempt 的核对修订链及事件，并拒绝事件存在但来源核对缺失的情况。无论首次全量读取还是游标已超过坏记录的增量刷新，都不能返回一份假装该记录已验证的成功状态。

校验使用同一读取事务，不依赖聊天文字或 Agent 判断，不进行网络调用。

### 实际界面

`AgentRequestReview.vue` 将返回回执与冻结提交逐项比对，包括操作 ID、上一修订、来源摘要及快照、判断、依据、actor、核对 attempt 和有效时间字段。快照按 JSON 值比较，不受对象键顺序影响。字段不完整或不一致时保留原提交供同 UUID 重试，不显示保存成功。

`AgentDialog.vue` 收到请求核对校验错误后清空已缓存任务状态及增量游标，并阻止较早在途请求重新填充状态。下一次读取强制请求完整快照；已经在途的较新增量响应若丢失了依赖的缓存，也不能当作全量历史，必须重新读取完整快照。正常读取成功后再恢复展示。普通暂时性刷新错误仍沿用原有错误提示逻辑，没有借此改变任务执行状态。

## 3. 红测与回归范围

先新增测试、在未修复实现上运行：

- 后端事件完整性两项：0 通过 / 2 失败；分别复现缺失事件仍可读取，以及数字 0 被误接受。
- 后端时间线一项：0 通过 / 1 失败；复现全量时间线接受缺失事件。
- 实际核对 SFC：原有 7 项通过，新增 2 项失败；复现同 ID 异内容和缺字段回执被显示为成功。
- 实际聊天 SFC：原有 45 项通过，新增 1 项失败；复现校验失败后缓存消息仍在。
- 后续聊天并发红测：46 项通过，新增 1 项失败；复现失去缓存基础的在途增量被当作完整状态。

修复后的定向测试：后端 `request_review_` 13 项通过，实际核对 SFC 9 项通过，实际聊天 SFC 47 项通过。

新增测试矩阵覆盖：事件删除、跨 scan/attempt、错误事件类型、无效/矛盾 JSON、重复事件；首次写入失败整事务回滚；历史读取/同操作重试/追加更正拒绝坏链；全量及三种增量游标位置；核对内容被改、来源被删；错误响应保留准确重试；JSON 键顺序无关；缓存失效、迟到旧成功响应及完整快照恢复。

原有正向回归继续覆盖迟到机器响应、resume/fresh 区分、数据库重新初始化、并发唯一胜者、请求占用不退款/不重放及真实时间线。故障测试对比执行状态、预算/租约/消息快照，确认校验失败没有修改它们。

## 4. 最终验证

最终完整组合已收集进程退出码 0：

- `cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1`：主库 **963 通过 / 0 失败**，372.59 秒；历史导入器 **30 通过 / 0 失败**，1.89 秒。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`：通过，7.13 秒。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --all --check`：通过。
- 全部实际 SFC：**129 = 47 + 26 + 32 + 7 + 9 + 3 + 5**，全部通过；分别对应 dialog、workbench、native-status、request-usage、request-review、scan-deletion、project-deletion 七个脚本。
- `npm run build`：类型检查与构建通过，主 JS 813.57 kB；已有的大 chunk 警告仍保留，不以放宽阈值处理。
- `git diff --check`：通过。

最终 Rust 测试/Clippy/fmt 串联进程 handle `10675` 退出码 0；最终 UI/构建串联进程 handle `76599` 退出码 0。未改 retirement allowlist、未禁用测试或放宽执行门禁。

首轮中间版本全量主库为 961 通过 / 1 失败，进程退出码 101：`web_binding_live_dispatch_rechecks_real_settings_and_owns_exactly_one_claim` 在恢复原配置后的 claim 返回 `web_binding_inputs_changed`。该轮测试与后续源码重编译有重叠；实际冻结合同包含从 `current_exe()` 读取的可执行文件摘要，因此构建扰动是可能原因，但该错误没有记录逐字段差异，不能据此断言已证实唯一根因。没有修改该测试、摘要合同或派发门禁；停止 Rust 源码修改后另跑最终全量组合，该测试与整个组合均通过。首轮日志保留于 `/tmp/oviraptor-request-review-integrity-full.log`。

日志：

- `/tmp/oviraptor-request-review-integrity-before.log`
- `/tmp/oviraptor-request-review-integrity-timeline-before.log`
- `/tmp/oviraptor-request-review-integrity-ui-before.log`
- `/tmp/oviraptor-request-review-integrity-dialog-before.log`
- `/tmp/oviraptor-request-review-integrity-delta-before.log`
- `/tmp/oviraptor-request-review-integrity-final-targeted.log`
- `/tmp/oviraptor-request-review-integrity-final-full.log`
- `/tmp/oviraptor-request-review-integrity-final-clippy.log`
- `/tmp/oviraptor-request-review-integrity-final-fmt.log`
- `/tmp/oviraptor-request-review-integrity-final-ui.log`
- `/tmp/oviraptor-request-review-integrity-final-build.log`

## 5. 未完成与不能推出的结论

- 人工核对仍不是机器效果确认，不退还请求预算，不解除停止，不授予新任务权限。
- 未交付未知结果的行政结案与独立任务交接。必须继续处理结案回执、在途调用、能力撤销、新授权合同与关联证据，不能将本次核对表当成执行许可。
- 没有新增 Host Agent、主机审批后端、远程执行入口，也没有部署或访问外部目标。
- 本轮 UI 回归是实际 SFC setup/template 测试，传输与宿主节点采用测试替身；没有声称桌面端到端或真实浏览器验收。
- 本地校验及追加式表不是抗数据库管理员的签名审计系统；有权限改写整库并重新计算全部摘要的主体不在本保证内。操作者仍是非认证的 `local_operator`。
- 未增加历史分页；大型历史数据集的轮询性能仍需验收。索引只改善实体查找，不证明整体规模性能。
- 本轮只核验人工核对回执，不将结论推广到全部协作事件、全网络通道或所有专家角色。其余 Master Plan 工作继续保持未完成。
