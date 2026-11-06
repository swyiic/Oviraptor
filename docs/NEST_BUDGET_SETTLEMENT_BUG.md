# Nest 同质 worker 预算结算失败：根因与修复方案

> 现场:对 `hjrich.cn` 扫描,web 分支失败
> `multi_agent_bootstrap_failed:child_budget_settlement_exceeded_or_stale;readonly_local_recovery:child_budget_settlement_exceeded_or_stale`
> 真实数据取自 scan `77492a83-…`、root `plan-77492a83-…-1-f9c5f8a6119d`、assignment `asg-ab3aef3c45…`。

## 根因(已用真实账目确证)

1. SpaApiMapper 子任务发起了 1 次模型调用,但**模型输出被判 `schema_invalid` 拒绝**。
2. 拒绝后,**receipt-forfeit 路径**(`budget/receipts.rs::forfeit_original_call_estimated`)把三条 token 维度的预留**作废(forfeit)**、并 `consume` 了 1 次 `model_requests`。证据(`agent_budget_entries`):
   - `model_input_tokens`: reserve 8000 → forfeit 8000(source 含 `received:specialist:…:receipt:…`)
   - `model_cached_tokens`、`model_output_tokens`: 同上
   - `model_requests`: reserve 1 → consume 1
3. 子任务进入 `paused`、`agent_runs.used_tokens=0`、`agent_assignments.budget_settled_at=''`。
4. bootstrap(`multi_agent_prepare_owned`)/只读本地恢复**又对这个已被 forfeit 的子任务做一次用量结算** `settle_child_usage`。因为 `budget_settled_at` 为空,绕过了重放保护(`child_accounting.rs:87`),走进 `budget::model::settle` 的逐维度主循环:
   - 对已 forfeit 的 token 维度:`own.reserved=0`,但 `amount`=实际用量(>0),`required = usage.total_tokens - held + reserved`(incomplete 分支)> `reserved(0)`;该 root 每个维度都有 `hard_limit`(model_* = 900000 等)→ `model.rs:201-205` 直接 `budget_usage_exceeds_reservation` → 被映射成 `child_budget_settlement_exceeded_or_stale`。

## 更深一层的 bug:双账本不同步

`agent_budget_entries`(逐维度)里 token 已 forfeit,但聚合账本 `agent_budget_ledger` **仍是 `reserved_tokens=8000`**——forfeit 路径更新了逐维度账目却**没有同步释放聚合账本的预留**。两套账本对同一子任务的状态不一致,是"重复结算冲突"的温床。

## 正确修复方案(按安全程度排序,建议 1+3)

**这是财务记账核心代码,约 15 处 `settle_child_usage*` 调用点,改动须谨慎、带测试、不破坏 replay/fencing/receipt 不变量。**

1. **幂等护栏(调用侧,最小爆炸半径,首选)**:在 bootstrap/恢复对"遗留子任务"做结算前,检测该 assignment 的模型预留是否已被 receipt-forfeit 解决(`agent_budget_entries` 存在 `kind='forfeit'` 且 source 含 `:receipt:` 且当前 `reserved=0`)。若已解决 → **跳过重复结算**(返回 Ok),失败子任务残留的聚合预留随 root 预算在扫描结束时销毁,可接受。
2. **结算幂等(settle 内)**:`budget::model::settle` 对"已 forfeit、reserved=0"的维度,把"结算其用量"视为 no-op,而不是 `required>reserved` 报错。风险:要确保不影响正常结算路径。
3. **修双账本同步(forfeit 源头,治本)**:`forfeit_original_call_estimated` 作废逐维度预留时,**同步释放 `agent_budget_ledger` 的对应 reserved**,让两套账本始终一致。治本但要核对所有 forfeit 入口。

**建议组合:1(先止血,让扫描能继续)+ 3(治本,消除双账本不一致)**,并加一条回归测试:"模型输出被拒→forfeit→再次 bootstrap 结算" 必须幂等、不报错、账本一致。

## 不影响的部分
确定性层(侦察 + 暴露探测 → git/.env/.DS_Store 等发现)独立于此,已可用。此 bug 只卡住"模型调查分支"。
