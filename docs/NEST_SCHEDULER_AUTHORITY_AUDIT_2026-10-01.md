# Native 调度重放与签发完整性增量（2026-10-01）

本轮继续 Master §5.2/§5.4。独立 assignment attempts 仍未实现；本增量补齐其前置防线，不替代真实角色、恢复、聊天、日志或最终验收。

## 数据作用域和授权

- 用户允许真实库业务记录读写及 Nest/CAS 清理，资产不得删除。本轮代码和数据库故障注入仅使用临时 SQLite；没有访问或清理真实 DB/CAS，也没有提交或更新安装包。
- 不新增数据库表、迁移或历史 backfill。当前 Native JSON 和标准 SARIF 不变，Strix 正向兼容不恢复。
- 逐文件检查已有差异，保留之前的预算、取消/续租及 App 启动修复；当前安装的 App 仍是上一轮启动修复版本。

## 修复前证明

`/tmp/oviraptor-scheduler-authority-red.log` 的三个用例确实失败，0/3：

1. 有效运行中 child 的调度重放仍写预算行；设置禁止重放 UPDATE 的触发器后返回错误。
2. 原 assignment fence 被改变后，调度重放仍成功，掩盖原权限损坏。
3. 能力签发 `RAISE(IGNORE)` 漏写后，调度仍成功返回 child。

`/tmp/oviraptor-budget-unclosed-red.log`：未闭合调用的诊断字段缺失（Null，预期 3）。
`/tmp/oviraptor-budget-unclosed-ui-red.log`：界面未拒绝非法计数。

`/tmp/oviraptor-web-binding-red.log`：不同 root 的 received 回执确实使原准入门禁返回成功。
`/tmp/oviraptor-web-binding-delete-red.log`：错误 fence 的 received 回执确实解除删除保护。
`/tmp/oviraptor-scheduler-deadline-red.log`：心跳后的旧 handle 使新 child 仍用旧期限。
`/tmp/oviraptor-takeover-fixture-red.log`：接管夹具新 root 缺冻结 Native timeout（`budget_frozen_timeout_invalid`）；补入相同目标的冻结计划，保留所有 stale worker 断言。

## 生产行为

- 已有逻辑 assignment 先只读核验原计划、child、Native Multi 模式、角色/泳道/作用域、运行状态、未过期 epoch/fence、取消标记、lane、合同 owner、预算与完整能力集合；有效时返回原 child，不重写预算、租约、状态或能力。
- 缺失、过期、撤销或更换的权限原样保留并拒绝重放；同 root 新 epoch 也不能自动接管旧 child。领取/重派仍需独立 attempt 合同，不能在该入口隐式完成。
- 新签发在最后一次写入后复核上述完整资源及统一期限；IGNORE、写中撤权或损坏整体回滚。
- 新 child 的签发期限取当前已提交 Coordinator row，读取期限不会续期。重放仍核验原 child，不跟随根心跳自动修复其过期权限。合同 owner 的只读核验保留 `stale_contract_fencing` 精确错误，不调用会轮换 token 的 acquire。
- `unclosedWebModelCalls` 在同一只读快照内按 root 统计 dispatch，无同 call/root/assignment/child/epoch/fence/round/request hash 的 received/unsent 回执即计入，uncertain 也计入。已收到但估算费用仍由十维未决账目表示。
- 返回计数而不暴露调用 ID、请求 hash、身份或 fence。UI 按需读取；非法、负数、非整数和超安全整数计数拒绝整份回执，并使用固定错误提示；缺失字段显示“—”，不冒充 0。
- 准入、原 assignment 结算、未发出预算释放和普通删除保护同用 `WEB_RECEIPT_BINDING`。恢复出的不同绑定回执不能解除原义务；实际 journal/预算/数据库行完整快照不变。received 与 unsent 的 7 维损坏各一组，共 14 种；删除保护另保留匹配/不匹配对照。

## 文件和结构

- 调度完整性集中在 `agent_runtime/multi_agent/scheduler/schedule_authority.rs`；调度入口接线，assignment 的原计划比较供共享核验使用。
- `scheduler_authority.rs` 保留独立重放、资源损坏、签发部分写入、同 root 更换 epoch 与期限边界测试。没有删除独有安全测试。
- 预算诊断/类型/实际 Vue 组件及现有诊断测试增补计数；事务快照补入合同 owner。
- 新文件均小于 400 行；已有 `src/types.ts` 是既存大型共享类型文件，本次仅加一个可选计数字段。

本轮核心修改文件行数（不包括既存接线 include、未变更资源与锁文件）：

| 归属 | 文件 | 行数 |
| --- | --- | ---: |
| 调度 | assignment repository / contract owner | 366 / 185 |
| 调度 | scheduler/assignment / schedule_authority / lifecycle | 386 / 134 / 276 |
| 调度测试 | scheduler_authority / child_start_authority | 151 / 54 |
| 预算 | budget / budget_gaps | 388 / 182 |
| 预算测试 | budget_web_binding / API diagnostics | 104 / 103 |
| Source Reviewer | native_source_reviewer / coverage_reviewer | 371 / 110 |
| Source 测试 | saved-paused budget / specialists / cancellation | 117 / 386 / 297 |
| 实际 UI | NativeBudgetDiagnostics.vue | 107 |

最终门禁起始的源码/测试/配置 783 文件摘要：`31954555352818b0bb34ea2babc346cceedc68b7ce6355b1d763419af84eb1d7`，清单 `/tmp/oviraptor-master-authority-snapshot.json`。不含真实数据库或 CAS；终态已核对无变动，未把运行期间变动套用旧测试结果。

## 开发门禁

修复后的最终完整 all-targets/all-features 已结束 **exit 0：主库 1582/1582（1842.16s）、导入工具 39/39、主程序 0 项**，日志 `/tmp/oviraptor-master-authority-all-targets-final.log`。运行期间及终态复核 783 个源码/测试/配置文件无改动或新增，与上文摘要一致。之前完整回归确实 exit 101：库 1575/1579、4 项失败，其他目标当时因库失败未运行；历史失败不删除或冒充通过。严格全目标全特性 Clippy 已通过（exit 0，`/tmp/oviraptor-master-authority-clippy-final.log`）。最终调度专项 5/5、Web 模型 12/12、诊断 API 2/2、UI 诊断 5/5、预算夹具 3/3、接管及合同 fencing 各 1/1，完整 UI 774/774、前端构建和 fmt/空白检查通过。它们不替代最后全量代码快照或整体验收。

第一次探索性 lib 回归在发现 4 项失败后主动停止，exit 143；两个缺 journal 的诊断夹具、缺冻结计划的新 Coordinator 夹具已修；合同 fencing 保留原精确错误。全部原测试和断言保留。后续增量另先证实期限/绑定问题，最终门禁只认下面最后快照的终态，不把已终止进程记作全量通过。

## 全量发现的 Source 收口缺口（本增量已修复）

- 三项失败来自候选/覆盖 Reviewer 的保存回执本地恢复：暂停状态完成时仅释放 lane，漏释放追加账本的并发槽，根收口被 `budget_reservations_require_settlement` 拒绝。这是生产缺口，不能归因于测试夹具。
- 新负测先用实际 localhost 模型执行到 `paused_received`，再注入释放 IGNORE/ABORT/写中根取消；修复前 IGNORE 后完成仍成功，`/tmp/oviraptor-source-saved-slot-red.log` exit 101。两处原事务内补 `release_slot`，不改权限、费用或运输；同一负测修复后 1/1，通过两类 Reviewer 的三种故障完整回滚、保留槽和费用、健康完成槽归零及幂等无写入，实际各 7 次请求未增加。
- 第四项取消测试的 sibling Coordinator 缺冻结 Native 计划，补入生产 builder 生成的准确计划/hash/预算。两项取消测试从 474 行旧文件完整移出，先验证拼回原文件字节一致，再仅修改 sibling 夹具；原文件 386 行，取消模块格式化后 297 行，全部取消断言保留。
- Source Reviewer 受影响回归已完成 59/59（619.16s），原三项失败均转绿；取消夹具专项 1/1，新保存回执槽释放故障矩阵 1/1。最后全目标全特性回归已完成 exit 0，日志 `/tmp/oviraptor-master-authority-all-targets-final.log`；本增量修复通过开发门禁，整体 Master 仍未完成。完整失败日志 `/tmp/oviraptor-master-authority-all-targets.log`；新修复不沿用前面的通过数字。

## 启动边界证明与独立 attempt 剩余作用域

- `scheduler/lifecycle.rs` 原启动只核验根权限和部分 Source 合同，已签发后能力撤销及启动写中撤权仍能成功：`/tmp/oviraptor-child-start-authority-red.log` 0/2、exit 101。最小补丁在原事务状态写入前后复用完整权限核验，保持原任务内容，最后核验 assignment/run 确为 running；不续租、签发或接管。
- 新 `child_start_authority.rs` 覆盖 9 种启动前损坏与 7 种写中损坏，完整快照不变；去掉故障后同一原 child 才能正常启动。修复后两项及原启动失败释放回归 3/3。生命周期模块 276 行，没有新业务万能模块或重复权限策略。
- §5.2 的实际缺口：数据库仅有 `agent_assignments`，未定义 `agent_assignment_attempts`；`budget.rs` 的 `lease_attempt_id` 仍由根 `lease_epoch:fencing_token` 拼接，逻辑状态仍含终态 `LeaseExpired`。领取/独立 worker/重派合同不能靠改名或补历史行完成。本轮不做不受约束的迁移或历史回填。

## 未完成和风险

- Master §5.2 独立 attempt/worker 身份、真正的安全重派及 Expired 从逻辑任务分离仍未完成；目前损坏/过期重放一律拒绝。
- Web 费用回执与模型事件/旧用量/检查点分阶段发布，任何已有 Web 历史的重入仍全部拒绝自动续租恢复；完整强杀恢复合同未交付。
- 单智能体/root 自身计费、动态 grant、精确人工对账、完整角色/模式/Reviewer 监督、聊天及逐路实时日志、完整安装态和授权 URL 仍未完成。
- 未闭合调用计数是诊断，不是新的恢复权限、退款依据或全部费用结算证明。安装态不包含本轮新代码。
