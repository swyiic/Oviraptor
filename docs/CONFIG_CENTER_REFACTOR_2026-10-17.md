# 配置中心重构方案（2026-10-17，草案，待拍板）

分支：`feature/agent-first-app-platform-20261016`。本文只是方案；同一提交里的会话页修复见文末「已修」。

## 1. 现状问题

| 问题 | 证据 |
|---|---|
| 两套「模型」并存，彼此不知道 | 旧版 Rust 模型档案（`ConfigDialog.vue`「模型与 Agent」，例如用户的 `deepseek-v4-flash`），另有 agent-host 的 xAI Grok OAuth（`AgentRuntimeSettingsPanel.vue`，`App.vue` 里的 `configSection==='agent-runtime'`）|
| 会话页和登录状态脱节 | `ConversationWorkspace.vue` 的标题读 `snapshot.replyAvailability.model`，也就是旧档案；发送走的是 Tauri `send_conversation_message`，不经过 agent-host |
| sidecar 每次启动都把 runtime 重置为 none | `agent_host_sidecar.rs` 硬传 `--runtime none`，Grok 登录在重启后就丢了 |
| provider 设置分散 | 「账号与 API」里同时放了资产检索账号、漏洞平台 API 和 Clash 代理；模型档案、多 Agent 运行环境、本地模型策略放在一个 1239 行的 `ConfigDialog.vue` 里；OAuth 又在另一个面板 |
| OAuth 和 API key 混在一起 | Grok OAuth、xAI API key（高级）、旧档案的 api_key 分布在三处，文案也不统一 |
| 项目映射是隐式的 | 桌面工作区和 agent-host 的 projects 是两张表（本次先按名称映射）|

> 「中国移动历史资产」是用户自己的**工作区名**，「deepseek-v4-flash」是用户自己保存的**旧模型档案**。两者都不是硬编码默认值或 seed，所以都不删。代码里唯一的 deepseek 字面量在 `profile.rs` 的 `bounded_wire_options` 里，那是线协议适配，不是默认值。

## 2. 新格局（以 Agent runtime 为中心，4 个分区）

1. **运行时与模型**（首屏）：用 Grok 登录（OAuth 主路径）、当前 runtime 和模型、质量/速度档位、登录过期提示。xAI API key 和 Grok CLI/ACP 折叠进「高级」。旧模型档案改名为「扫描/旧版对话模型（高级）」，和问答主路径分开展示。
2. **已装应用**：资产、扫描等 app 的启用、升级、配对 Skills（现在散在侧栏和 agent-ui 里）。
3. **数据权限**：按项目授予 `inventory.read`、`investigation.read`、`inventory.exposure.read`，显示 runtime 接收方身份；会话页的「访问」面板链接到这里。
4. **高级**：Clash 代理和 net-proxy、资产检索 / 漏洞平台账号、本地模型资源策略、Hook 审计、规则源、多 Agent 运行环境。

## 3. 迁移步骤

1. （已做）sidecar 不再强制 `--runtime none`；会话页在 runtime 为 `xai-api` 时走 agent-host（`AgentHostChat.vue`），旧版对话保留一个入口。
2. 把 `ConfigDialog.vue` 拆成分区组件（只搬代码，行为不变），`App.vue` 的 `configSection` 改成上面 4 个 key，旧 key 做别名映射。
3. 「运行时与模型」首屏合并 `AgentRuntimeSettingsPanel` 和旧档案（旧档案降为高级）。
4. 数据权限页接 agent-host `/api/apps/:id/grants`，会话页访问面板链接过去。
5. 项目映射：agent-host 加 `externalId`（桌面 projectId）字段，替代按名称映射。

## 4. 向后兼容

- 旧模型档案、账号、规则数据原样保留，只调整展示位置。
- runtime 不是 `xai-api` 时（未登录或选了 none/ACP），会话页仍走旧版 Rust 对话，行为不变。
- 旧的 `configSection` key 保留别名，深链不失效。

## 5. 验收

- 登录 Grok → 会话页标题显示 `xAI Grok (OAuth) · grok-4.7 · <工作区>`；重启 app 后仍是这样。
- 退出登录 → 会话页回到旧版对话，标题显示旧档案模型。
- 4 个分区都能找到原有每一项设置；不需要做数据迁移。
- 单测：`session-runtime-wiring.test.mjs`；第 2–5 步各自补组件测。

## 6. 需要拍板

第 2–5 步属于较大的 UI 重构（大约要拆 `ConfigDialog.vue` 1.2k 行，并改 `App.vue` 导航）。建议拍板后单独一个 PR 做完。
