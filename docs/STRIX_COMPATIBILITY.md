# Strix 兼容升级通道

Oviraptor 将 Strix 上游版本假设集中在
`src-tauri/src/commands/strix_compatibility.rs`。日常补丁和小版本升级不应在扫描业务代码中增加版本号判断。

## 小版本升级检查

1. 更新 `STRIX_INTEGRATION_TARGET_VERSION`、`STRIX_MAXIMUM_AUDITED_VERSION` 和默认 sandbox 镜像。
2. 用新二进制运行 `--help`，确认目标、指令、非交互、模式、预算、回合、范围和 diff 参数；能力探测会按二进制路径、大小和修改时间自动失效缓存。
3. 核对 `run.json`、`vulnerabilities.json`、`coverage.json`、SARIF、CSV 和 `.state/agents.db`。仅在文件名或成功状态变化时调整兼容层。
4. 更新兼容层单元测试和一份真实产物夹具，再运行 Rust、Python、前端类型检查与生产构建。
5. 新任务的 `runtimePolicy` 会记录适配目标、实际 CLI、镜像和产物契约，便于回溯。

## 大版本升级门禁

自动更新只允许当前明确审核到的最高版本；不仅新的主版本会被拦截，同一主版本内高于 `STRIX_MAXIMUM_AUDITED_VERSION` 的版本也必须先完成本清单，避免“主版本相同”掩盖产物语义变化。

## 1.6.x 产物语义

- `coverage.json` 是覆盖账本，不是漏洞列表。Oviraptor 将其保存为独立检查点和 `coverage_summary`，展示已测试面、未发现、排除、不适用、待跟进和完整性缺口。
- `findings.sarif` 同时包含漏洞失败结果与覆盖结果。只有 `kind=fail`（或旧版未声明 `kind` 且 `level!=none`）可以进入漏洞结论；`strix-coverage/*`、`pass`、`notApplicable`、`open` 永远不能转成漏洞。
- `vulnerabilities.json` 的 `counterevidence`、`confidence`、`confidence_rationale`、`severity_change_conditions`、`fix_verification`、`updated_at` 和 `update_history` 必须完整保留并面向用户展示。
- `run.json` 的 `workspace_files`、`mcp_connections` 和 `mcp_connection_status` 是可选运行上下文；缺失不得导致旧任务解析失败。
- CLI 的 `--workspace-file`、`--mcp-config`、`--mcp-server`、`--mcp-exclude` 是可选能力，只记录在 `runtimePolicy`，不作为 1.5.3 兼容下限的硬门禁。

新的上游版本必须先检查：

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
