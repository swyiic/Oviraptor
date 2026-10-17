# 设置与普通删除入口退役审计（2026-10-01）

## 作用域与问题证据

- `db.rs` 原把运行时归一化结果写回所有配置；`db_initialize_columns.rs` 的一次性 SQL 原删除六个旧控制字段并推进水位。临时库负向测试先失败：`agentBackendPolicy` 在启动后消失。
- 配置 API 原隐藏旧字段后整体保存 JSON；新临时库读→编辑→保存测试先失败，证明原字段被删除。
- 普通配置/任务删除原允许删除旧配置、finding/checkpoint/signature 和退役 run。新临时库负向测试均先返回成功，之后改为拒绝并比较完整数据库快照。
- 所有数据库测试仅操作唯一临时目录。没有访问真实 DB/CAS、清理旧文件或提交改动。

## 最小修改

- 移除两条启动持久清理路径。运行时仍只接受当前字段，旧凭据不能生成 Native Profile。
- 当前配置保存保留已有惰性字段，提交的新别名不能改写或激活旧字段；新增配置仍归一化为当前格式。当前字段清空的语义保留。
- 配置 API 从原大文件按职责提取到 `workspace_config_profiles.rs`。普通删除在事务内拒绝含惰性字段的配置；无效 JSON 也拒绝隐式替换或删除。
- `retired_data_guards.rs` 在结果面准备和普通任务删除前共享精确作用域拒绝，不解析旧格式，不做旧结果映射或迁移。
- 原删除用例保留已结清 Native 和当前 Native JSON 导入的正向语义，旧 run 的正向删除改由新的无写入拒绝矩阵覆盖。
- 漏洞原始记录使用中性未审核标签；旧发布模板原样保存至 `ARCHIVED_RELEASE_NOTES_2026-08-24.md`，移出产品前端。

## 验证与限制

- 启动保留专项通过；`retired` 专项 26/26 通过（预算改动前快照）。
- 配置编辑保存专项通过。普通删除专项正在本轮运行，结果以日志和 progress 最新记录为准。
- 实际 Vue SFC 编译/SSR 的来源标签与发布文案负向测试 2/2 通过；这是组件合同验证，不是安装态验收。
- 日志：`/tmp/oviraptor-master-settings-red.log`、`/tmp/oviraptor-master-settings-green.log`、`/tmp/oviraptor-master-settings-save-red.log`、`/tmp/oviraptor-master-settings-save-green.log`、`/tmp/oviraptor-master-delete-red.log`、`/tmp/oviraptor-master-delete-green.log`。
- 旧数据精确盘点、可恢复备份和独立确认清理工作流仍未实现；这些门禁会明确保留旧数据。Master、十维预算接入、完整聊天/日志/安装态和 URL 验收仍未完成。
