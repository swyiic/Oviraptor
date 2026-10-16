# Claude 任务话术：打磨 Oviraptor Agent 平台（2026-10-16）

复制下方代码块，整段粘贴给 Claude。

```text
你在 Oviraptor 仓库工作，分支 feature/agent-first-app-platform-20261016。
必读规格：docs/AGENT_PLATFORM_APP_SPLIT_SPEC_2026-10-16.md（第 B、C、D.Phase3 节）。

## 工作区（同仓白名单模式）
只允许读写：
- packages/agent-host/**、packages/agent-ui/**、packages/runtime-acp/**
- packages/skills-library/**（平台代码）、packages/app-sdk/**、packages/native-app-sdk/**（SDK 契约）
- examples/echo-app/**、新建 examples/fixture-*/**（假应用夹具）
- tools/build_app_packages.mjs、tools/verify_agent_first_build.mjs、tools/test_skills_library_resources.cjs
- src/plugins/**、src/shell/**、src/features/conversations/**、src/features/skills/**、src/App.vue（仅导航相关）

禁止打开或修改（黑名单）：
- apps/assets/**、apps/scanning/**
- src/features/assets/**、src/features/sentinel/**、src/features/hackerone/**
- src-tauri/src/** 全部（含 agent_runtime/、assets/、scanning/、native_pipeline/、tool_supply/、commands/、db*.rs、bin/）
- docs/NEST_*、docs/agent-first-20261016/**
测试涉及应用时只用 echo-app 或你新建的 fixture 应用。

## 本轮目标：让 agent 更聪明（P0）
已核实的现状：
- packages/agent-host/src/service.mjs 的 prompt 只拼接项目 JSON 和一句通用安全提示，没有告诉模型当前装了哪些应用、有哪些工具、选了哪些 Skill。
- service.mjs 的 mcp() 会把应用工具名里的 '::' 换成 '__' 后通过 tools/list 暴露，但只返回 name/description/inputSchema，没有应用上下文。
- packages/agent-host/src/mcp-bridge.mjs 把所有调用失败都压成同一个错误 "Scoped application capability unavailable"（-32000）。模型分不清是没授权、revision 冲突、应用停用还是超时，因此无法恢复。
- Skill 只能由用户在会话里显式选择（skills.capture/skills_list/skills_read）。应用和 Skill 之间没有关联，agent 不知道某个工具该看哪份 Skill。
- 对话没有可见的计划或步骤事件，也没有 host 侧的会话记忆。

P0 任务：
1. 工具/应用发现：每轮 prompt 前由 host 注入一段有界摘要，内容是已启用且已授权的应用、它们的工具（限定名和一行用途）、已选 Skill 标题。摘要按不可信数据包裹。tools/list_changed 之后，下一轮摘要要随之更新。
2. 结构化工具错误与有界重试：AgentHost.mcp 和 mcp-bridge 透传结构化错误码，包括 host_tool_permission_denied、host_grant_revision_conflict、host_tool_authority_changed、timeout，并标注 retryable。只有 revision 冲突和超时可以重试，最多 2 次，带退避。权限拒绝不重试，改为向用户说明需要授权。
3. 应用↔Skill 关联：在 app-sdk manifest 校验中加入可选字段 paired_skills（只引用 Skill，禁止可执行内容）。工具发现摘要里标出「调用 X 前先读 Skill Y」，并让配对 Skill 能进入会话的 skills_read 范围。不能给 Skill 增加任何权限。
4. 多步计划可见：如果运行时发出计划或工具调用事件，host 把它们规范成 runtime.plan / runtime.step 事件，并在 agent-ui 中显示步骤进度。
5. 不越权的回归测试：用 fixture 应用验证三件事：未授权或已停用应用的工具不出现在 tools/list；直接调用返回 403 且不会重试；grant 撤销后，进行中的调用以 authority_changed 结束。

## 硬禁止
- 不得打开或修改黑名单路径。
- 不得编写任何扫描、探测、漏洞、攻击载荷、暴露面相关的代码、示例、夹具或文档。
- 不得修改 Nest、同质主管或多智能体内核。
- 不得把可执行逻辑放进 Skill。
- 不得 commit 或 push，除非用户另行明确要求。

## 完成定义
- 以上 5 项都实现了，并且各自有测试（放在 packages/agent-host/test 或 packages/app-sdk 测试中）。
- node --test packages/agent-host/test、node tools/build_app_packages.mjs、node tools/verify_agent_first_build.mjs 全部通过。
- git diff --name-only 中只有白名单路径。

## 回报格式
1. 改动文件列表（git diff --name-only）
2. 每项 P0：做了什么，对应的测试名
3. 测试命令和结果
4. 未完成项或风险
5. 确认未触碰黑名单
```
