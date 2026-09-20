# 原生多智能体重构总说明（不要直接作为单次编码任务执行）

> 当前可执行任务只有 `docs/QWEN_MULTI_AGENT_STAGE1A_TASK.md`。
> 请先完整完成并验收 Stage 1A，然后停止。未经复核，不得根据本文件直接连续实现 Stage 1 至 Stage 9。

请在 `/Users/swyiic/Desktop/Rust/Oviraptor` 中继续 Oviraptor（用户可见模块名 Nest）的原生
多智能体重构。不要根据旧聊天或文件名猜测当前实现，先检查当前工作树、实际代码和现有测试。

开始前必须完整阅读：

1. `docs/NATIVE_AGENT_RUNTIME_REMEDIATION_REQUIREMENTS.md`
2. `docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md`
3. `docs/NATIVE_AGENT_EXECUTOR_TASK.md`
4. `docs/NATIVE_MULTI_AGENT_REFACTOR_REQUIREMENTS.md`
5. `docs/NEST_AGENT_COLLABORATION_UI_REQUIREMENTS.md`

第 4 份文档是本轮多智能体实现与验收合同；Phase 2 可靠性不变量的优先级更高。不得只增加角色
枚举、prompt、trait 或多个相同循环后宣称完成。

## 执行要求

1. 先运行任务书 Stage 0 的基线命令，记录失败项；先修复 Phase 2 回归和 Clippy 警告。
2. 严格按 Stage 1 至 Stage 9 顺序推进。每个 Stage 都必须保持可编译、可测试和可通过 feature
   policy 回退；不得同时复制多套临时状态机。
3. 先写失败测试，再实现对应合同。每完成一个 Stage，立即运行该 Stage 的单元/集成测试。
4. Coordinator 是唯一全局队列、预算和终态写入者；它不能直接调用目标工具。
5. 专家只能由确定性证据谓词触发，只能领取结构化 Assignment、能力租约和预算租约；运行时不得按角色列表机械串行执行。
6. 每个 evidence revision 必须支持受控缺口协商：专家提交 GapProposal，其他专家结构化补充或质疑，Coordinator 根据收益、成本、风险、依赖和重叠度动态仲裁。Agent 不能互相直接下命令。
7. 三条 lane 必须是实际调度约束：一个目标触碰 Agent、一个只读分析 Agent、一个 Reviewer。
8. 共享证据图是唯一协作面；mailbox 不传原始响应、秘密或自由增长的聊天历史。
9. 同一合同只能有一个 owner。暂停、崩溃、租约到期和续跑都不能重复请求、重复计费或重复 review。
10. 执行 Agent 只能提交 CandidateFinding。只有独立 Evidence Reviewer 能返回
   `confirmed / rejected / insufficient_evidence`，只有 Coordinator 能把 confirmed review 投影为 Finding。
11. 所有角色共用现有 Tool Broker、scope gate、脱敏、artifact store、事件、snapshot 和 reducer；
    禁止平行复制。
12. 通用代码和 Nest UI 不再新增 `strix_*` 命名。Strix 只保留为兼容 adapter 和历史来源标识。
13. `agentBackendPolicy=native` 的所有新增 E2E 必须断言没有启动 Strix 进程。
14. 所有角色默认复用同一个 ModelGateway/Profile/API；每个 run 使用独立上下文和预算。
15. 自定义 Agent 只能使用兼容能力包；不能通过 prompt、工具名或沙箱配置自行扩权。
16. 实现可见、可审计的协作线程、Agent 工作室和人工干预入口；不展示私有思维链。

## 工作区保护

当前仓库有大量用户未提交修改和未跟踪的新 Rust 文件。禁止：

```text
git reset --hard
git checkout -- .
git restore .
git clean -fd
```

不要覆盖或格式化与当前 Stage 无关的文件，不要把大文件整份重写。发现并发修改时重新读取目标
文件，只做最小增量 patch；无法安全合并时停止并报告具体冲突。

## 完成前检查

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build
git diff --check
```

必须执行 `NATIVE_MULTI_AGENT_REFACTOR_REQUIREMENTS.md` 第 19 节的测试矩阵和 mock E2E，并记录
第 20 节指标。不得使用真实生产网站作为自动化验收目标。

最终回复严格按任务书第 23 节逐项报告。任何未完成内容必须写“未完成”，不得使用“基本完成”、
“已兼容”或“后续优化”掩盖。
