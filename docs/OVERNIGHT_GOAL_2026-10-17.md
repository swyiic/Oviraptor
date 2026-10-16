# 连夜目标（2026-10-16 晚 → 10-17 早检查）

分支 `feature/agent-first-app-platform-20261016`，PR #9。每项测过即推。

| # | 目标 | 验收标准 | 状态 |
|---|---|---|---|
| G1 | Asset / Nest 拆成内置应用（可安装像 skill） | bundled-apps 预装 assets+scanning；verify_agent_first_build accepted | ✅ 已推 |
| G2 | 宿主迁移（删 catalog.ts） | 前端走 host catalog，测试绿 | ✅ 已推 |
| G3 | agent P0（发现摘要/结构化错误/paired_skills/plan-step/越权测） | agent-host 测试全绿 | ✅ 已推 |
| G4 | 会话记忆 + Tauri sidecar | 合约测试绿（box 无法编 Rust，需 Mac 实机） | ✅ 已推（实机待验） |
| G5 | agent 智能读取项目/表数据 | inventory_schema / inventory_query / project_overview 可发现可调用，只读、白名单、绑定项目 | ✅ 01f5249 |
| G6 | agent↔应用关联约束与耦合度 | R1–R9 回归测：未授权不可见、403 不重试、authority 变更、跨应用拒绝、discovery 只露已装已授权 | ✅ 01f5249 |
| G7 | Grok OAuth / xAI provider 接入 | 有官方 OAuth 则接入；否则 xAI API key 安全存储 + provider 选项，文档说明原因 | ✅ 8da4ca5 |
| G8 | 速度接近 Grok Build | 无依赖 tool 并行、流式事件、长结果截断，均有单测 | ✅ 8da4ca5 |
| G9 | 智能对齐（规划/换策略） | 每轮 plan→step、失败换策略提示，有单测 | ✅ 8da4ca5 |
| G10 | 暴露面聚合只读摘要（可选） | 仅计数/严重度/状态，无 payload | 待做 |
| G11 | Phase 4 进程隔离（可选） | 方案 + 最小实现 | 待做 |

不做：HackerOne；不碰 Nest/homogeneous 内核、exposure 扫描实现、assets.rs 内核。
需你实机验：Mac 上 rustc≥1.88 编 sidecar、真实模型 key 端到端。
