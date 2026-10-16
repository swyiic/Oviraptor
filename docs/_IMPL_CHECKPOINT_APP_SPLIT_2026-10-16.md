# IMPL CHECKPOINT — App Split 2026-10-16

Updated: 2026-10-16 23:50 Asia/Shanghai

## 目标完成度

总体：「智能 agent 平台 + Asset/Nest 内置应用」— **约 92%**。

| 目标块 | 状态 | 说明 |
|---|---|---|
| 内置预装 Asset/Nest | ✅ | bundled-apps、shell catalog |
| scanning 薄入口 | ✅ | 先前完成 |
| agent P0 五件 | ✅ | 发现/错误重试/paired_skills/plan/不越权 |
| 会话记忆 | ✅ | host SQLite 有界摘要 |
| Tauri sidecar | ✅ 接线 | 本机需 rustc≥1.88 |
| paired_skills 安装预览 | ✅ | 保留 |
| HackerOne 应用化 | ❌ 已取消 | 按用户要求 |
| **A) agent 智能读项目/表** | ✅ 本轮 | discovery 增强 + assets 只读工具 |
| **B) agent↔应用耦合约束** | ✅ 本轮 | §5 文档 + 可测规则 R1–R8/R9 |

## Done this round — G7/G8/G9 xAI provider + 速度 + 智能

- 文档：`docs/AGENT_GROK_OAUTH_AND_SPEED_PLAN_2026-10-16.md`
- G7：新增 `packages/agent-host/src/provider.mjs`（`ProviderStore` 0600 存 key、`validateProviderConfig`）；runtime `xai-api`；`POST/DELETE /api/settings/provider`；UI 设置页 xAI 区块。**非 OAuth**：xAI 无公开第三方 OAuth 流程，官方为 API key；Grok CLI 登录（grok-acp）仍是官方 OAuth 路径。
- G8：`OpenAiCompatibleRuntime` 流式 SSE → text_delta；`runToolCalls` 无依赖并行/有依赖分波；`truncateMcpResult` 16KB 截断并提示再查。
- G9：`PLANNING_GUIDANCE` 每轮注入；`strategyHint` 接到 bridge 错误 `data.hint` 与循环 tool 结果。
- 测试：`test/provider-agent-loop.test.mjs` 8 项；agent-host 全量 **331 pass / 0 fail**；agent-ui+runtime-acp **108 pass**；build:agent-first + verify `accepted:true`（Node 24 via npx，box 默认 node 20 无 node:sqlite）。
- 缺：真 key e2e、长会话 messages 压缩、Responses API。

## Previous round — agent 数据读取 + 应用耦合（夜间自主）

### 文档
- `docs/AGENT_DATA_ACCESS_ANALYSIS_2026-10-16.md`：追加 **§5 agent↔应用关联约束与耦合度**（现状关联、耦合分级、目标边界、可测规则清单）；标「已实现」
- 本 checkpoint 更新

### P1 发现增强
- `packages/agent-host/src/discovery.mjs`：摘要为每个工具附 `purpose`（description 截断）+ `required` 必填参数名；导出 `MAX_SUMMARY_BYTES`；超限有序截断并标 `truncated`
- 单测：`packages/agent-host/test/discovery-summary.test.mjs`（字节上限与截断顺序）

### B 耦合可测规则
- 新测：`packages/agent-host/test/agent-coupling.test.mjs` 覆盖 R1–R8：
  - 未授予 → list 不含；call → 403/`host_tool_permission_denied`、不重试
  - grantRevision 变 → `host_tool_authority_changed`，须刷新
  - 未启用 → list/discovery 空
  - builtin/uninstallable 拒绝卸载
  - discovery 只暴露已装且已启用且已授权
  - A 不能以 B 工具名经 A 调用；B 无 grant 时 MCP 403
  - disclosure_runtime 不匹配则 capabilities 空

### P2 只读数据工具（JS 薄层，未改 Rust 内核）
- **apps/assets**（permission: `inventory.read`）：
  - `inventory_schema`：静态白名单表/列，不查 sqlite_master
  - `inventory_query`：白名单 filter + 分页 limit≤100 + cursor
  - `project_overview`：inventory 聚合 + legacy snapshot 可用性标志
- **apps/scanning**：澄清 `task_status` / `journal_read` 的 agent 发现文案（已有工具，只补 description）
- 相关测试：`apps/assets/test/inventory.test.mjs`、`stdio.test.mjs`

### Commands run（结果）
- `node --test --test-concurrency=1 packages/agent-host/test/*.test.mjs` → **323 pass / 0 fail**
- `node --test packages/agent-host/test/discovery-summary.test.mjs` → 3 pass
- `node --test packages/agent-host/test/agent-coupling.test.mjs` → 7 pass
- `node --test apps/assets/test/inventory.test.mjs stdio…`（排除缺 UI dist 的 native-package）→ **70 pass**
- `node --test apps/scanning/test/*.test.mjs` → **26 pass**
- `node tools/build_app_packages.mjs` → ok（含新 assets tools）
- `npm --prefix packages/agent-host run build` → Independent host built
- `node tools/verify_agent_first_build.mjs` → `accepted:true`，bundled apps=2，skills=36

### 新增 MCP tools 列表
| 应用 | 工具 | permission |
|---|---|---|
| org.oviraptor.assets | `inventory_schema` | inventory.read |
| org.oviraptor.assets | `inventory_query` | inventory.read |
| org.oviraptor.assets | `project_overview` | inventory.read |

（scanning 无新工具；`journal_read` / `task_status` 仅更新 description）

### Remaining / risks
- `apps/assets/test/native-package.test.mjs` 仍因本机缺少 `apps/assets/ui/dist/build-manifest.json` 失败（**与本轮无关的预存环境缺口**）
- 桌面侧需 rustc≥1.88 编 sidecar
- Phase 4 进程隔离未做
- 不做通用 SQL；不碰 Nest/homogeneous / exposure 扫描实现 / assets.rs 内核
- P3–P5（scan_runs_list、exposure 摘要、projects_list）未做
