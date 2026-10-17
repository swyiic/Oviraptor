# 独立角色持久化调用与原子结果交付审核

日期：2026-09-26。承接 `NEST_SPECIALIST_TRANSPORT_AUDIT_2026-09-26.md`，推进真实 child-run/mailbox 执行链，不代表整个 Master Plan 完成。

## 本轮实现

### 1. 派发前必须落库

新增 `agent_specialist_calls`，通过正常数据库初始化加入新建及既有数据库；不修改历史 JSON 结构或引入旧运行时依赖。

每个 readonly specialist assignment 只有一行，绑定 child、root、role、epoch/fencing，以及脱敏后的冻结请求和摘要。请求包含 schema version、消息、模型名、上下文/输出配置、endpoint/proxy 的摘要，不保存 API key 或代理凭据。真实生产调用覆盖 Mapper、IdentitySession、Reviewer、Investigator；人工提案保留已有独立协议。

派发在 `IMMEDIATE` 事务内重新检查：

- scan 仍在扫描、attempt 正确、未删除，Coordinator 未结束。
- assignment、child、root、角色、lane、目标和 fencing 对应。
- 首次派发必须是 running、尚未结算、预留至少一次模型请求，且持有有效的只读证据能力。
- 没有历史模型事件或 snapshot，防止把旧版已经调用过但没有新日志的 child 当作未执行。
- `executing` 写入成功且读回一致，才返回一次派发许可。

并发请求看到同一个 executing 调用时不再派发。该状态也覆盖“落库后、真正发出 HTTP 前崩溃”的不可判定窗口：宁可待核对，也不能猜测未调用并重试。

### 2. 三种状态，不伪造成功

| 状态 | 含义 | 再次进入 |
| --- | --- | --- |
| executing | 已取得一次派发许可，尚无可信的持久化结果 | 拒绝自动重试 |
| uncertain | 模型调用返回错误，结果/费用无法据此视为零 | 拒绝自动重试 |
| received | 脱敏响应、用量事件、checkpoint 同一事务保存成功 | 同一请求/归属可纯本地读取 |

`received` 不代表建议执行、角色业务处理完成、漏洞确认或 Reviewer 通过。已收到的空输出、意外工具调用以及过大结果也保存为拒绝型回执；不会执行模型返回的工具。工具参数不入此回执。

正常响应文本最多保留 1 MiB；超过时保存 `response_too_large` 拒绝标记和合法用量，不截断后伪装成正常评估。非法用量不能创建有效回执，此时仍需要核对。

### 3. 原子回执与幂等读取

`agent_runtime/multi_agent/specialist.rs` 负责持久化不变量；命令层负责真实模型传输。

收到响应后，同一事务写入：

1. 模型用量事件。
2. 对应 checkpoint。
3. 脱敏响应、用量、绑定摘要、事件序号及 received 状态。

提交前读回验证 receipt hash、事件 payload 和 checkpoint 的 run/次数/用量/摘要。任一步失败，包括 SQLite `RAISE(IGNORE)` 静默跳过写入，都回滚整组数据，保留此前的 executing 记录。

重放要求相同冻结输入和模型配置、相同且仍有效的归属；校验回执、事件及 checkpoint 后只返回本地结果。不再次 HTTP、不追加用量事件、不重复计费，也不恢复已撤销能力。数据库触发器禁止 received/uncertain 记录被常规 UPDATE 改写；请求及归属绑定也不可更新。hash 是完整性校验，不是抵御持有数据库完全写权限者的签名。

过期/换代 fencing、scan 结束或新 attempt 不适用这个执行期重放入口。旧归属/终止后的通用 specialist 对账仍需专门工作流。

### 4. Mapper / Identity 结果交付原子化

原代码先结算并结束 child，随后才发送和确认消息。消息失败可能留下已完成角色却没有可用交付的状态。

本轮 `complete_readonly_assessment` 将以下操作置于一个事务：费用结算、child/assignment 结束、能力撤销、lane 释放、结果 mailbox 写入、投递与确认。提交前核验实际费用、终态、预算清理、权限回收、lane 释放及单次确认。

交付失败时这组状态回滚；生产调用方继续使用费用保全暂停，独立保存的模型回执不丢弃。同一个已完成交付的内部重放只验证原结果，不重复结算或投递。不同结果不能覆盖原消息。

本审计当时的原子交付仅覆盖 Mapper/Identity。后续 Reviewer 决策发布与 Investigator 双向消息的原子交付见 `NEST_REVIEW_DELIVERY_ATOMICITY_AUDIT_2026-09-26.md`；它仍不代表整个任务恢复或终止后业务补交已完成。

## 新增测试

`agent_tests_specialist_journal.rs` 新增 11 项测试，使用真正的 loopback HTTP 和 SQLite：

1. 已保存响应重复读取，无第二次模型调用、无重复事件、无新增费用，且 paused 角色不被恢复执行权限。
2. 两个并发 transport 入口只发一次请求；另一个要么读到回执，要么明确等待核对。
3. HTTP 503 持久化 uncertain，重新进入不重试。
4. 响应行、事件、checkpoint 三处 ABORT/IGNORE 注入，验证整组回滚且移除故障后也不会重发未知调用。
5. 派发记录 ABORT/IGNORE 时 HTTP 请求数必须为零。
6. 在提供方返回前替换 fencing，迟到响应不能写入新归属。
7. 改坏请求、响应、用量、事件或 checkpoint 后拒绝重放；常规更新不可变回执先被触发器阻止。
8. 暂停任务、新 attempt、删除任务、终止 Coordinator、暂停 assignment、撤销能力、过期租约及旧模型历史均拒绝首次派发。
9. uncertain 写入被忽略时，同时返回原 HTTP 错误和持久化错误；executing 记录仍阻止再次请求。
10. 预算、assignment、run、capability、lane、结果消息和确认七处 IGNORE 注入，证明 Mapper 原子交付回滚；去掉故障后本地读取原结果并仅结算/确认一次。
11. 真实 `multi_agent_prepare` 中 Mapper/Identity 的确认失败分别保留收到的回执和预留，暂停角色，不留下完成样式的结果消息。

定向匹配 `specialist_` 的 24 项测试通过（包含上述 11 项、上一轮 9 项和已有相关测试）。首次接入时发现 Reviewer 阶段 `context.run` 已指向同根 Executor；绑定检查已改为验证数据库中的同 scan/attempt/target/root 关系，而不是错误要求 context 必须直接指向 root。没有放宽到其他任务。

## 全量验证

本轮结果如下，未修改退休 allowlist：

| 检查 | 结果 | 日志 |
| --- | --- | --- |
| `cargo test --offline --all-targets --all-features -- --test-threads=1` | 主库 748、导入器 30，通过 | `/tmp/oviraptor-20260926-specialist-journal-full.log` |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | 通过 | `/tmp/oviraptor-20260926-specialist-journal-clippy.log` |
| `cargo fmt --all -- --check` | 通过 | `/tmp/oviraptor-20260926-specialist-journal-fmt.log` |
| `npm run test:agent-dialog` | 38 通过 | `/tmp/oviraptor-20260926-specialist-journal-ui.log` |
| `npm run build` | 通过；777.36 kB JS 主包警告仍在 | `/tmp/oviraptor-20260926-specialist-journal-build.log` |
| `node tools/test_native_runtime.cjs` | 通过；8 请求、身份隔离通过 | `/tmp/oviraptor-20260926-specialist-journal-native.log` |
| `git diff --check` | 通过 | 当前工作树检查 |

定向输出保存在 `/tmp/oviraptor-20260926-specialist-journal-tests.log`。这些是本机短期日志，不代替交接时重新执行验收，也不证明下面未完成项已交付。

## 尚未完成的恢复与产品工作

后续说明：同日 `NEST_READONLY_RECOVERY_AUDIT_2026-09-26.md` 已补齐下述 paused/completed 初始化分析重入、paused received 本地交付及 prepare 内观察者误清理问题。下列列表保留本回执增量提交时的边界；终止后通用恢复、外层整体执行竞争及其他业务链的未完成项仍有效。

- 运行期 transport 可读回结果不等于整个编排入口已支持断点续跑。`multi_agent_prepare` 仍会拒绝既有 paused/completed assignment，终止后没有通用 specialist UI 对账入口。
- 并发的上层调用若遇到“已有调用结果不明”，仍可能进入现有暂停清理；已派发调用只会收到一次并保存回执，但尚没有将所有上层竞争收敛为同一个完整业务返回值。
- 当前保存的是网关解析后的脱敏只读结果和用量，不是原始供应商 HTTP 响应。无效用量、写入期间归属过期/替换、响应保存失败，仍留下待核对的执行记录。
- 用量超出预留仍会由结算门拒绝；输出预算与输入大小的事前估算不是本轮交付。
- Investigator 的缺口 → 新合同 → 真实采证 → 新证据 revision → Reviewer 再审仍未贯通；Stage 10 其余专家也不能据此视为完成。
- 本轮不访问外部授权目标、不部署、不新增主机 Agent。主机边界仍为 Web-only。

下一步从保存的 received 回执继续完成作用域明确的业务恢复和终止后对账，再贯通补证再审；不要通过重发模型或伪造 completed 来绕过现有缺口。
