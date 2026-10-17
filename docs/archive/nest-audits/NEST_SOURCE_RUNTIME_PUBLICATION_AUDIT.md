# 源码任务模型、预算与人工策略发布合同审计

状态：发布合同与生产入口验证已实现，该增量完整回归通过。本文不是源码多智能体生产编排或整个 Master Plan 已完成的声明。

## 1. 修复的问题及生产边界

此前源码工作台已经冻结源码范围与 CI 政策，但源码专家需要的模型选择、预算、人工策略和技能输入未形成完整的发布回执。后续启动不能根据可变设置或历史 task JSON 临时推导模型执行权。

本次在真实 `publish_workbench_start` 事务内注册 `source_runtime_contracts`，生产 `run_native_source_scan` 在快照和分析器启动前核验。入口当前保留返回的模型/runtime/合同，尚未将其接入专家编排；不能因为多出回执就声称已经调用 RepoMapper、SourceAnalyst 或 Reviewer。

主要文件：

- `src-tauri/src/commands/native_source_runtime.rs`：注册、重建候选输入与验证。
- `src-tauri/src/commands/workbench_startup.rs`：发布事务绑定及最终后置条件。
- `src-tauri/src/commands/native_source_scan.rs`：生产入口核验；注入分析器的测试 seam 不冒充生产模型调用。
- `src-tauri/src/db_schema.rs`：不可变回执表与约束。
- `src-tauri/src/commands/tests_source_runtime.rs`：10 项新增回归。

## 2. 冻结和不授予的内容

公开合同绑定 scan/attempt/project、实际工作目录与源码范围，保存模型公开策略、人工策略、有效策略、模式、时限、token 上限、预算声明、技能名称和指令。明确 `targetRequestsGranted=0`、`hostActionsGranted=0`。

模型 endpoint、API key 和代理配置不进入公开合同；它们与 task 文件摘要、完整模型环境、工作目录身份一起进入 HMAC 材料。Unix 目录 dev/inode 也参与绑定。每个 attempt 独立生成 32 字节私钥，使用 create-new、私有文件权限及 fsync；不覆盖或收养遗留密钥。

`WebBindingDirectory` 只复用安全文件操作，未生成 Web URL、Web 执行计划、浏览器身份或目标请求权限。

重要语义：当前实现从设置重建候选后与已发布回执匹配。模型或相关预算/策略发生变化时拒绝执行，不是保存旧明文凭据并继续使用旧配置。主题等无关设置变化不影响有效合同。

## 3. 原子性、恢复与历史兼容

1. 回执与 scan/attempt/context/branch 等发布数据在同一数据库事务提交。
2. 回执插入被 ABORT/IGNORE、最后插入触发器破坏前序数据、分支被提前领取时，发布拒绝并回滚。
3. 私钥文件不能随 SQLite 回滚自动消失；遗留文件不能据此补建授权。新的尝试使用新的目录与密钥。
4. 回执禁止 UPDATE/REPLACE 及存活父记录下的单独 DELETE；合法父记录删除仍支持级联。
5. 迁移不根据历史 JSON 或已有密钥补建可执行回执。历史数据兼容与本次执行授权分离。
6. 验证缺少 task、私钥、回执，或者目录/文件身份不符时，不自动修复或降级放行。

## 4. 已通过的定向验收

新增 10 项覆盖：

1. 冻结人工策略、预算、技能和模型；公开 JSON 不含配置凭据；验证不创建 run。
2. 模型、endpoint、凭据、token、时限、代理排除、人工指令和技能变更被拒绝，真实生产入口在快照前停止。
3. 回执 ABORT/IGNORE 的发布原子回滚，旧私钥不补权。
4. 最后插入触发器修改 context、预算、模型 key、task_path、targets 或 source claim 时整批回滚。
5. 缺失/替换私钥、task 篡改、伪造 tag 被拒绝，无读时修复。
6. Unix 不安全权限、symlink、hardlink 和目录替换被拒绝。
7. 回执不可变、独立新 attempt 密钥、旧 attempt 拒绝及父删除级联。
8. 迁移不从旧 task/私钥回填执行权。
9. 非法美元预算或发布准备后更换模型摘要时不发布。
10. 无关 UI 主题设置不导致误拦。

最终定向 session **43747 已退出 0**：源码相关 **118 passed / 0 failed**（包含上述 10 项）、Stage 4 路由 **3 passed**、退役守卫 **4 passed**、严格 Clippy 通过。日志前缀 `/tmp/oviraptor-source-runtime-`：`source-v2.log`、`routing-v2.log`、`retirement-v2.log`、`clippy-v2.log`。

实际界面 session **82462 已退出 0**：聊天、工作台、运行状态、用量/复核、任务/项目删除共 **147 passed / 0 failed**，日志 `ui.log`（同前缀）。构建和 localhost 浏览器 session **8561 已退出 0**：匿名与对照采集均 complete，8 个观测请求、身份隔离通过；日志 `build.log`、`browser.log`。主 JS 830.15 kB 的拆包警告仍存在，未通过提高阈值消除。

完整串行 Rust session **86524 已退出 0**：主库 **1103 passed / 0 failed**（532.45 秒）、历史导入器 **30 passed / 0 failed**；后续严格 Clippy、fmt 与空白检查均通过。日志 `/tmp/oviraptor-source-runtime-full.log`、`clippy-final.log`、`fmt-final.log`、`diff-final.log`（后面三者同前缀）。运行期间没有修改 Rust 源码或构建输入。本结果只涵盖源码发布合同增量，不涵盖之后的多引擎来源修改。

## 5. 真实失败及修正

- startup 首轮 **3 passed / 8 failed**：fixture 插入并列默认模型配置，但生产选择的是原默认配置。修为更新实际默认 profile，不放宽生产模型校验。
- startup 第二轮 **9 passed / 2 failed**：macOS `/var` 与 `/private/var` 目录身份不同。fixture 使用 canonical 根和私有目录权限，不跳过文件安全检查。
- 修正后 session 46177：当时的 8 项 runtime 与 11 项 startup 全通过；随后增加预算/选择与无关设置 2 项回归。
- 扩大源码回归首轮 **115 passed / 1 failed**：生产路由测试改为真实发布后，发现表会有信息级 source inventory；原“表为空”的假设不成立。修为确认唯一合法 inventory，并逐列比较分析前后整条记录不变，仍禁止新增/伪造漏洞结果。
- 退役 CLI trap 与生产路由测试改为真实发布 fixture 后，人工复核 REM-012 两个文件的全文摘要和理由。字面量数分别维持 17 与 4，不扩展扫描豁免、不移除守卫。

原失败日志保留：`startup.log`、`startup-v2.log`、`source.log`（同 `/tmp/oviraptor-source-runtime-` 前缀）。

## 6. 必须继续完成的生产执行链

以下为未完成项，不能以本次合同测试替代：

1. **真实 Source Coordinator 注册/恢复。** 从核验后的合同与本轮 snapshot/view/results/plan 生成 root，恢复不能用 `store::create_run` 的 upsert 覆盖旧状态；无真实材料不构造假 Web root。已终态和未知在途执行必须分别处理。
2. **生产专家派发。** 将已有 `source::task_slice`、`assessment_input`、scheduler 和共享 transport 接到源码启动器，逐次派发和恢复都核验发布合同。入口一次验证不够。角色初评的“未读源文件/未执行工具/未独立复核”限制必须保留，不能写成完整代码审计。
3. **模型预算与费用。** 本次冻结的是预算声明，不是完整美元计价、派发前费用预留或未知账单对账。不能忽略 `maxBudgetUsd`，或把目标请求上限直接当成无限模型调用授权。时限、token、已花费/已预留、取消和未知调用需有完整收口。
4. **真实源码工具与完成回执。** 去除源码工具对 Web-only 授权入口及无关 MAX(revision) 的依赖，将 `SourceBroker::finish` 接到持久 assignment/child/mailbox。候选必须绑定当前接受修订。
5. **独立 Source Reviewer 与 CI。** 真实独立 run 和审查动作绑定当前 candidate/revision/root；缺审查保留 `source_review_not_completed` 和 coverage-incomplete，不提前移除门禁。
6. **事件和用户指令。** UI 展示已持久提交的角色消息、真实派发/拒绝/完成及人工引导消费记录；不存在的模型对话不得补造。源码单独任务及 greybox 双分支都需验证停止粒度和结案。
7. **完整产品验收。** 沙箱工具供应、知识治理、资产、安装包及授权环境测试仍按 Master Plan 执行。本增量不覆盖这些要求。

本轮没有部署、访问用户公网 URL、执行主机操作或启用 Host Agent；主机边界继续遵循既有决策。
