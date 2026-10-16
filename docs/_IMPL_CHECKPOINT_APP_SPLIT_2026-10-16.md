# IMPL CHECKPOINT — App Split 2026-10-16

Updated: 2026-10-16 23:20 Asia/Shanghai

## 目标完成度

总体：「智能 agent 平台 + Asset/Nest 内置应用」— **约 85%**。

| 目标块 | 状态 | 说明 |
|---|---|---|
| 内置预装 Asset/Nest | ✅ | 20bff6f：bundled-apps、shell catalog、宿主导航默认 agent-host |
| scanning 薄入口 | ✅ | 先前完成（本轮未改业务内核） |
| agent P0 五件（发现/错误重试/paired_skills/plan/不越权） | ✅ | 20bff6f |
| **会话记忆** | ✅ | 本轮：host SQLite 有界 turn/tool/fact 摘要，跨轮注入 prompt，重载可恢复 |
| **Tauri sidecar** | ✅ 接线完成 | 本轮：spawn cli、`--session-token`、注入 origin/session、生命周期；本机 rustc 1.85 无法编 Tauri（需 ≥1.88），Node 契约测试已过 |
| paired_skills 安装预览 | ❌ 次优先未做 | 仅有契约 + 摘要提示 + skills_read 范围 |
| HackerOne 真正应用化 | ❌ 次优先未做 | 仍为 shell 回退项 |

## Done this round — 会话记忆 + Tauri sidecar

### 会话记忆
- 新表 `conversation_memory`（payload JSON：turns / facts）
- `packages/agent-host/src/memory.mjs`：有界规范化、recordTurn、pinFact、prompt 块
- `service.mjs`：每轮注入 `Host session memory`；工具事件写入 turn 摘要；turn 结束持久化
- 测试：`session-memory.test.mjs`（4）— 跨轮引用、SQLite 重载、边界裁剪

### Tauri sidecar（最少必要接线）
- `cli.mjs`：`--session-token`；stdout 打印 `Session: <hex>`
- `src-tauri/src/agent_host_sidecar.rs`：env 优先，否则 spawn Node cli；`agent_host_endpoint` 命令；Exit 时 kill
- `lib.rs` / `tauri.conf.json` CSP：允许 `http://127.0.0.1:*`
- `hostCatalog.ts`：`resolveHostCredentials` 经 invoke 回退；`createHostAppsClient` 异步
- 测试：`cli-sidecar-contract.test.mjs` — 打印 origin/session 且 `/api/status` 200

### Changed files
**packages/agent-host/**
- src/memory.mjs (new)
- src/store.mjs / service.mjs / cli.mjs
- test/session-memory.test.mjs (new)
- test/cli-sidecar-contract.test.mjs (new)
- test/authority.test.mjs（core tables 含 conversation_memory）

**src-tauri/**
- src/agent_host_sidecar.rs (new)
- src/lib.rs
- tauri.conf.json

**src/**
- plugins/hostCatalog.ts / plugins/index.ts / App.vue

### Commands run (node v24.11.0 via /home/box/node24)
- `node --test packages/agent-host/test/session-memory.test.mjs` → 4 pass
- `node --test --test-concurrency=1 packages/agent-host/test/*.test.mjs` → **307 pass**
- `node tools/verify_agent_first_build.mjs` → accepted:true
- `cargo test --lib agent_host_sidecar::tests` → **blocked**：rustc 1.85.1 < deps 要求 1.88（环境限制，非代码逻辑失败）

### Remaining / risks
- paired_skills 安装时 Skill 预览导入 UI/安装流
- HackerOne 应用化（manifest + bundled + shell）
- 桌面侧需在 rustc≥1.88 机器上编过 sidecar 实机联调
- Phase 4 进程隔离（Nest/Asset 不链进 Host）未做
- 不触碰暴露面扫描实现细节 / 攻击 payload
