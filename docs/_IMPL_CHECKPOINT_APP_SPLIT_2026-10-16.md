# IMPL CHECKPOINT — App Split 2026-10-16

Updated: 2026-10-16 23:20 Asia/Shanghai

## 目标完成度

总体：「智能 agent 平台 + Asset/Nest 内置应用」— **约 90%**。

| 目标块 | 状态 | 说明 |
|---|---|---|
| 内置预装 Asset/Nest | ✅ | 20bff6f：bundled-apps、shell catalog、宿主导航默认 agent-host |
| scanning 薄入口 | ✅ | 先前完成（本轮未改业务内核） |
| agent P0 五件（发现/错误重试/paired_skills/plan/不越权） | ✅ | 20bff6f |
| **会话记忆** | ✅ | host SQLite 有界 turn/tool/fact 摘要 |
| **Tauri sidecar** | ✅ 接线完成 | spawn cli、session-token；本机 rustc 1.85 无法编 Tauri（需 ≥1.88） |
| **paired_skills 安装预览** | ✅ 保留 | 2bb0788 / a2a1065：安装预览、导入、一键 skills_read；未回滚 |
| **HackerOne 应用化** | ❌ 已取消 | **按用户要求取消 HackerOne**：已 revert `3dc12d4`；不再预装、不再出现在 bundled |

## Done this round — 取消 HackerOne 应用化（保留 paired_skills）

### 按用户要求取消 HackerOne
- `git revert 3dc12d4`：删除 `apps/hackerone/`；从 `bundled-apps.json` 移除 `org.oviraptor.hackerone`
- `shell-apps.json`：`hostAppId` 恢复为 `null`（shell-only，与改前一致；不预装、不进 bundled）
- `shell-apps.mjs` 注释与 `bundled-apps` / `shell-apps` 测试恢复到无 host 包状态
- 本地 `.agent-build/apps/org.oviraptor.hackerone-1.0.0` 已清理（gitignore）
- **未改** `src-tauri` HackerOne 业务内核

### 保留：paired_skills 安装预览导入流
- `2bb0788` / `a2a1065` 全部保留
- host：预览、导入、includeInSkillsRead；卸载应用不删 Skill
- agent-ui：应用卡片 paired Skills 操作

### Changed files（本撤销轮）
**删除** `apps/hackerone/**`  
**packages/agent-host/** bundled-apps.json, shell-apps.json, src/shell-apps.mjs, test/bundled-apps.test.mjs, test/shell-apps.test.mjs  
**docs/** _IMPL_CHECKPOINT_APP_SPLIT_2026-10-16.md

### Commands run
- `git revert 3dc12d4`（再更新 checkpoint）
- `node --test packages/agent-host/test/paired-skills-install.test.mjs`
- `node --test --test-concurrency=1 packages/agent-host/test/*.test.mjs`（含 bundled / shell / verify 相关）
- `node tools/verify_agent_first_build.mjs`（若环境允许）

### Remaining / risks
- 桌面侧需在 rustc≥1.88 机器上编过 sidecar 实机联调
- Phase 4 进程隔离（Nest/Asset 不链进 Host）未做
- 不触碰暴露面扫描实现细节 / 攻击 payload
- HackerOne 保持既有 Tauri/shell-only；不再做 host 应用化
