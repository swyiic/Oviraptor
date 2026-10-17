# 配置中心重构（2026-10-17）— 已落地

分支：`feature/agent-first-app-platform-20261016`。用户已批准整包；本文件记录落地结果。

## 四分区（App.vue）

| Key | 中文 | 内容 |
|---|---|---|
| `runtime-model` | 运行时与模型 | `AgentRuntimeSettingsPanel`（Grok OAuth 主路径）+ 指向高级里旧版模型的说明 |
| `apps` | 已装应用 | 侧栏内置插件 + agent-host MCP 应用启用/确保内置包 |
| `data-permissions` | 数据权限 | 按桌面工作区授权 `inventory.read` 等；披露确认；runtime 身份 |
| `advanced` | 高级 | 运行方案（扫描/旧版对话）、规则、Worker、运行环境、账号/代理提示 |

旧 key 别名（`configCenter.ts`）：`agent-runtime`→`runtime-model`，`plugins`→`apps`，`profiles|rules|workers|runtime`→`advanced`（并落到对应子页）。`managePlugins()` 仍可用。

## ConfigDialog

- 「模型与 Agent」改名为「扫描 / 旧版对话模型（高级）」，文案标明不用于 Grok 问答。
- 默认值抽出到 `configProfileDefaults.ts`，减小 `ConfigDialog.vue`。

## 项目映射（步骤 5）

- host `projects.external_id` + `ensureProjectByExternalId`
- `POST /api/projects` 支持 `ensureByExternalId`
- 会话页 `AgentHostChat` 用桌面 `project.id` 作 externalId

## 验收

1. 配置中心导航只有 4 项；默认落在「运行时与模型」。
2. 登录 Grok 后会话页标题仍是 xAI Grok；在「数据权限」给资产应用勾选 `inventory.read` 并保存后，问答可查资产。
3. 「高级 → 运行方案 → 编辑」仍可改 deepseek 等旧档案；不影响 OAuth 主路径。
4. 侧栏「管理内置插件」仍打开配置中心「已装应用」。

## 测试

- `packages/agent-host/test/config-center-and-external-id.test.mjs`
- 既有 `session-runtime-wiring` / phase-a
- `vue-tsc --noEmit` 无错
