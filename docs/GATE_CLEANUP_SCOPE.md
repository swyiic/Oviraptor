# 门禁清理范围清单（待你确认）

> 你的指示：**「除了高风险操作的强制门禁，其他的门禁都去掉。」**
>
> 这份清单把扫描/agent 链路里现有的每一类「门禁」逐一列出并分类，目的是把你的意图
> （让 agent 完全放开去干活）落实成一个**安全**的具体方案，而不是按字面把安全线一起删掉。
> **本清单只做分类与建议，未改任何代码**，等你确认后我再动手。

---

## 核心结论（先看这个）

扫描跑不动、agent 束手束脚，**真正的原因不是这些「门禁」拦得太多**，而是：

1. **高危能力本身还是惰性的**（Track C 没接线）——authorization 表、per-action 批准、6 个能力类的
   控制面都在，但没有任何工具真正去执行 RCE/写目标/外带/WAF 绕过。所以"放开"= **接线激活这些能力**
   （在你刚做的强制开关背后），不是删门禁。
2. **按角色/泳道的工具白名单**把单主脑能用的工具卡窄了——这才是可以放宽的那部分。

而你反复强调过的硬边界（SSRF 确定性判定、硬上限、WAF/CAPTCHA、结算/fencing、作用域合法授权），
**删掉它们不会让扫描更强，只会让这个工具变成"越权攻击任意主机 / 伪造结论"的东西**——那不是你要的。

所以下面分三桶。**C 桶才是真正要动的**；A/B 桶保留。

---

## A 桶 · 必须保留：硬安全线 / 合法授权边界（删 = 违法或破坏正确性）

| 门禁 | 代码位置 | 为什么必须留 |
|---|---|---|
| **SSRF 硬守卫**（`ssrf_internal_denied`）| `agent_tools_scope.rs:45`、`host_is_internal_literal:180`、DNS-pin `agent_pin_public_address` | 内网/环回/链路本地/元数据地址**确定性**拒绝，永不交给 agent 判断（目标可注入诱导）。内网测试应当**由操作者签名授权**后开放（见 C-2），而不是拆掉守卫。 |
| **作用域/授权域边界**（`scope_denied`、`comparison_origin_denied`）| `agent_tools_scope.rs:84`、`agent_tools_identity_diff.rs:4` | 冻结的 `allowed_origins` 就是"你被授权测的目标"。删掉 = agent 可打任意主机 = 越权违法。这正是你 UI 里"合法授权"勾选所声明的范围。 |
| **硬预算上限**（token / 请求 / 墙钟；`seed_budget_exhausted`、`hard_request_budget`、`root_http_budget_*`）| `agent_native/policy.rs`、`budget/*` | 钱包与失控保护。你自己把硬上限列为硬边界。 |
| **WAF/CAPTCHA 检测**（`confirmed_waf_or_challenge`）| `coordinator_protection_facts.rs:169` | 检测要留。**但**"撞到就停" vs "智能绕过"这一步行为，挂到 C-3 的 `waf_evasion` 高危类后面。CAPTCHA/人机验证**永不绕过**（硬禁止）。 |
| **Fencing / 能力租约 / 监督**（`tool_capability_or_fencing_denied`、`worker_supervision_denied`、`root_tick_frozen_mode_denied`）| `agent_tools_dispatch.rs`、`supervisor` | 多 agent 隔离与一致性不变量，删掉会直接腐蚀运行时状态。 |
| **证据/快照/回执**（`*_snapshot_required`、`*_receipt_*`、`contract_unknown`）| 遍布 `agent_tools_*`、`agent_native/*` | 保证每条结论都绑定真实请求/响应回执。删掉 = agent 可"凭空拼结论"（伪造漏洞）。 |
| **身份绑定 / 结算完整性**（`tool_identity_binding_denied`、`child_budget_reservation_*`）| `agent_tools_*`、`budget/*` | 正确性/审计不变量。 |

## B 桶 · 必须保留：高危强制门禁（这是你要留的那一个）

| 门禁 | 代码位置 | 说明 |
|---|---|---|
| **能力授权表 + 不可变**（`scan_capability_authorizations`）| `commands/scan_capability_authorization.rs` | 刚做的 UI 写入的那张表，默认全关、要合法授权、不可改。保留。 |
| **调度期类门禁**（`scan_class_authorized`）| `scheduler/child_contract.rs` | 未授权的高危类在调度处就拒。保留。 |
| **逐动作人工批准**（per-action 批准链）| `high_risk_action_approval.rs`、`controller_phase_lease.rs` | 即便类已授权，真实高危动作仍逐次人工放行。保留。 |
| **写接口合约**（`mutation_requires_contract` / `mutation_contract_required`）| `agent_tools_discovery.rs:19`、`agent_tools_browser.rs:36`、`agent_tools_replay.rs:193` | 这其实就是 `target_write` 的执行机制：保留，并明确"写"只有在 `target_write` 已授权 + 合约 + 逐次批准时才放行。 |

## C 桶 · 真正要动的：放开 / 激活（在强制开关背后）

| 项 | 现状 | 建议动作 |
|---|---|---|
| **C-1 按角色/泳道的工具白名单**（`tool_policy_or_role_denied`、`tool_execution_surface_denied`）| 按角色把可用工具卡窄 | **放宽**：让单主脑能驱动全部**非高危**工具；高危工具仍走 B 桶门禁。这是"agent 全部控制"的落点。 |
| **C-2 内网作用域**（`agent_internal_scope_authorized` 硬编码 `false`）| 所有内网目标一律拒 | **改成操作者签名授权的高危类**（新 `internal_scope` 类或并入 `egress`）。这样"SSRF 看到内网、尝试不同协议深入"**由开关授权后确定性开放**——满足你的需求，又不拆 SSRF 守卫。 |
| **C-3 高危能力接线（Track C）**| 6 个能力类惰性，无工具真正执行 | **接线激活**：RCE 无害命令白名单（id/whoami/ifconfig）自动、其余逐次批准；出网/OOB（可配自备 DNSlog）；WAF 智能多次绕过；写/持久化/外带的证明即止、可回滚。全部在授权 + 逐次批准背后。**这才是"完全放开"的主体工作。** |
| **C-4 `premature_finish_target`** | 过早收尾被拒 | **建议保留**（它是逼 agent 把 7 个族跑全的"反早退"推力，利于"扫描跑完"，不是能力限制）。如你坚持也可降级为告警。 |

---

## 我不会在没有你单独明确决定时去碰的（即便字面属于"其他门禁"）

- SSRF 确定性守卫、DNS-pin（A 桶）——只通过 C-2 的"签名授权"开放内网，不拆守卫。
- 作用域/授权域边界（A 桶）——这是合法授权的技术边界。
- 硬上限、fencing、结算、快照/回执（A 桶）——删了=失控/伪造结论。
- CAPTCHA/人机验证绕过——硬禁止，永不做。

## 建议的落地顺序（确认后）

1. **C-1**：放宽角色/泳道工具白名单（小、低风险，立刻让单主脑更放得开）。
2. **C-2**：把内网作用域接到"签名授权 + 高危类"，UI 加一个开关项。
3. **C-3**：逐个能力接线（RCE 白名单 → 出网/DNSlog → WAF 绕过 → 写/持久化/外带），每个都在强制开关 + 逐次批准背后，单测 + 真机验证。

> 请你在 A/B/C 的分类上拍板，尤其确认：**C-2 内网走"签名授权"而非拆 SSRF 守卫**、**C-4 保留**、
> 以及 A 桶整体保留。确认后我从 C-1 开始做。
