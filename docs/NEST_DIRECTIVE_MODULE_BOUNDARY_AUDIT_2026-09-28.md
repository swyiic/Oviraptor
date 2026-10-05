# 人工指令业务模块边界审计（2026-09-28）

本次只整理 Native 多智能体人工指令业务的代码结构。它不是 Master Plan §18 的最终验收，也不表示真实模型、浏览器、沙箱和授权 URL 已联测。工作树原有大量未提交变动，均未清理或重置。

## 模块归属和体量

| 文件（`src-tauri/src/agent_runtime/multi_agent/` 下） | 行数 | 职责 |
| --- | ---: | --- |
| `directive.rs` | 175 | 公开接口、数据类型、事实引用核验及旧同级模块的兼容导入 |
| `directive/interpretation.rs` | 358 | 聊天文本草案分类、URL scope 与主机边界识别 |
| `directive/draft_store.rs` | 386 | 草案创建、线程归属、完整性哈希及读取 |
| `directive/confirmation.rs` | 224 | 草案确认与取消，保持原事务/fencing 核验 |
| `directive/inbox_claim.rs` | 259 | 领取待处理指令和受 fencing 约束的状态迁移 |
| `directive/delivery.rs` | 172 | 模型上下文恢复和送达回执 |

各新增手写业务文件低于 Master Plan §4.1 的 400 行目标；没有把业务策略搬到万能共享 `utils`。原公开调用仍由 `directive.rs` 重导出；事务内辅助函数仅对同业务父模块及其子模块开放。`proposals`、`queue_actions`、`source_guidance` 仍沿用旧 `super::*` 导入，因而入口保留少量兼容性导入；下一次触及这些业务时可逐步改成显式依赖，但不应为了消除导入一次性重写其状态机。

## 行为和测试

- 草案分类及后续 SQL/确认/领取/送达代码按原函数边界搬迁；没有改变数据库 schema、JSON/hash 字段、状态名、授权校验顺序或事务提交位置。
- 没有新增一次性测试、重复夹具，也没有删除 unit/safety 回归。清理测试仍遵循 §4.1：先证明等价覆盖，再清除过期入口或重复夹具；故障注入、权限拒绝、历史兼容和预算守恒测试不得因压缩体量被删。
- 拆分中间阶段的 `cargo test -j 1 --lib directive -- --test-threads=2` 为 **108/108**；最终结构的 `cargo clippy -j 1 --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check` 和全目标全特性 `cargo test -j 1 --all-targets --all-features -- --test-threads=2` 均通过。全量 Rust 主库 **1393/1393**，历史导入器 **30/30**，主程序 **0/0**。全量测试单编译作业、双测试线程、低优先级运行。
- 本次没有改动前端代码；此前 UI 251/251 与生产构建的通过记录见 `NEST_UI_MODULE_BOUNDARY_AUDIT_2026-09-28.md`，不冒充本次重新执行。

## 余项

这次解决了 `directive.rs` 原 1512 行的结构债，但不是整个仓库的体量治理完成。`SentinelBoard.vue`、旧数据库 schema 与 HTTP 工具等存量大文件仍须在实际触及时按业务边界渐进拆分。安装态一致性、真实模型/浏览器/沙箱、授权目标与 Master Plan §18 仍待验收。
