# Agent UX 产品 backlog（agent-first）

> 2026-10-18：当前统一审计与实施状态见 [Agent 与设置重构计划](./AGENT_SETTINGS_REFACTOR_PLAN_2026-10-18.md)。本文的历史波次记录不代表当前完整验收；配置所有权和已复现阻塞以新计划及当前代码为准。

- **分支**：`feature/agent-first-app-platform-20261016`
- **来源**：`/workspace/oviraptor-agent-ux-gaps.md`（2026-10-17 审计）
- **范围**：`packages/agent-host` / `packages/agent-ui`；桌面轨仅作对照，本 backlog 不要求移植桌面 Chat 全套能力
- **原则**：小步可验收；不扩大 plugability / 信任模型；不引入跨 app RPC、自由市场签名或任意 FS 工具面

---

## 当前约束（仍延期）

以下能力**明确延期**，不进本三波交付：

| 主题 | 说明 |
| --- | --- |
| 真正 plug-and-play | 安装≠启用≠grant≠tools/list；壳 `capabilities` ≠ MCP 工具；无发布者签名 / OS 沙箱 |
| Scanning 完整执行进 app | Nest/费用/租约仍在桌面；agent 包保持草稿/日志薄壳 |
| 跨 app host/call | 仅有 draft handoff |
| iframe 代发工具 | 无 host API token |
| ACP 任意 FS / 终端 | 不广告 `fs` / `terminal` |
| 自由洞察表滥用 inventory | 需专用 permission + schema（见 Wave3） |

文档与 UI 可继续澄清「安装→启用→选项目→勾权限→刷新 tools/list」清单，但不算本 backlog 的功能波次。

---

## Wave 1 — 会话可辨识 + 工作中可见性（优先）

**状态**：**已完成** @ commit `acdb4ba`（`feat(agent-ux): Wave1 auto title + runtime heartbeat keepalive`）。

**目标**：长列表能认出会话；长工具轮不把 UI 感知成「卡住」。

| 项 | 验收 | 备注 |
| --- | --- | --- |
| **自动标题** | 标题仍为默认「New conversation」时，用**首条用户消息**（轻量截断规则）写入 title，并 SSE `conversation.updated`；非默认标题不覆盖 | 不对齐桌面 `conversation_set_title` 工具面；不做隐藏二次模型调用。可选后续：模型一行标题 |
| **Heartbeat** | turn / 工具活跃期间，host 每 N 秒 emit `runtime.heartbeat`（`runId`、`elapsedMs`、`lastEventAt`、可选当前 step）；UI 显示「仍在运行 · Ns」 | 复用已有 SSE；不是独立 RPC。已有 `runtime.step` / plan 继续保留 |
| （可选小步） | `PATCH /api/conversations/:id` 手动改名 + 列表旁重命名 | 可跟 Wave1 或紧随其后；不阻塞自动标题 |

**非目标**：统一桌面 `ConversationToolActivity`；弱网自动重发消息。

---

## Wave 2 — Scratch / 空状态项目流

**状态**：**最小垂直切片已完成** @ commit `2a2bae3`。仍保持 **project 绑定**；未做可空 `project_id` 草稿表。

**目标**：降低「必须先有项目才能聊」的摩擦，但仍保持 **project 绑定** 数据模型。

| 项 | 验收 | 状态 |
| --- | --- | --- |
| 空状态一键 | UI：「开始聊天」→ `POST /api/conversations` `{ scratch: true }`（自动 Scratch 项目 + 首会话） | **已做** |
| 可选 scratch | 产品开关 `scratchConversations`（默认开）：缺省 `projectId` 时可自动 `createScratchProject("Scratch · …")`，`kind=scratch` 可归档标记 | **已做** |
| Promote | UI/host：`POST /api/projects/:id/promote` 命名后 `kind=project`；scratch 横幅提示 | **已做** |
| （中期，可另开） | chat-first：`project_id` 可空草稿会话 → 用户确认后 `attachProject` / `createProject` | **未做**（剩余缺口） |

**剩余缺口（刻意未做）**：

- 可空 `project_id` / 独立草稿会话表与 `attachProject`
- Scratch 批量归档 / 清理 UI（仅有 `kind=scratch` 标记）
- ACP cwd 仍按 project，非 per-session（见审计 §6）
- 不合并桌面与 host 的 `external_id` 身份叙事

**非目标**：本波不改 ACP cwd 隔离策略（见审计 §6）；不合并桌面与 host 的 `external_id` 身份叙事。

---

## Wave 3 — Legacy 只读桥 + 洞察写回（含 teammate 契约）

**状态（Legacy）**：**宿主 staged 切片已落地**（见下方验收列）。Node stdio worker **不能**自注入 `legacyBridge`（无子进程权限）——完整 host→worker 运输仍延期，见 `LEGACY_BRIDGE_TRANSPORT_BLOCKER`。

**状态（洞察写回）**：**assets 工具 + `inventory_insights` 持久化已落地**；**agent-ui 确认条已落地**（`insight.proposal` SSE → diffSummary + Confirm/Cancel → commit `userConfirmed:true`）。宿主 `POST /api/projects/:id/insights/propose|commit` 写同一 `inventory.sqlite3`；无 UI 确认时，调用方仍须显式传 `userConfirmed: true`。

**目标**：Agent 能先看见桌面业务真相（只读），再在用户 grant 下写回结构化洞察；与 Asset / Nest teammate 的工具契约先定文档。

| 项 | 验收 | 状态 |
| --- | --- | --- |
| **Legacy 只读桥** | 宿主托管 staged `legacy-input.sqlite3`；仅当 staged **且** `oviraptor-app-worker`（或测试 adapter）可服务时，才把 `legacy_snapshot_stats` / `legacy_saved_scans` 加进 capabilities 并由宿主路由；缺任一条件不登记（避免空库存误读） | **已做（staged + host 路由）** |
| **洞察写回** | project-scoped `insight_*`（新表、强 schema、新 permission）；禁止用 `inventory_import` 塞自由文本当洞察 | **已做（含 UI 确认）**：assets 工具 `insight_propose`（`inventory.read`）/ `insight_commit`（`inventory.write` + `userConfirmed`）；落库 `inventory.sqlite3`；宿主 HTTP 同路径；propose 返回 `diffSummary` 并 emit `insight.proposal`；agent-ui 确认卡 Confirm→`POST .../insights/commit` `{userConfirmed:true}` / Cancel 仅关闭。commit 仍强制 `userConfirmed===true` |
| 文档边界 | Agent 分析边界 =「已 grant 的 app SQLite + 会话 memory」，不是整个项目磁盘 / 桌面主库直连 | 文档 |
| **Asset teammate 契约** | 见下方「Asset：insight propose/commit」 | **契约已文档化**；assets 模型工具 + SQLite 落库已挂；宿主 API 对齐 |
| **Nest teammate 契约** | 见下方「Nest：opaque draft / journal」 | **契约已文档化**；代码未做 |

**Legacy 落地说明（本切片）**：

- 模块：`packages/agent-host/src/legacy-bridge.mjs`（`stageLegacyInputSnapshot` / `probeLegacyBridge` / `createHostLegacyBridge`）
- `AppManager.stageLegacySnapshot` + capabilities/call 仅在 probe 通过时暴露 legacy 工具
- `createApplication({ legacyBridge })` 仍为受信嵌入缝；注入时同步把工具写进**返回的** manifest（stdio handler 对齐）
- **延期 blocker**：把 bridge 注入进已安装 Node worker 进程需新的 host↔worker RPC（或放开子进程权限，已否决）

**非目标**：Scanning 执行进 app；任意 SQL / `sqlite_master`；Skills 当执行器写库；把 ownership / auth 暴露为模型工具。

### Asset：insight propose / commit（teammate 契约；代码已落地）

两阶段写回，避免模型直接改库存：

| 工具 | 权限 / 参数 | 行为 |
| --- | --- | --- |
| `insight_propose` | `inventory.read` | 只读提议：产出带 `proposalId` 的草案（拟改字段 + base 版本），不落库 |
| `insight_commit` | `inventory.write`；必填 `userConfirmed`、`proposalId`、`operationId` | 用户确认后提交；绑定同一 `proposalId` / `operationId`，幂等可追溯 |

- **字段白名单**（仅允许这些键进入 propose/commit 载荷）：`tags` / `note` / `labels` / `classification`。白名单外一律 `field_denied`。
- **失败码**（契约枚举，宿主原样回传）：
  - `proposal_expired` — 提案超时或已作废
  - `mismatch` — `proposalId` / `operationId` 与宿主记录不一致
  - `base_stale` — 提案基于的 base 已被他人改写
  - `field_denied` — 请求字段不在白名单
  - `confirm_required` — 缺少或为假的 `userConfirmed`
- **UI 确认**：propose 成功后宿主 emit `insight.proposal`（含 `diffSummary`）；agent-ui 显示 Confirm/Cancel；Confirm 走宿主 commit 并传 `userConfirmed:true`（模型不得自行伪造确认）。
- **明确不做**：ownership / auth **不**作为模型可调用工具；鉴权与归属仍由宿主与桌面轨处理，agent 侧只见 propose/commit 与上述失败码。

### Nest：opaque draft / journal（teammate 契约，无代码）

Scanning / Nest 对 agent 保持薄壳；路径与进度语义对模型不透明：

| 约定 | 说明 |
| --- | --- |
| **路径对 agent 不透明** | 不向 agent 暴露宿主/桌面真实 FS 路径；工具面只用句柄与相对语义 |
| **draft handle** | `{ appId, hostBoundProjectId, taskId }` —— agent 持有的唯一草稿标识；不得推断或拼接绝对路径 |
| **`journal_read`** | 参数 `sinceSequence` + 固定 **event enum**（宿主定义的事件类型集合）；按序号增量读日志，agent 不发明事件名 |
| **草稿无假进度** | draft 阶段不得向 UI/模型伪造「执行中 / 百分比」类进度；真实进度仅来自 journal 事件 |
| **禁止绝对 `dataDir`** | 不得把绝对 `dataDir`（或等价根路径）下发给 agent；数据根仅宿主内部使用 |


---

## Wave 4 — 桌面 Session 模型切换 + AgentHostChat 操作面（文档已立）

**来源**：用户试用反馈 2026-10-17（Asia/Shanghai）— 桌面 Session 聊天路径。

**详细修复规格**：[`WAVE4_FIX_SPEC.md`](./WAVE4_FIX_SPEC.md)（根因 / 文件级改动 / 验收）。

**状态**：**A–E 最小切片已落地**（见 `WAVE4_FIX_SPEC.md`）；已知缺口：host 无历史截断 / 运行中注入 API，编辑重发=追加新消息，steer=取消后重发。

**背景（用户现象）**：

1. 模型切换器位置难看/错位；无 VPN 时 Grok 失败；切到 DeepSeek 后会话仍不可用。
2. 已发送消息缺少：重新编辑、复制、steer（纠偏）。
3. 此前产品诉求几乎感觉没落地到桌面 Session。

**根因（代码已核对，写入 backlog 以免再混）**：

| 问题 | 根因 |
| --- | --- |
| 模型切换器位置 + 切 DeepSeek 无效 | 桌面 Session 页在 agent-host runtime 为 `xai-api` 时走 `AgentHostChat`（`ConversationWorkspace.vue`：`useAgentHostChat` / `usesAgentHostRuntime`）。该路径只连 **agent-host Grok/xAI**。DeepSeek / `modelProfiles` 在 ConfigDialog「**扫描 / 旧版对话模型（高级）**」——仅服务扫描与旧版对话；切换它们**不会**改 AgentHostChat runtime。文件内注释写明：session 跟随 agent-host，而非 legacy deepseek 默认。 |
| UI 语义混淆 | `ConfigRuntimeModelSection` 同时展示 `AgentRuntimeSettingsPanel` + 指向 legacy 模型的链接；「会话模型」与「扫描/旧版模型」在同一配置面被混读。 |
| 缺 copy / 重编 / steer | `AgentHostChat.vue` 是薄壳：消息为 plain `<article>`，无 `MessageCopyButton`、无重编、无 steer。旧版 `ConversationWorkspace` 有 copy（`ConversationMessage.vue`）与 reply 期间 composer steer（`replyActive`）。 |
| DeepSeek 非一等会话选项 | `agent-host` `provider.mjs` 已有 `openai-compatible` PROVIDERS 与 `OpenAiCompatibleRuntime`；桌面 `AgentRuntimeSettingsPanel` / agent-ui `settings.js` 主要暴露 `xai` / `grok-acp` / `none` —— DeepSeek **不是**一等 session 切换项。 |
| Wave1–3 ≠ 桌面 Session chrome | Wave1–3 交付的是 agent-host：自动标题、heartbeat、scratch、legacy 桥 staged、insight propose/commit+确认 UI。它们**不等于**桌面 `AgentHostChat` 的 copy/edit/steer，也不等于多 provider 会话切换。Plugability / Nest 执行仍延期。 |

### 验收标准（未来改码；小步可验收）

| 项 | 验收 |
| --- | --- |
| **A) Session 模型 UI** | 会话模型控件**只**改 agent-host runtime；legacy / `modelProfiles` 明确标注为「仅扫描 / 旧版对话」。支持把 **openai-compatible（含 DeepSeek）** 作为真实聊天路径：可配 `baseUrl` + `apiKey`（及 model），切过去后 Session 实际走该 runtime，而非仍钉在 xAI。 |
| **B) AgentHostChat 操作面** | 消息可 **复制**；可对**最后一条用户消息**（或选中消息）**重新编辑**再发；**steer** 在 host API 允许范围内对齐旧版 composer 语义（`replyActive` 期间纠偏）。若 host 缺少对应 API，在实现 PR 中显式记下缺口，不做假 UI。 |
| **C) 已有 host 信号上屏** | 若桌面 `AgentHostChat` 尚未展示 Wave1–3 已在 host 侧存在的信号（如自动标题更新、`runtime.heartbeat`「仍在运行」），则补齐展示；已有则跳过。 |

### 明确非目标（本波不做）

- 完整 Nest plug-and-play / Scanning 执行进 app
- 强迫用户开 VPN 才能用 Grok（应提供可工作的 openai-compatible 路径）
- 把桌面 Chat 全套能力一次性移植进 AgentHostChat
- 扩大 plugability / 信任模型 / 跨 app RPC（仍见文首「当前约束」）

## 建议排期顺序

1. ~~Wave1 自动标题 + heartbeat~~ — **已落地** `acdb4ba`
2. Wave1 可选手动 PATCH 重命名（若未同迭代落地）
3. ~~Wave2 空状态 / scratch 开关~~ — **最小切片已落地** `2a2bae3`（scratch + promote；可空 `project_id` 仍延期）
4. ~~Wave3 legacy 桥 + insight 工具~~ — legacy staged/host 路由已落地；insight propose/commit（assets 工具 + `inventory_insights` 落库 + `userConfirmed` + agent-ui 确认卡）已落地
5. **Wave4** 桌面 Session 模型切换语义澄清 + AgentHostChat copy/重编/steer（及 openai-compatible 真实聊天路径）— **文档已立，改码未开始**

对照审计 Top 5：Wave1–3 覆盖会话可辨识、工作中可见性、scratch 空状态、legacy 桥、洞察写回；Wave4 对准桌面 Session 轨的模型切换与聊天 chrome；**plugability / Nest 执行仍延期**。

---

*产品计划文档；实现以仓库 commit 为准。*
