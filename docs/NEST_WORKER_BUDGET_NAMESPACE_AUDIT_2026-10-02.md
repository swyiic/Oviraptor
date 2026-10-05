# Worker 账本隔离审计（2026-10-02，开发中）

Master 尚未完成。此项只补同一逻辑 assignment 的多个原 worker 财务作用域；不构成生产重派、Supervisor、强杀或整体功能验收。

## 问题证据与作用域

- 起点 HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`；上一批 841 代码/测试/配置路径摘要为 `43e884f85684504979dd0944b42f4bae679b3bd901adc0403eda30c935893e19`。已有改动保留，逐文件保存修改前文本，空 git diff 的 untracked Native 模块没有当作干净文件。
- 五项先实际运行：`/tmp/oviraptor-worker-budget-red.log`，**0/5、exit 101**。第二 worker 的预留报 `budget_entry_replay_conflict`；结算减去旧已知账单报 `budget_usage_below_recorded_cost`；并发槽释放误读两份报 `budget_concurrency_reservation_conflict`；未发出释放与未知转未决误取总余额报 `budget_transition_exceeds_balance`。
- 夹具只在临时库恢复两个不同 worker 财务上下文：原 worker 经生产原子过期 API 撤权，新行用于预算存储合同，不签发完整能力或执行重派。不能把夹具当成合法重派流程，更不能据此解除未知费用阻断。
- 本批不接触真实 DB、业务资产、CAS、安装 App、外部 URL，也不提交/重置/推送。账本无 schema 升级，无历史键或费用改写。

## 最小修改

- 所有账本追加先验证 `(root,assignment,attempt)` 的原序号。序号 1 保留当前 Native 键；后续序号保存 `worker:{attemptUUID}:` 前缀；已保存的自身键允许精确重放，外部 worker 前缀拒绝。此函数只做存储 namespace，调用者的执行或原费用证明仍独立核验。
- 当前模型结算、未发出释放、未知转未决、槽位释放读取精确 attempt 余额；结算与回执历史同时限定 attempt 和 prefix。原回执初始 token/HTTP dispatch 也查原 worker 的保存键。
- 当前未发出/结算的 Source、专家与 Web journal 按精确 child 读取。旧 worker 的已知调用不冒充当前发送事实；旧费用和预留逐行保持，Root 总账仍累加所有 worker，任一未知费用继续阻断新预留。
- 原金额/种类/source/attempt 冲突检测、账本不可变 guard、Root 硬限额和最终写后 owner 核验保留；没有续期、退款、费用转移、重发或旧 Coordinator 接管财务的旁路。

## 当前验证

- 最小修改后最初五项 **5/5、exit 0**，日志 `/tmp/oviraptor-worker-budget-green-first.log`。
- 增补六项后 **11/11、exit 0**，日志 `/tmp/oviraptor-worker-budget-negative.log`：同 call_source 的不同 worker 回执不合并、幂等回放/变更 receipt 拒绝；第二 worker expired 原费用读取自身初始配额；旧已知调用不阻止当前未发出释放；旧未知费用继续阻断；外部 key 和最后 budget 写入切换 child 回滚；第二 worker 原 HTTP dispatch 可精确结账且未知 dispatch 拒绝。
- 最后格式后的扩大回归 **208/208、exit 0（558.89 秒）**，日志 `/tmp/oviraptor-worker-budget-affected.log`；严格全目标全特性 Clippy exit 0（13.09 秒）；作用域 fmt/差异空白检查及 844 路径最终复核通过。未新跑导入测试或最终全量，不沿用前批门禁。
- 最后 844 代码路径运行快照 `/tmp/oviraptor-worker-budget-code-snapshot.json`，摘要 `88e47a8b5e3d94a6b94a101338912a3240701e9684d49d44e4a76f81f0ca10e2`。基线 `/tmp/oviraptor-worker-budget-baseline.json`，修改前文本 `/tmp/oviraptor-worker-budget-before.json`，逐文件增量 `/tmp/oviraptor-worker-budget-scoped.diff`，scope 清单 `/tmp/oviraptor-worker-budget-scoped-files.json`。最终 844 路径与扩大回归开跑时一致。

## 文件体量

共 9 个已有代码文件改变、3 个新增，没有移除原测试。仅以下 12 个文件改变；各手写文件都在 400 行以内。

| 文件 | 当前行数 |
|---|---:|
| `src-tauri/src/agent_runtime/multi_agent/budget.rs` | 353 |
| `src-tauri/src/agent_runtime/multi_agent/budget/entries.rs` | 92 |
| `src-tauri/src/agent_runtime/multi_agent/budget/model.rs` | 318 |
| `src-tauri/src/agent_runtime/multi_agent/budget/limits.rs` | 185 |
| `src-tauri/src/agent_runtime/multi_agent/budget/historical.rs` | 136 |
| `src-tauri/src/agent_runtime/multi_agent/budget/receipts.rs` | 243 |
| `src-tauri/src/agent_runtime/multi_agent/budget/receipts/write.rs` | 166 |
| `src-tauri/src/agent_runtime/multi_agent/budget/receipts/owner.rs` | 142 |
| `src-tauri/src/commands/agent_tests_multi_agent.rs` | 48 |
| `src-tauri/src/agent_runtime/multi_agent/budget/scope.rs` | 47 |
| `src-tauri/src/commands/multi_agent/tests/worker_budget.rs` | 236 |
| `src-tauri/src/commands/multi_agent/tests/worker_budget_receipts.rs` | 260 |

## 未完成与风险

- 生产安全重派需新 UUID/worker/fence/ordinal、完整 lane/contract/能力/原预算退出证明及真实执行。当前 scheduler 仍只能首次领取，旧 Source controller 的全局计数及 canonical Reviewer 恢复也待补。
- Source 工具 transport 仍用失去 typed BeforeTransport 的 complete_once；预算 Root/Single、动态 grant/精确对账不完整。
- 后台 Supervisor/真实进程强杀、全角色/模式、复杂用户聊天与工具闭环、AST/浏览器/进程逐路实时日志、精确旧数据盘点备份清理及最终全门禁/安装态/两授权 URL 均未完成。Goal 旧 blocked 工具记录不表示项目完成；继续已授权开发。
