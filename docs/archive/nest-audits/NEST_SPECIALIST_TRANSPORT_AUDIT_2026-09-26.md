# 独立角色真实调用与失败费用保全审核

日期：2026-09-26。对应 Master Plan Stage 7–9 的执行基础，不代表 Stage 9、Stage 10 或整个多智能体产品已经完成。

## 本轮问题与修复

生产路径中的 Mapper、IdentitySession、Evidence Reviewer、Deep Investigator 使用独立 child run，但原模型调用允许隐式重试；每个 assignment 却只预留一次模型请求。原单元测试使用固定角色替身，未直接覆盖这个真实 HTTP 行为。部分调用和结算错误还会通过普通 child 结束流程释放全部剩余预留，丢失“远端可能已经消耗费用，但本地尚未结算”的状态。

本轮修改 `src-tauri/src/commands/multi_agent_runtime.rs`：

1. 提取生产与测试共用的 `multi_agent_child_round_transport`；真实调用使用 `complete_once`，不在一次请求预留下隐式再次请求。测试的真实传输开关为线程局部 RAII，只在定向回环测试启用，不改变其他夹具。
2. 收到模型响应后先写已有用量事件与 checkpoint，再拒绝空文本和只读角色意外返回的工具调用；不会执行返回的工具调用。
3. Mapper、Identity、Investigator 的调用/结算失败改用费用保全清理：未结算 assignment 与 run 暂停，保留预算和 lane，撤销执行能力。不把失败当零费用，不在相同 gap revision 上自动重试。
4. 独立 Reviewer 的请求失败状态与 child 失败清理放在同一 `IMMEDIATE` 事务中；清理失败时整体回滚，并同时返回原错误和清理错误。
5. 在未结算暂停提交前重新读取数据库，核验 assignment/run 已暂停、失败分类正确、结算标记仍为空、预算预留未变、child 没有未撤销能力。SQL 执行未报错本身不再被当作成功证明。

此机制也保护复用该清理 helper 的 Executor/Authorization 未结算错误路径。已结算 child 仍走普通失败收口，不重复收费。

## 真实回环与故障注入

新增 `src-tauri/src/commands/agent_tests_specialist_transport.rs`，9 项测试：

1. Investigator 收到 HTTP 503 只发一次请求；再次进入相同 revision 不追加调用，保留 4000 tokens / 1 request 的预留。
2. Investigator 已收到响应但结算写入失败，原结算错误保留，预算不被异常清理释放。
3. Mapper HTTP 401 后保留 8000 / 1，Executor 不被派发。
4. Reviewer HTTP 503 的正常清理与 SQLite ABORT 注入：前者暂停并保留 8000 / 1；后者保留原错误及清理错误，请求状态回滚为 running。
5. Identity 在 Mapper 成功后 HTTP 503：共两次 HTTP，仅 Mapper 的已知 20 tokens / 1 request 被结算；Identity 保留 4000 / 1，身份凭据不进入模型请求。
6. Mapper → Identity → Reviewer → Investigator 四个角色分别使用真实 loopback HTTP：4 次独立请求，4 份 child 用量（每份 20 / 1），总账 80 / 4，GapProposed 与 ProposalAssessed 两条消息均已确认。模型请求没有目标工具与测试凭据。
7. Reviewer 暂停 assignment、暂停 run、撤销 capability 三处分别注入 `RAISE(IGNORE)`，必须报告提交前核验失败，并回滚整组请求/角色状态变化。
8. Reviewer 已收到响应但结算失败：保留预留，checkpoint 保留 20 / 1，不能冒充已完成复核。
9. Investigator 返回空文本或越权工具调用：拒绝输出，保留已收到的 20 / 1 checkpoint 和未结算预留，不执行工具。

第 6 项的 Executor 是预先收口的夹具，未请求测试目标；不能将该测试描述成真实目标端到端扫描、自动补证或再审验证。

两次修复前失败记录：

- `/tmp/oviraptor-20260926-specialist-red.log`：隐式重试产生两次 HTTP；结算/调用失败错误释放预留。
- `/tmp/oviraptor-20260926-specialist-silent-red.log`：数据库静默跳过 assignment 暂停后，旧代码未识别清理未完成。

## 验证结果

本轮已完成下列验证，未修改退休 allowlist 来消除失败：

| 检查 | 结果 | 本地日志 |
| --- | --- | --- |
| specialist 真实传输定向测试 | 9 通过 | `/tmp/oviraptor-20260926-specialist-tests.log` |
| `cargo test --offline --all-targets --all-features -- --test-threads=1` | 主库 737、导入器 30，通过 | `/tmp/oviraptor-20260926-specialist-full.log` |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | 通过 | `/tmp/oviraptor-20260926-specialist-clippy.log` |
| `cargo fmt --all -- --check` | 通过 | `/tmp/oviraptor-20260926-specialist-fmt.log` |
| `npm run test:agent-dialog` | 38 通过 | `/tmp/oviraptor-20260926-specialist-ui.log` |
| `npm run build` | 通过；JS 主包 777.36 kB 警告仍在 | `/tmp/oviraptor-20260926-specialist-build.log` |
| `node tools/test_native_runtime.cjs` | 通过；8 个请求，身份隔离通过 | `/tmp/oviraptor-20260926-specialist-native.log` |
| `git diff --check` | 通过 | 当前工作树检查 |

`/tmp` 日志是本机短期验证证据，不是跨机器归档物；交给其他执行者时需要重新运行。门禁全绿不代表下面列出的产品缺口已经完成。

## 明确保留的缺口

- 这些通用 specialist 尚没有与人工只读提案同等的持久化 dispatch/原始响应/原子结果交付协议；模型事件与 checkpoint 仍不是同一事务。保留 checkpoint 不等于已经具备自动费用对账能力。
- 未知远端结果、通用 paused child、旧 fencing 的恢复与人工费用确认仍待实现。本轮没有允许自动重试来掩盖该问题。
- 暂停事务本身失败时会明确报错并回滚，不能宣称权限撤销已落盘；后续仍须依靠已有租约检查/根任务收口，不能把这种错误当成可继续执行的成功状态。
- Mapper/Identity 的成功结束与 mailbox 交付仍需进一步完善原子恢复。Reviewer 在模型派发前创建请求失败的清理错误传播也需后续核对。
- Investigator 当前给出与交换的是补证建议，Coordinator 的评估仍为 deferred。缺口 → 新合同 → 实际采证 → 新证据 revision → Reviewer 再审的执行闭环尚未完成。
- 这次验证不覆盖真实供应商费用、外部授权 URL、生产部署、跨平台沙箱或浏览器视觉验收。前端主包体积警告不因后端测试通过而消失。
- 主机能力继续仅记录越界候选，不实现 Linux HostVerifier，不扩展 Web 授权范围。

下一步优先补齐可恢复的 specialist 交付与费用状态，再贯通新合同采证/再审链；不得把已通过的基础测试当作整个 Master Plan 的退出门禁。
