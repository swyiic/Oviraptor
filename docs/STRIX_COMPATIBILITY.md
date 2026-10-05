# 已废止合同：Strix 历史产物兼容边界

> 用户最新要求取消所有历史兼容。本文以下内容仅记录旧实现边界，**不是当前实施指令，也不得用于阻止删除退役代码或增加格式支持**。当前要求见 `NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md` §0 最新需求。保留本文是为了审计旧决策，不表示产品继续支持这些功能；实际代码和旧数据清理尚未完成。

本文替代原“兼容升级通道”。保留 Strix 产物兼容，不保留 Strix 活运行时。旧文中的 CLI 探测、版本常量、镜像升级和恢复流程已失效，不得作为实施指令恢复到产品中。完整发布条件仍以 `NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md` §18 为准；本文不宣告这些条件全部通过。

## 代码归属

- `src-tauri/src/artifact_import/`：历史文件发现、解析、canonical 修订、原件保存及导入诊断。
- `src-tauri/src/commands/artifact_import_status.rs`：导入报告和状态，不承担历史列表/预览的 SQL。
- `src-tauri/src/commands/historical_import_views.rs`：任务预览、历史导入列表和包预览；命令以只读连接读取已有数据库。
- `src-tauri/src/commands/historical_agent_trace.rs`：从 canonical 投影读取历史轨迹，不重新打开原件执行或重放。
- `src-tauri/src/commands/trace_display_privacy.rs`：详情返回前的展示过滤，不改写原始产物、模型输入或知识记录。

## 历史数据的权限与语义

1. 历史导入属于 `historical_external`、`unreviewed`、`readOnly: true`、`executionEligible: false`。展示边界要求受支持的 canonical schema 和真正的 JSON 布尔值，不能以数字 1/0 代替权限标记。
2. 支持旧版文件意味着由导入适配器转换为受支持的 canonical 表示，不是直接信任任意 schema 或历史声明。
3. 历史漏洞只能作为未审核候选；历史 coverage、成功状态、工具结果或 Token 用量不能替代 Native 执行证据、独立 Reviewer 结论或恢复权限。
4. 历史别名必须满足记录的来源/任务关系；撤销、删除和当前投影约束不能因再次导入而被绕过。
5. 原始字节保存与 UI 展示是不同责任。可选字段、未知扩展、反证及修订信息需要在导入兼容回归中核对；不等于全部原文字段应直接展示。凭据和明确标记的私有推理不得直接展示。

## 增加历史格式支持时

1. 使用已保存或合成的 JSON、JSONL、SQLite、SARIF、CSV、Markdown 夹具；不安装、探测、启动或更新旧 CLI/镜像来实现运行时回退。
2. 先写能够复现格式差异的导入测试，核对原件保存、字段语义、重复导入、修订、撤销与删除边界。复用既有夹具，不复制数据库架构或创建新测试框架。
3. 未支持或损坏的输入应保留诊断，不能用“完成”“无问题”或 Native 权威记录掩盖解析缺口。未知 canonical schema 不应进入只读历史展示合同。
4. 对导入及展示变更运行相关 Rust 回归、退役静态检查和严格 Clippy；前端变更还须 UI 回归及类型/生产构建。全量、打包和安装态验收另行记录，不能用专项通过替代。
5. 任何剩余旧名称都要有明确用途。修改命中退役登记表的文件后，先逐项语义审查，再更新对应摘要和理由；不得自动放行全部变化。

## 禁止的做法

- 为兼容历史数据重新加入旧执行器、依赖探测、安装、镜像拉取或恢复分支。
- 将历史 prompt、MCP 配置、工具声明、后端名称或完成状态当作执行授权。
- 把旧任务或本地知识直接晋升为 Native 已确认结论。
- 重写历史 JSON，让旧任务看起来由新后端执行；为压缩项目体量删除唯一的兼容或隔离回归。

具体证据、已知限制和安装态差异见 `NEST_BACKEND_RETIREMENT_AUDIT_2026-09-26.md`；最新验证状态见 `NEST_IMPLEMENTATION_PROGRESS_2026-09-23.md`。
