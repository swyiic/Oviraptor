# IMPL CHECKPOINT — App Split 2026-10-16

Updated: 2026-10-16 22:20 Asia/Shanghai

## Done this round — Phase 3 P0「让 agent 变聪明」

### Completion criteria
- [x] P0.1 工具/应用发现：每轮 prompt 注入有界摘要（已授权应用+工具+已选 Skill）；toolsFingerprint 随 list 变化
- [x] P0.2 结构化工具错误 + 有界重试：mcp-bridge / AgentHost.mcp 透传错误码与 retryable；仅 revision 冲突与超时最多重试 2 次；不再压成单一 "Scoped application capability unavailable"
- [x] P0.3 manifest 可选 paired_skills（只引用 Skill）；摘要提示「调用 X 前先读 Skill Y」；paired Skill 可进入 skills_read 范围；无执行权限
- [x] P0.4 runtime.plan / runtime.step 规范化；agent-ui 显示 Plan progress
- [x] P0.5 fixture-authority 回归：未授权/停用工具不出现在 list；直接调用 403 不重试；撤销后进行中调用以 authority_changed 结束
- [x] 未 commit / push
- [x] 本轮改动均在白名单；黑名单 apps/scanning 等为先前脏树，未触碰

### Changed files (this P0 round)
**packages/agent-host/**
- src/discovery.mjs (new) — 有界 capability 摘要
- src/service.mjs — prompt 注入摘要；mcp 有界重试；plan/step 规范化；paired skill ids
- src/mcp-bridge.mjs — 结构化错误码 + 有界重试
- src/store.mjs — hostError retryable；mcpErrorPayload
- src/server.mjs — 错误体透传 retryable
- src/skills.mjs — execute 接受 pairedSkillIds 扩展 skills_read 范围
- test/agent-smartness.test.mjs (new) — 5 项 P0 回归

**packages/app-sdk/**
- src/manifest.mjs — paired_skills 校验（禁止可执行引用）
- test/manifest.test.mjs — paired_skills 用例

**packages/agent-ui/**
- src/controller.js — runtime.plan / runtime.step 状态
- src/main.js — Plan progress UI
- src/styles.css — plan-progress 样式
- test/core.test.mjs — plan/step 用例

**examples/fixture-authority/** (new) — 假应用夹具（ping/status + paired_skills）

### Commands run (node v24.11.0 via /home/box/node24)
- `node --test packages/agent-host/test/agent-smartness.test.mjs` → 9 pass
- `node --test packages/agent-host/test/*.test.mjs` → 302 pass
- `node --test packages/app-sdk/test/*.test.mjs packages/agent-ui/test/*.test.mjs` → 117 pass
- `node tools/build_app_packages.mjs` → ok（含 fixture-authority）
- `node tools/verify_agent_first_build.mjs` → accepted:true

### Not done / risks
- Phase 3「会话记忆」未做（规格标为 Phase 3 条目，但不在本轮 5 件必做内）
- paired_skills 的「安装时 Skill 预览导入」UI/安装流未做（仅契约 + 摘要提示 + skills_read 范围）
- 黑名单 apps/scanning 与 tools/test_conversation_plugins.cjs 等仍为工作区既有脏文件，本轮未改
- No commit/push（按硬禁止）
