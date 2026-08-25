# Strix 兼容升级通道

Oviraptor 将 Strix 上游版本假设集中在
`src-tauri/src/commands/strix_compatibility.rs`。日常补丁和小版本升级不应在扫描业务代码中增加版本号判断。

## 小版本升级检查

1. 更新 `STRIX_INTEGRATION_TARGET_VERSION` 和默认 sandbox 镜像。
2. 用新二进制运行 `--help`，确认目标、指令、非交互、模式、预算、回合、范围和 diff 参数；能力探测会按二进制路径、大小和修改时间自动失效缓存。
3. 核对 `run.json`、`vulnerabilities.json`、SARIF、CSV 和 `.state/agents.db`。仅在文件名或成功状态变化时调整兼容层。
4. 更新兼容层单元测试和一份真实产物夹具，再运行 Rust、Python、前端类型检查与生产构建。
5. 新任务的 `runtimePolicy` 会记录适配目标、实际 CLI、镜像和产物契约，便于回溯。

## 大版本升级门禁

自动更新只允许当前已审核的 Strix 主版本。新的主版本必须先检查：

- CLI 参数是否删除、改名或改变语义；
- Docker 镜像、浏览器环境和挂载目录；
- 运行状态及 Token 使用字段；
- 漏洞、SARIF 和 Agent SQLite 表结构；
- Oviraptor `sentinel_*`、调查图谱、验证记录是否需要新增字段或迁移。

完成审核后再修改 `STRIX_SUPPORTED_MAJOR`。数据库变更必须通过 `db.rs` 的版本化、幂等迁移完成，不能在页面或扫描执行器里临时补字段。

## 禁止的做法

- 在 `scan_execution.rs`、页面组件或结果入库中散落 Strix 版本号；
- 仅按版本号猜测 CLI 能力；
- 本地知识命中直接晋升漏洞或可验证状态；
- 重写历史任务 JSON，使旧任务看起来像由新版本执行。
