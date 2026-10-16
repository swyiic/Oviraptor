# IMPL CHECKPOINT — App Split 2026-10-16

Updated: 2026-10-16 23:45 Asia/Shanghai

## 目标完成度

总体：「智能 agent 平台 + Asset/Nest 内置应用」— **约 90%**。

| 目标块 | 状态 | 说明 |
|---|---|---|
| 内置预装 Asset/Nest | ✅ | 20bff6f：bundled-apps、shell catalog、宿主导航默认 agent-host |
| scanning 薄入口 | ✅ | 先前完成（本轮未改业务内核） |
| agent P0 五件（发现/错误重试/paired_skills/plan/不越权） | ✅ | 20bff6f |
| **会话记忆** | ✅ | host SQLite 有界 turn/tool/fact 摘要 |
| **Tauri sidecar** | ✅ 接线完成 | spawn cli、session-token；本机 rustc 1.85 无法编 Tauri（需 ≥1.88） |
| **paired_skills 安装预览** | ✅ 本轮 | 安装/启用返回预览；导入 library；一键纳入 skills_read；卸载不删 Skill |
| **HackerOne 应用化** | ✅ 最小可安装壳 | manifest + 只读工具 + 页面占位 + shell/bundled；未改 src-tauri 业务内核 |

## Done this round — paired_skills 安装预览 + HackerOne 最小壳

### paired_skills 安装预览导入流
- `packages/agent-host/src/paired-skills.mjs`：预览（excerpt/digest/status）与导入（package-relative → Skill library）
- 安装 / 启用 HTTP 响应附带 `pairedSkillsPreview`；`GET/POST .../paired-skills[/import]`
- 一键 `includeInSkillsRead`：导入后写入会话 selectedSkillIds（仍无执行权）；卸载应用不删 Skill
- agent-ui：应用卡片展示 paired Skills，支持 Refresh / Import / Include in skills_read
- 测试：`paired-skills-install.test.mjs`（6）+ agent-ui panels（2）

### HackerOne 最小可安装壳
- `apps/hackerone/`：仿 echo 形态；工具 `programs_list` / `program_get`（本地空缓存只读）；无攻击类能力
- `paired_skills` → `docs/operator-guide.md`
- `shell-apps.json`：`hostAppId=org.oviraptor.hackerone`（未安装时 status=`missing`）
- `bundled-apps.json`：可选预装 `enable:false`（不自动启用，可卸载）
- `build_app_packages` 已产出 `org.oviraptor.hackerone-1.0.0`
- **未改** `src-tauri` HackerOne 业务内核；桌面看板仍由既有 shell/Tauri 门禁持有

### Changed files (本轮)
**packages/agent-host/** paired-skills.mjs, apps.mjs, service.mjs, server.mjs, shell-apps.mjs, shell-apps.json, bundled-apps.json, test/paired-skills-install.test.mjs, test/shell-apps.test.mjs, test/bundled-apps.test.mjs  
**packages/agent-ui/** paired-skills.js, main.js, test/panels.test.mjs (+ dist rebuild)  
**apps/hackerone/** 新包  
**docs/** _IMPL_CHECKPOINT_APP_SPLIT_2026-10-16.md

### Commands run (node v24.11.0 via /home/box/node24)
- `node tools/build_app_packages.mjs` → 含 hackerone
- `node --test packages/agent-host/test/paired-skills-install.test.mjs` → 6 pass
- `node --test --test-concurrency=1 packages/agent-host/test/*.test.mjs` → **313 pass**
- `node --test packages/agent-ui/test/*.test.mjs` → **76 pass**
- `node tools/verify_agent_first_build.mjs` → accepted:true

### Remaining / risks
- HackerOne 桌面看板 ↔ host app 本地缓存的受控桥接（同步/凭证仍在 Tauri；本壳不接攻击面）
- 桌面侧需在 rustc≥1.88 机器上编过 sidecar 实机联调
- Phase 4 进程隔离（Nest/Asset 不链进 Host）未做
- 不触碰暴露面扫描实现细节 / 攻击 payload
