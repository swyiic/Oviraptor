# 千问执行任务：Nest 多智能体 Stage 1A——运行时合同与持久化骨架

> 这是一次**单独、封闭的实现任务**。只完成本文要求，然后停止并提交验收结果。
> 不要继续实现 Scheduler、多个模型循环、自定义 Agent、协作 UI 或任何后续 Stage。

## 1. 唯一目标

在不改变当前 Native 单 Agent 扫描行为的前提下，为后续多智能体增加：

1. 稳定的角色、调度模式、lane、assignment、证据图和 review Rust 合同；
2. 对应的 SQLite 增量表、索引和 repository；
3. 完整的迁移、幂等、去重、状态转换和兼容读取测试；
4. `single | shadow | multi` 策略字段，但本任务中**不得接入执行路径**，默认必须为 `single`。

完成本任务后，运行一次现有扫描时必须仍然只有一个 Coordinator run、一个模型循环，并且目标请求数、
模型请求数、结果和终态行为与修改前一致。

## 2. 开始前必须做的事

先阅读：

1. `docs/NATIVE_AGENT_RUNTIME_REMEDIATION_REQUIREMENTS.md`
2. `docs/NATIVE_MULTI_AGENT_REFACTOR_REQUIREMENTS.md` 的第 4、6、7、9、10、11、13、15、16、18 节
3. `src-tauri/src/agent_runtime/contract.rs`
4. `src-tauri/src/agent_runtime/store.rs`
5. `src-tauri/src/agent_runtime/tests.rs`
6. `src-tauri/src/db.rs` 中现有 `agent_*` 表和增量迁移方式

然后记录以下基线结果。如果基线失败，只报告原有失败，不要借本任务大范围修复：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build
git diff --check
```

当前工作树有大量未提交和未跟踪文件。先读取再修改，不得把任何文件恢复到 Git 版本。

## 3. 严格改动边界

优先只改动或新增：

```text
src-tauri/src/agent_runtime/contract.rs
src-tauri/src/agent_runtime/store.rs
src-tauri/src/agent_runtime/mod.rs
src-tauri/src/agent_runtime/tests.rs
src-tauri/src/agent_runtime/multi_agent/mod.rs
src-tauri/src/agent_runtime/multi_agent/assignment.rs
src-tauri/src/agent_runtime/evidence_graph/mod.rs
src-tauri/src/agent_runtime/evidence_graph/contract.rs
src-tauri/src/agent_runtime/evidence_graph/store.rs
src-tauri/src/agent_runtime/review.rs
src-tauri/src/db.rs
```

如果测试拆分需要，可在 `agent_runtime/` 下增加测试文件并从 `tests.rs` 引入。

本任务禁止修改：

```text
src-tauri/src/commands/agent_native.rs
src-tauri/src/commands/agent_backend.rs
src-tauri/src/commands/agent_tools*.rs
src-tauri/src/commands/scan_execution*.rs
src-tauri/src/commands/strix_compatibility.rs
src/**/*.vue
src/**/*.ts
package.json
```

除非出现由本任务新增类型造成的编译错误，否则不要触碰 `commands/`。如果确实必须修改，只允许做导入或
旧枚举名称适配，并在最终报告逐行说明原因；不得调整扫描控制流。

## 4. 必须实现的 Rust 合同

### 4.1 `MultiAgentPolicy`

精确定义：

```rust
enum MultiAgentPolicy {
    Single,
    Shadow,
    Multi,
}
```

要求：

- serde 使用 `snake_case`；
- `as_str()` 只返回 `single | shadow | multi`；
- `parse()` 对空值和未知值返回 `Single`；
- `Default` 必须是 `Single`；
- 本任务只持久化该值，不得让 `Shadow` 或 `Multi` 启动额外模型调用、Agent 或目标请求。

### 4.2 `AgentRole`

删除旧角色作为可写枚举值，稳定写入值固定为：

```rust
Coordinator
SpaApiMapper
ExternalSurface
IdentitySession
Authorization
InputParser
Upload
BusinessLogic
Concurrency
ClientSide
DeepInvestigator
EvidenceReviewer
```

读取历史行时必须支持以下别名，但再次写入时只写右侧新值：

```text
evidence_triage          -> spa_api_mapper
contract_verifier        -> input_parser
identity_comparator      -> authorization
business_flow_analyst    -> business_logic
attack_chain_correlator  -> deep_investigator
final_reviewer           -> evidence_reviewer
```

未知值继续按当前兼容策略读取为 `Coordinator`，不要批量更新历史数据。

### 4.3 `AgentLane`

固定为：

```rust
TargetTouching
ReadOnlyAnalysis
Review
```

稳定数据库值：`target_touching | read_only_analysis | review`。

### 4.4 `AssignmentState`

固定状态：

```text
prepared
leased
running
waiting_review
paused
needs_evidence
completed
failed
cancelled
lease_expired
```

实现一个纯函数 `can_transition(from, to)`，只允许：

```text
prepared -> leased | cancelled
leased -> running | cancelled | lease_expired
running -> waiting_review | paused | needs_evidence | completed | failed | cancelled | lease_expired
paused -> leased | cancelled
needs_evidence -> leased | cancelled
waiting_review -> completed | needs_evidence | failed | cancelled
```

终态 `completed | failed | cancelled | lease_expired` 不允许再次转换。

### 4.5 证据图合同

`EvidenceNodeKind` 固定包含：

```text
target
identity
page_state
endpoint
request_record
response_shape
business_object
hypothesis
contract
tool_invocation
candidate_finding
finding
```

`EvidenceProvenance` 固定为：

```text
observed
source_derived
inferred
```

`EvidenceEdgeKind` 固定包含：

```text
derived_from
observed_by
supports
contradicts
targets
uses_identity
controls
tests
supersedes
```

所有 enum 必须具有稳定 `as_str/parse` 和 serde round-trip。不得把 hypothesis、candidate 或 inferred
证据自动转换成 finding。

### 4.6 Review 合同

`ReviewVerdict` 只能是：

```text
confirmed
rejected
insufficient_evidence
```

Review 记录必须包含：candidate id/revision、reviewer run id、reason codes、evidence refs、
counter-evidence refs、missing-evidence refs、confidence、severity 和创建时间。

### 4.7 Message 合同

在现有 `AgentMessageKind` 中增加：

```text
gap_proposed
proposal_assessed
review_requested
review_decided
human_directive
```

这里只增加可持久化词汇；不要实现 Agent 自由聊天或 coordination loop。

## 5. 必须实现的数据库增量

不得删除、重建或批量改写已有表。初始化必须可以对旧数据库重复执行。

### 5.1 `agent_runs` 新列

```text
root_run_id              TEXT NOT NULL DEFAULT ''
assignment_id            TEXT NOT NULL DEFAULT ''
lane                     TEXT NOT NULL DEFAULT ''
orchestration_policy     TEXT NOT NULL DEFAULT 'single'
capability_lease_json    TEXT NOT NULL DEFAULT '[]'
reserved_tokens          INTEGER NOT NULL DEFAULT 0
reserved_requests        INTEGER NOT NULL DEFAULT 0
heartbeat_at             TEXT NOT NULL DEFAULT ''
cancel_requested_at      TEXT NOT NULL DEFAULT ''
```

使用现有的 `ensure_column`/幂等迁移模式。现有历史行不得被改成 `multi`。

### 5.2 `agent_assignments`

必须建立以下稳定列：

```text
id, coordinator_run_id, child_run_id, role, lane, target_key, state,
dedup_key, trigger_code, task_slice_json, evidence_revision,
contract_keys_json, identity_handles_json, reserved_tokens, reserved_requests,
capability_lease_json, deadline_at, failure_class,
created_at, leased_at, started_at, finished_at, updated_at
```

要求：

- 主键为 `id`；
- 唯一约束为 `(coordinator_run_id, dedup_key)`；
- 为 `(coordinator_run_id, state, lane)` 和 `child_run_id` 建索引；
- JSON 列只放任务切片和列表，role/lane/state/预算/生命周期不得藏进 JSON。

### 5.3 `agent_evidence_nodes`

稳定列：

```text
id, root_run_id, revision, kind, provenance, natural_key_hash,
payload_json, artifact_refs_json, created_by_run_id, supersedes_id, created_at
```

唯一约束：`(root_run_id, revision, natural_key_hash)`。

### 5.4 `agent_evidence_edges`

稳定列：

```text
id, root_run_id, revision, from_node_id, to_node_id, kind,
payload_json, created_by_run_id, created_at
```

唯一约束：`(root_run_id, revision, from_node_id, to_node_id, kind)`。

`from_node_id` 和 `to_node_id` 必须引用 evidence node；插入悬空边必须失败。

### 5.5 `agent_review_decisions`

稳定列：

```text
id, root_run_id, candidate_id, candidate_revision, reviewer_run_id,
verdict, reason_codes_json, evidence_refs_json, counter_evidence_refs_json,
missing_evidence_json, confidence, severity, created_at
```

唯一约束：`(candidate_id, candidate_revision, reviewer_run_id)`。

本任务不得新增 `finding` 发布逻辑。保存 `confirmed` review 不得自动写 Sentinel finding。

## 6. Repository 行为必须明确

实现类型化 repository，不允许让未来调用者到处手写 SQL。最少提供：

```text
insert_assignment                 // 重放相同 dedup key 返回既有记录/未插入，不生成第二条
load_assignment
list_assignments_for_coordinator
transition_assignment             // compare-and-set；校验 can_transition

insert_evidence_node              // 写入前 redact_json；幂等去重
insert_evidence_edge              // 写入前 redact_json；拒绝悬空引用
list_evidence_nodes_at_revision
list_evidence_edges_at_revision

insert_review_decision            // 同一唯一键幂等；冲突内容不得静默覆盖
list_review_decisions_for_candidate
```

具体函数可以使用 Rust 命名，但语义必须一致。

所有 payload 和 artifact 引用写入前复用现有脱敏逻辑。禁止在本任务中创建第二套 secret/redaction
实现。数据库错误必须返回 `Result`，不得 `unwrap` 或吞错。

## 7. 必须新增的测试

至少覆盖以下测试；测试名可调整，但一个条件不能省略：

1. `MultiAgentPolicy` 默认和未知值都是 `single`。
2. 所有新 `AgentRole` round-trip；六个旧值能读成新语义；写回只产生新值。
3. lane、assignment state、evidence kind/provenance/edge、review verdict 全量 round-trip。
4. 每一条允许的 assignment 转换成功；禁止转换和终态二次转换失败。
5. 对已有数据库执行两次初始化不报错，旧 `agent_runs` 行仍存在且 policy 为 `single`。
6. 新列、四张新表、唯一约束和索引确实存在。
7. 相同 assignment dedup key 重放不会生成第二条；不同 coordinator 可使用相同 dedup key。
8. compare-and-set 使用错误旧状态时不更新。
9. evidence node/edge 可以 round-trip；悬空 edge 插入失败。
10. evidence payload 中的 Authorization、Cookie、token、password 等敏感值不会原样落库。
11. 同一 candidate/revision/reviewer 的相同 review 重放幂等；不同内容冲突返回错误；新 revision 可保存。
12. 保存 `confirmed` review 后 `sentinel_findings` 数量不变化。
13. 现有 Native mock E2E 仍只有一个 Coordinator run，不产生 child run、assignment 或额外请求。

测试必须使用临时 SQLite 和 mock server，不允许访问真实网站。

## 8. 明确禁止

- 不实现 Scheduler、Coordinator tick、Agent runner 或并发队列。
- 不实例化任何新角色，不创建 child run。
- 不调用第二次模型 API，不增加任何目标 HTTP/CDP/浏览器请求。
- 不实现 `shadow` 计算；这里只保留策略值，真正 shadow planner 属于下一任务。
- 不实现 `AgentDefinition`、自定义角色、能力包注册或 Role Studio；这些属于 Stage 1B。
- 不实现 gap deliberation、Deep Investigator 往返、Reviewer 模型逻辑或人工干预。
- 不修改 Nest 页面，不制作聊天气泡或虚假进度。
- 不新增通用 `strix_*` 命名，不移动或删除 Strix 兼容代码。
- 不添加 shell/Python/任意二进制执行能力。
- 不新增第二套 Tool Broker、预算、终态 reducer、artifact store 或脱敏器。
- 不因 Clippy 删除安全检查、测试或错误处理。
- 不执行 `git reset --hard`、`git checkout -- .`、`git restore .`、`git clean -fd`。

## 9. 完成门禁

完成后必须逐条运行：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build
git diff --check
```

并额外用 `git diff --name-only` 证明没有修改禁止文件。若因为编译适配修改了例外文件，必须给出精确
文件、行和原因。

## 10. 最终回复格式

严格按下面格式回复，不要只说“已完成”：

```text
Stage 1A 状态：完成 / 未完成

改动文件：
- 路径：改了什么

合同实现：
- policy：
- roles/aliases：
- lane/state：
- evidence graph：
- review/message：

数据库：
- 新列：
- 新表/索引/约束：
- 迁移兼容结果：

行为不变证明：
- child run 数：
- assignment 数：
- 新增模型请求数：
- 新增目标请求数：

测试：
- fmt：
- cargo test：通过数/失败数
- clippy：
- npm build：
- diff check：

未完成或风险：
- 必须如实列出；没有则写“无”
```

任一验收项未通过时，状态必须写“未完成”。本任务结束后停止，不要自行开始 Stage 1B。
