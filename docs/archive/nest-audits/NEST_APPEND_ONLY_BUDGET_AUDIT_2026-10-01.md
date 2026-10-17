# 2026-10-01 追加预算实现审计（进行中）

本记录对应 Master §5.4 的开发增量，不是完成声明。用户要求“完成 master”，随后指出开发未完善前不应开始整体验收；本轮只以先红后绿的合同测试验证具体修复。最终全量门禁、安装态、真实团队/聊天/日志/授权 URL 尚未开始。

## 证明的问题与作用域

- 旧 `agent_budget_ledger` 每 root 一个可变汇总，只保存合并模型 token/requests；不能由它推断十维来源、未决花费或每个身份的目标成本。
- 普通 finish 曾能退回已收模型响应的未结算预留；根 fence 更新后新协调者可释放旧 child 的预留；终态回执旧汇总结清但新账本未结。
- 三路目标请求有独立真实 journal，但缺新向量来源。授权声明 INSERT IGNORE/写入时撤权可被误认为准入成功；模型与目标未知成本之间可继续派发。
- 普通任务删除只检查旧模型汇总；即使新向量仍有 unknown，只要旧汇总/状态标记结清即可删除 root 来源。
- 各 child 重试/并发没有共享根墙钟记账，过期根可取得新 child 时钟。

所有 DB/HTTP 故障注入和请求都使用唯一临时目录、临时 SQLite 与 loopback 服务。没有打开真实用户库、CAS、真实授权站点，没有清理历史行或提交。

## 最小实现

- 最新补充（覆盖下方旧逐轮/墙钟快照）：Source/专家收到实际响应时同事务写入逐轮 consume 或未决估算，最终子任务仅补差；Web 使用 `agent_web_model_journal` 的不可变 dispatch/received/uncertain/unsent 事实，I/O 前绑定原 owner/round/请求 hash，已收费用回执与新账本原子提交。回执尚未闭合则阻断新工作/退款/普通删除/成功收口；无 provider 完整分类的估算不冒充已知发票。输入+输出合计超有限子任务原预留也保持未决并保留原发票。Web journal 只存 hash 和费用元数据，不存提示词/响应文本/凭据。它不授予重放、续期或恢复工具的权限。
- 最新墙钟：所有已接线联合模型传输及 HTTP/匿名采集/授权三侧请求，以冻结根剩余时间限制整个请求；根成功收口和写后复核共享起点。超时请求保持未决原费用，root 不能报告成功。单智能体、root 自身与启动侦察仍未覆盖。
- 最新执行权限：Web 模型的实际 CancelChecker 复核 root/child 的取消标记；心跳按原身份检查未取消、未过期的 Native child，续期使用一次捕获的统一截止时间，并在同一 IMMEDIATE 事务最终验证原 owner、原能力集合与四类期限。部分续期/静默 IGNORE/写中取消全部回滚。重入前的 journal 门禁在续租事务中先执行；目前任何有既有 Web 模型声明或事件的 child 一律拒绝自动恢复续租，不能把安全拒绝记为恢复功能完成。
- `multi_agent/budget/`：不改当前 Native JSON。新 `agent_budget_entries` 为 append-only，键绑定 root/assignment/原始 epoch+fence/dimension/kind/amount/idempotency/source；limits/clock origins 也不可更新或删除。旧汇总保留作交叉校验，不从历史旧行自动补造 sources。
- 计划冻结十维上限：模型四维；目标按冻结 Web 合同总限；并发三条 lane；根 timeout；Source 目标、写入、上传、不可用宿主浏览器为零。模型显式零总限仍保持既有 unlimited 语义，其余零授权拒绝。
- scheduler 准入、assignment、lane、旧汇总、新预留同一 IMMEDIATE 事务；ABORT/FAIL/IGNORE 与写后绑定变化均拒绝。model 最终结算区分 input/cache/output/request；同总数不同分布不视为幂等。只有 total 而无可证明分类时保持 indeterminate，不补造分布。
- `target.rs` 与真实 HTTP/public/authorization brokers：发请求前 reserve→forfeit，响应头或原 artifact 回执同事务 reconcile。晚到 HTTP 只能在原 owner/fence 下记成本，不续期、不恢复 run、不重放请求。新授权 broker 按职责从旧大文件抽出，保留原身份/方法/对象/冻结计划/能力检查及真实三请求验证。
- `admission.rs` 阻止未决向量及真实 uncertain model journal 的跨维新派发；已排队专家也复核。模型失败调用把实际 call 对应估算转 forfeit，未发送 tombstone 才可释放，未知调用不能 finish 退款。
- clock origins 冻结 root 创建时点；调度/模型/目标准入采样根 elapsed，只追加比上次高水位多出的 delta，并发不重复累计。起点改变、时钟回退、超时拒绝。它目前不是终态/在途/侦察完整时钟。
- 普通任务删除检查所有 root、所有维度的 reserved/indeterminate。合法删除已结清任务后新账本保留不透明审计 ID；不依赖 root FK，也不能据此恢复已删除 run。任何真实旧数据清理仍需独立盘点/备份/确认流程。

## 红绿证据

- 新取消/重入红测：`/tmp/oviraptor-master-web-model-authority-red.log` 和 `web-model-publication-red.log` 分别证明取消未关闭真实模型传输、未结调用及费用已收但旧事件未发布的重入会先续租；对应 `web-model-authority-green.log` 10/10。心跳另外两项红测位于 `web-heartbeat-atomic-red.log`，证明已取消 child 及写中取消仍可续租；追加四张表的 ABORT/FAIL/IGNORE/写中取消矩阵。此处重新打开临时数据库并非 OS 强杀验收。
- 四表故障和取消完整切口：`/tmp/oviraptor-master-web-heartbeat-atomic-green.log` 12/12；首轮严格 Clippy 指出心跳 owner 元组类型过长，改为具名权限结构，保持拒绝和回滚合同，未降低门禁。
- 当前受影响门禁（非最终验收）：`/tmp/oviraptor-master-heartbeat-affected-budget.log` 99/99（更窄的 `budget_` 过滤为 82，不能充当整个预算集合）；同前缀 `broker_` 20/20、`source_rounds` 10/10、`source_expired` 9/9、`directive_closure` 21/21、`commands_agent_tests_e2e` 11/11、`retirement_literal_allowlist_matches` 1/1。`/tmp/oviraptor-master-web-heartbeat-clippy.log` 严格全目标全特性退出 0，`web-heartbeat-fmt.log` 格式检查退出 0；`git diff --check` 通过。集合有重叠，不求和，不沿用旧全量 Rust/UI/构建/安装态结论。
- 最新日志：`/tmp/oviraptor-master-web-journal-red.log` 证明 provider 原先无持久声明；`/tmp/oviraptor-master-web-usage-red.log` 与 `source-usage-red`、`specialist-usage-red` 证明估算误作已知费用/允许后续动作；`web-combined-red` 证明分类各不超限却合计超预留。对应新预算集合 `/tmp/oviraptor-master-web-source-receipts-budget.log` 94/94，消息收口 `/tmp/oviraptor-master-model-fees-child-completion.log` 9/9，面板 `/tmp/oviraptor-master-model-fees-ui.log` 4/4。上述局部集合重叠，编译、回环或重新打开库均不代替真实强杀和安装态验收。首次用量回归 33/34 的旧夹具分类缺失已修，原故障/回滚断言保留。
- 账本/收到响应退款/IGNORE/取消前未发送/十维上限/过期终态回执/替换 fence：`/tmp/oviraptor-master-*-red.log` 与对应 green；新预算测试位于 `multi_agent/tests/budget_journal.rs`。
- 三路请求缺成本、声明 IGNORE、写中撤权、删除未决成本、跨维未知继续：`target-budget-red`、`three-route-budget-red`、`target-authority-delete-red`、`cross-dimension-red` 日志均有实际失败，随后 green。`budget_target.rs` 同时验证请求数及失败时未出网。
- 共享 clock、过期根、改起点：`budget-clock-red/green`；测试还包括 wall INSERT 三类故障整体回滚。
- 不确定模型仍作为普通 reserve：`model-unknown-vector-red/green`，从 (60,0,0) 修为 (0,0,60)，请求仅实际一份 unknown、另一份未使用 reservation 保留。
- 预算 56、journal 38、授权 21、public 12 通过是后续模型 forfeit 之前的快照。forfeit 后 specialist journal 11/source rounds 9/已证明未发送 1/scan deletion 10 通过。后续代码变化必须重核，不沿用旧完整 Rust/Clippy/UI/build 数字。
- 追加向量诊断的 API 初始缺 `appendJournal` 实际红测，随后真实十维读数/作用域/不写 DB 为绿；坏 UI 向量可被展示的红测已修，实际 SFC 与坏回执等 4 项通过，当前构建通过。
- 首次严格全目标 Clippy 发现独立导入工具没有运行时 module 的新依赖错误及 re-export unused；schema 改为 DB 私有模块、配置更新改为明确 wrapper 后严格 Clippy 退出 0。新增诊断之后还需重核。退役字面量门禁首跑指出新 Node 负向夹具未登记，逐文件审查该夹具 2 处字面量后添加精确 hash，复跑 1/1 通过。

## 仍未完成与风险

1. root 自身与单智能体仍使用原权威，尚未加入新向量；当前十维“有上限”不等于每条费用路径完整。
2. Source/专家/Web 已有逐轮 consume/未决费用接线，仍需精确 unknown/超额费用对账及跨过期 fence 的人工核对合同。Web 费用回执与旧模型事件/用量/检查点尚分阶段发布：缺失阶段拒绝自动恢复，不代表完整恢复闭环。真实强杀矩阵未验；未闭合 Web 声明由 journal 门禁保留预留，诊断未单列这类回执数。
3. clock 已接联合模型/三路目标请求全传输限时及成功收口采样/写后检查，仍缺单智能体/root 自身、启动侦察与全部超额时长审计。静态零动作上限不是真实浏览器/XHR/WebSocket 全网络审计。合法扩预算的 grant 尚未建追加式上限事件。
4. 新向量已接按需 API/真实面板，展示 root 的十维已用/预留/未决数字和实际 writer 覆盖范围；旧汇总差额单独保留。严格校验坏向量，读 API 不泄漏来源/身份/fence。当前报告的完整审计源交付仍未完成，诊断不能当准入权威。
5. 真实全角色/模式/Reviewer/聊天/日志/强杀恢复、安装态、用户授权 URL 验收尚未完成。两个站点仅授权匿名只读，登录身份待用户后续提供。
6. 临时负测不能证明用户数据清理安全。现阶段对退役数据只拒绝执行/普通删除并保留原始行；专项精确盘点、可恢复备份与确认 UI 仍待实现。
