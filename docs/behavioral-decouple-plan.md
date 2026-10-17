# Oviraptor：agent_runtime → scanning 行为边解耦计划

- 日期：2026-10-17（Asia/Shanghai）
- 分支：`feature/agent-first-app-platform-20261016`
- 当前 ratchet：`crate::scanning` **27**（类型/常量薄表面 + 切片 1 `log_events` 已落地；其余行为边仍按本文件规划）
- 已完成（类型/端口）：`scope_contract` / `financial_continuation` / `recovery_approval_signature` / `fresh_signature` / `mapper_loop_bounds` / `evidence_bridge_const`
- 已完成（行为端口）：`log_events`（切片 1，边 −3）
- 约束：实现仍留在 `scanning`（include 单元）；`agent_runtime` 只经 `scanning_contract` 点名；每片降 baseline、`cargo check`、普通 push（不 force、不 pop stash）

## 剩余 30 边按模块

| 模块 | 边数 | 调用点摘要 |
| --- | ---: | --- |
| `homogeneous_execution` | 6 | `validate_draft` / `validate_assign` / `release_on` / `accept_on` / `slice_minimum` / `goal_ready` |
| `publication_replay` | 5 | 五类 `native_coordinator_*_publication_replay_on` |
| `evidence_bridge` | 5 | `resolve_agent_evidence_location`×3、`native_business_logic_{input,output}_collection_on` |
| `log_events` | 0（已迁 contract） | `notify_native_sdk_log` → `scanning_contract::log_events` |
| `runtime_test_support` | 3 | `native_homogeneous_apply_after_paid_tick`（仅 test） |
| `http_budget_receipt` | 2 | `verify_saved_on` |
| 输出回执 `read_original_on` | 3 | authorization / external_surface / business_logic |
| `mapper_evidence_lookup` | 1 | `MapperEvidenceInventory` / `Limits` / `Observation`（含 `capture_on`/`new`） |
| `recovery_draft` | 1 | `native_protection_recovery_draft_action_on` |
| `source_deletion` | 1 | `verify_closed_source_for_deletion` |

## 建议切片顺序（低耦合 → 高耦合）

### 切片 1 — `log_events`（边 −3）✅ 已落地

- **风险**：低。副作用为日志总线通知，无 DB 权威写路径。
- **建议端口**：
  ```rust
  // scanning_contract::log_events
  pub(crate) fn notify_native_sdk_log(payload: &serde_json::Value) -> Result<(), String>;
  ```
- **说明**：diagnostics 三处 fire-and-forget；可先做，验证 contract 再导出模式。

### 切片 2 — `runtime_test_support`（边 −3，仅 `#[cfg(test)]`）

- **风险**：低（测试辅助）。可与切片 1 同 PR 或紧随其后。
- **建议端口**：
  ```rust
  // scanning_contract::runtime_test_support（#[cfg(test)]）
  pub(crate) fn native_homogeneous_apply_after_paid_tick(...) -> Result<..., String>;
  ```
- **说明**：签名与 `scanning::runtime_test_support` 现有薄 re-export 对齐即可。

### 切片 3 — `source_deletion` + `recovery_draft`（边 −2）

- **风险**：低～中。只读校验 / 单点 draft action，调用面窄。
- **建议端口**：
  ```rust
  // scanning_contract::source_deletion
  pub(crate) fn verify_closed_source_for_deletion(db, actor) -> Result<..., String>;

  // scanning_contract::recovery_draft
  pub(crate) fn native_protection_recovery_draft_action_on(db, scan) -> Result<..., String>;
  ```

### 切片 4 — `http_budget_receipt` 校验（边 −2）

- **风险**：中。与已落地的 `fresh_signature::FreshHttpBudgetClaim` 配对；只搬 `verify_saved_on`，不搬签发实现。
- **建议端口**：
  ```rust
  // scanning_contract::http_budget_receipt
  pub(crate) fn verify_saved_on(db, actor, value, signature) -> Result<(), String>;
  ```

### 切片 5 — 输出回执 `read_original_on`（边 −3）

- **风险**：中。admission 只读原始回执；三类对称，宜一次搬完并共享命名。
- **建议端口**：
  ```rust
  // scanning_contract::output_receipts
  pub(crate) fn read_authorization_output_original_on(db, actor, child) -> Result<Option<...>, String>;
  pub(crate) fn read_external_surface_output_original_on(db, actor, child) -> Result<Option<...>, String>;
  pub(crate) fn read_business_logic_output_original_on(db, actor, child) -> Result<Option<...>, String>;
  ```
- **说明**：返回类型若仍是 scanning 内 Verified*，可再薄挂到 contract（类似 `fresh_signature`），避免 agent_runtime 为返回类型再点名 `crate::scanning`。

### 切片 6 — `publication_replay`（边 −5）

- **风险**：中～高。监督义务路径上的 replay 判定，语义敏感，但五函数形状同构。
- **建议端口**：
  ```rust
  // scanning_contract::publication_replay
  pub(crate) fn native_coordinator_authorization_output_publication_replay_on(db, actor, child) -> Result<bool, String>;
  pub(crate) fn native_coordinator_business_logic_output_publication_replay_on(db, actor, child) -> Result<bool, String>;
  pub(crate) fn native_coordinator_external_surface_output_publication_replay_on(db, actor, child) -> Result<bool, String>;
  pub(crate) fn native_coordinator_capability_transition_publication_replay_on(db, actor, child) -> Result<bool, String>;
  pub(crate) fn native_coordinator_web_expiry_publication_replay_on(db, actor, child) -> Result<bool, String>;
  ```

### 切片 7 — `evidence_bridge` + `mapper_evidence_lookup`（边 −6）

- **风险**：高。路径解析、冻结证据集合、Mapper inventory `capture_on` 纠缠；`mapper_evidence_lookup` 虽只 1 条 `use`，但 agent_runtime 直接调 `MapperLookupLimits::new` / `MapperEvidenceInventory::capture_on`（行为 API，不是纯类型）。
- **建议端口**：
  ```rust
  // scanning_contract::evidence_bridge
  pub(crate) fn resolve_agent_evidence_location(db, root) -> Result<PathBuf, String>;
  pub(crate) fn native_business_logic_input_collection_on(db, actor, directory) -> Result<..., String>;
  pub(crate) fn native_business_logic_output_collection_on(db, lease, child, directory) -> Result<..., String>;
  // BUSINESS_LOGIC_READONLY_SYSTEM 已在 evidence_bridge_const

  // scanning_contract::mapper_evidence
  pub(crate) type MapperEvidenceInventory = ...; // 或封装构造
  pub(crate) type MapperLookupLimits = ...;
  pub(crate) type MapperLookupObservation = ...;
  // 优先：把 capture_on/new 收成自由函数端口，减少 agent_runtime 调用 inherent 行为方法
  pub(crate) fn capture_mapper_evidence_inventory_on(db, actor, directory, limits) -> Result<MapperEvidenceInventory, String>;
  pub(crate) fn mapper_lookup_limits(source_bytes, observation_bytes) -> Result<MapperLookupLimits, String>;
  ```

### 切片 8 — `homogeneous_execution`（边 −6）— **最后**

- **风险**：最高。切片生命周期（校验/分配/释放/验收/目标就绪/预算下限）深入 apply/intent/budget_bridge；与 lane、assignment、verdict 交织。
- **建议端口**（trait 或模块均可；倾向自由函数端口 + 可选 trait 便于日后共享 crate）：
  ```rust
  // scanning_contract::homogeneous_execution
  pub(crate) trait HomogeneousExecutionPort {
      fn validate_draft(&self, db, lease, task, draft) -> Result<(), String>;
      fn validate_assign(&self, db, lease, slice, body) -> Result<(), String>;
      fn release_on(&self, db, assignment) -> Result<(), String>;
      fn accept_on(&self, db, lease, assignment) -> Result<(), String>;
      fn slice_minimum(&self, db, lease, slice) -> Result<(i64 /*tokens*/, i64 /*requests*/), String>;
      fn goal_ready(&self, db, task) -> Result<bool, String>;
  }
  // 过渡期：scanning_contract 内 `pub(crate) use crate::scanning::homogeneous_execution::{...}` 薄转发，
  // agent_runtime 全部改走 contract；trait 可第二步再抽。
  ```

## 每片落地清单

1. 在 `scanning_contract.rs` 增加对应 `mod`（薄 `use` 或自由函数转发）。
2. 替换 `agent_runtime` 内全部 `crate::scanning::<mod>::…`。
3. 确认无新增 `crate::scanning` 边；下调 `tools/test_agent_runtime_import_ratchet.cjs` 的 `BASELINE`。
4. `cd src-tauri && cargo check`（本机用 stable ≥ 依赖要求的 rustc）。
5. 普通 `commit` + `push`（禁止 force；禁止 `git stash pop`）。
6. 同步 `ARCHITECTURE.md` §11 基线数字与端口名单。

## 明确不在本轮做的事

- 不搬任何行为实现本体出 `scanning`。
- 不宣称共享 crate 抽取完成。
- 不合并切片 7+8（证据桥与 homogeneous 应分开 PR，便于回滚）。
