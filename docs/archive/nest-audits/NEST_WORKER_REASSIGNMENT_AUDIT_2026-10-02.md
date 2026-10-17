# Worker 显式替换、Web 启动恢复与强杀边界审计（2026-10-02）

Master 尚未完成。本批只落实同一Coordinator的无派发替换、Web Mapper/Identity入口和原回执强杀边界，不是后台Supervisor、完整恢复或整体验收。

## 问题、权限和数据作用域

- HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 未变；基线848路径为Source unsent最终批。逐文件保存既有文本，保留其他未提交改动；没有重置、自动提交、真实DB/CAS/资产、安装包或授权URL操作。
- 起点scheduler只能ordinal1。实际初始红测1/2：合法过期无派发worker无法替换。临时库使用生产原子过期API，没有手工复制新worker或能力作为重派证据。
- 数据仅当前Root/assignment/child/原attempt的Native审计、模型配额及槽位；Root同级权限/业务/费用原行、原worker/run/能力和时钟均完整比较。没有资产表读取或CAS操作。新替换事实不回填历史记录、无FK级联删除权限。

## 最小实现

- 专门reassign_undispatched_expired入口先核验当前Coordinator、冻结角色合同/内容、已撤权原审计、无任何派发或业务效果、精确原十维余额及Root未决/限额/时钟。普通schedule_child不会续租、修复或隐式替换。
- 当前入口允许Web SpaApiMapper、IdentitySession、DeepInvestigator的无效果上下文；不能因角色枚举存在就重放已有消息、HTTP或工具效果。原子释放原四维模型预算/槽位后创建独立UUID、worker、fence、新child、递增ordinal；Root汇总预留量、lane/contract与clock origin不变。
- 新agent_assignment_replacements绑定原/新attempt、Root/assignment/当前Coordinator、完整原审计与冻结计划哈希，禁止update/delete/replace。调度只读重放核验全链；旧费用/worker/能力全列保持，Root共享数据与新grant/template/heartbeat/预算追加序列/outbox最后核验。
- 原模型/HTTP等durable claim、unknown/收费/消息/证据效果均拒绝此替换；不能把absence of response当absence of dispatch。跨Coordinator不采用旧预算或结果。
- Web prepare实际调用独立prepare_supervised_readonly_child：原请求的完整冻结预留量匹配，活worker只读验证，received只本地交付，已签发prepared worker仅同一身份启动；过期无派发才进入替换。没有model错误重试循环或恢复target执行。
- 公共角色合同函数按原块抽取，child grant仅参数化已有创建代码；共享Rows从原expiry proof按原块抽取，只有可见性改变。抽取原文和参数映射存于 `/tmp/oviraptor-worker-reassignment-extraction.json`。第一次grant抽取误改root_run_id，编译实际拒绝，已修；抽取后3/3只覆盖两个child-start与Source生产入口，最后扩大回归覆盖更多。

## 实际失败与验证

- `/tmp/oviraptor-worker-reassignment-red.log` 1/2，缺生产替换入口。
- 首次grant抽取编译错误和balance_for_attempt可见性编译错误均保留日志，修正后初始合同2/2。
- `/tmp/oviraptor-worker-reassignment-final-write-red.log` 0/1复现最后写入篡改新worker费用；模板核验后 `/tmp/oviraptor-worker-reassignment-heartbeat-red.log` 0/1复现心跳篡改。最终完整fresh grant核验修复两者。
- 新增真实localhost替换模型/业务结算、两次递增替换、22种IGNORE/ABORT/共享与原审计篡改、并发替换一次、不可变来源/损坏旧历史拒绝、跨Coordinator拒绝。接线前9/9与扩大217/217是中间证据，不能当最终结果。
- 接线前严格Clippy实际exit101揭示API无应用调用者，未压警告；新增Web入口后 `/tmp/oviraptor-worker-reassignment-bootstrap-red.log` 0/1实际传输仍使用过期worker，随后修复。 `/tmp/oviraptor-worker-reassignment-bootstrap-budget-red.log` 0/1额度漂移被接受； `/tmp/oviraptor-worker-reassignment-ready-red.log` 0/1已签发prepared替换无法继续；分别先红后补完整合同和同一worker启动。
- 最新 `/tmp/oviraptor-worker-reassignment-kill-contracts.log` 15/15（5.65秒），含13项合同、一个无env时零工作的probe和一项三相真实SIGKILL。
- SIGKILL严格检查signal=9，stdin独立writer保持至死亡，不靠Drop/EOF/panic冒充。before_claim可替换后真实localhost传输和业务结算一次；after_claim即使未真正发送HTTP，也因缺NoSend证明保留原预算/槽位且拒绝新调用；after_receipt是真实HTTP保存回执后被杀，仅本地结清/ACK一次、原worker保持expired。使用受控deadline夹具后生产撤权，不能写成自然TTL、后台监控或安装App强杀证明。所有probe日志/使用目录限定本次临时root并清理。
- 最后格式/临时目录限定后 `/tmp/oviraptor-worker-reassignment-final-affected.log` **223/223、exit0（131.39秒）**； `/tmp/oviraptor-worker-reassignment-web-consumers.log` **24/24、exit0（13.12秒）**，包括真实四角色独立传输、恢复/错误费用/公开面与orchestrator消费者，集合有重叠，不相加。当前主库1703项，未跑最终全库。
- `/tmp/oviraptor-worker-reassignment-clippy-final.log` 严格全目标/全特性Clippy exit0（21.61秒）； `/tmp/oviraptor-worker-reassignment-importer.log` 导入工具39/39（2.31秒）；22文件fmt、差异空白及860路径最终哈希一致。

最后快照 `/tmp/oviraptor-worker-reassignment-code-snapshot-final.json` 摘要 `7f5b28b1494b2e12f3cc723528bb4759501fbab778075b9e9c685925f56b73c6`。857路径中间快照被最后860路径覆盖；基线、修改前文本、scope及逐文件差异均在 `/tmp/oviraptor-worker-reassignment-*`。10已有代码文件、12新增，无旧代码文件删除或旧测试移除。

## 体量与归属

| 文件 | 行数 |
|---|---:|
| `src-tauri/src/agent_runtime/multi_agent/attempts.rs` | 171 |
| `src-tauri/src/agent_runtime/multi_agent/attempts/audit_rows.rs` | 63 |
| `src-tauri/src/agent_runtime/multi_agent/attempts/expiry_proof.rs` | 207 |
| `src-tauri/src/agent_runtime/multi_agent/attempts/schema.rs` | 58 |
| `src-tauri/src/agent_runtime/multi_agent/budget.rs` | 354 |
| `src-tauri/src/agent_runtime/multi_agent/budget/entries.rs` | 92 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler.rs` | 33 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/assignment.rs` | 236 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/child_contract.rs` | 140 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/child_grant.rs` | 54 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/reassignment.rs` | 135 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/reassignment/lineage.rs` | 102 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/reassignment/proof.rs` | 216 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/reassignment/undispatched.rs` | 70 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/schedule_authority.rs` | 143 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/supervised_bootstrap.rs` | 91 |
| `src-tauri/src/commands/agent_tests_multi_agent.rs` | 52 |
| `src-tauri/src/commands/multi_agent/prepare.rs` | 313 |
| `src-tauri/src/commands/multi_agent/tests/worker_reassignment.rs` | 235 |
| `src-tauri/src/commands/multi_agent/tests/worker_reassignment_bootstrap.rs` | 118 |
| `src-tauri/src/commands/multi_agent/tests/worker_reassignment_faults.rs` | 174 |
| `src-tauri/src/commands/multi_agent/tests/worker_reassignment_kill.rs` | 247 |

所有本批手写文件≤400行；本批未触及超大静态db_schema债务。

## 未完成和风险

- 只有三个Web只读角色的无派发显式替换，Web自动准备仅Mapper/Identity。已有费用/消息/typed unsent、Source轮次、canonical Reviewer、target工具与跨Coordinator generation恢复仍不支持；未知不得退款/重试。
- 未实现后台Supervisor和全角色自然超时/进程失联矩阵；本批真实强杀仅同一Coordinator存活的测试worker subprocess，不是生产应用已经启动OS角色worker或完整桌面kill/restart验收。
- Root/Single全部账本、动态grant/精确对账、真实全角色/模式、复杂聊天与工具闭环、AST/浏览器/进程逐路日志、真实旧数据盘点备份清理和最终全量/安装态/授权URL继续开发。旧blocked Goal工具记录不等于项目完成，不据局部绿色标Master完成。
