# Agent UX 产品 backlog（agent-first）

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

**目标**：长列表能认出会话；长工具轮不把 UI 感知成「卡住」。

| 项 | 验收 | 备注 |
| --- | --- | --- |
| **自动标题** | 标题仍为默认「New conversation」时，用**首条用户消息**（轻量截断规则）写入 title，并 SSE `conversation.updated`；非默认标题不覆盖 | 不对齐桌面 `conversation_set_title` 工具面；不做隐藏二次模型调用。可选后续：模型一行标题 |
| **Heartbeat** | turn / 工具活跃期间，host 每 N 秒 emit `runtime.heartbeat`（`runId`、`elapsedMs`、`lastEventAt`、可选当前 step）；UI 显示「仍在运行 · Ns」 | 复用已有 SSE；不是独立 RPC。已有 `runtime.step` / plan 继续保留 |
| （可选小步） | `PATCH /api/conversations/:id` 手动改名 + 列表旁重命名 | 可跟 Wave1 或紧随其后；不阻塞自动标题 |

**非目标**：统一桌面 `ConversationToolActivity`；弱网自动重发消息。

---

## Wave 2 — Scratch / 空状态项目流

**目标**：降低「必须先有项目才能聊」的摩擦，但仍保持 **project 绑定** 数据模型。

| 项 | 验收 |
| --- | --- |
| 空状态一键 | UI：「快速项目 + 首会话」（仍先 `createProject`，减少步骤） |
| 可选 scratch | 产品开关：缺省 `POST /api/conversations` 时可自动 `createProject("Scratch …")` 并打标可归档 |
| （中期，可另开） | chat-first：`project_id` 可空草稿会话 → 用户确认后 `attachProject` / `createProject` |

**非目标**：本波不改 ACP cwd 隔离策略（见审计 §6）；不合并桌面与 host 的 `external_id` 身份叙事。

---

## Wave 3 — Legacy 只读桥 + 洞察写回

**目标**：Agent 能先看见桌面业务真相（只读），再在用户 grant 下写回结构化洞察。

| 项 | 验收 |
| --- | --- |
| **Legacy 只读桥** | 宿主托管 staged `legacy-input.sqlite3`（`legacyBridge`）；恢复公开只读 legacy 工具（如 stats/search），避免空库存误读 |
| **洞察写回** | project-scoped `insight_*`（新表、强 schema、新 permission）；禁止用 `inventory_import` 塞自由文本当洞察 |
| 文档边界 | Agent 分析边界 =「已 grant 的 app SQLite + 会话 memory」，不是整个项目磁盘 / 桌面主库直连 |

**非目标**：Scanning 执行进 app；任意 SQL / `sqlite_master`；Skills 当执行器写库。

---

## 建议排期顺序

1. Wave1 自动标题 + heartbeat（本迭代最小实现）
2. Wave1 可选手动 PATCH 重命名（若未同迭代落地）
3. Wave2 空状态 / scratch 开关
4. Wave3 legacy 桥 → insight 工具与 permission

对照审计 Top 5：本三波覆盖会话可辨识、工作中可见性、scratch 空状态、legacy 桥、洞察写回；**plugability 仍延期**。

---

*产品计划文档；实现以仓库 commit 为准。*
