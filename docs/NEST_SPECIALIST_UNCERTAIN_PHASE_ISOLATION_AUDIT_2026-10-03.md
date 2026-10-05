# 专家 uncertain 阶段写入隔离审计 · 2026-10-03

HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`不变。4路径2已有/2新；无重置、批量覆盖、提交、真实DB/CAS/资产/安装/URL操作。Master未完成。

## 实际问题与作用域

真实ordinary creator/HMAC/原Root/C/UUID worker到实际loopback provider503。原record_uncertain的正常事务使用调用方连接，AFTER UPDATE state触发器能将projects.status从active改为archived。首0/1（0.39秒）在项目全行比较失败；原production transport已返回Err，但错误字样/费用事实/重放/最终SDK次数尚未断言，第二budget entries触发器分支未达到。不是纯只读诊断或界面模拟。

## 最小修复

新增56行phase_writer，私有文件RW/no-CREATE、busy250ms/FKON/syncFULL；在内层BEGIN之前安装hook直至COMMIT或ROLLBACK，连接析构自动释放，绝不覆盖caller hook。只允许main/no-accessor直接UPDATE agent_specialist_calls的state/failure_code/finished_at，以及INSERT agent_budget_entries/agent_model_cost_facts；其余写入/DDL/pragma拒绝，所有trigger写入即使目标是许可表也拒绝。原record_uncertain_on整个body、原C/worker/request绑定及正常/晚费用算法不变，只将其一次调用置于独立连接。失败仍沿原受限费用fallback保留原未知义务；不自动重试/退款/补授权，不发布结果/ACK/finding。

## 验证

实际SDK两个trigger分支修后均通过：phase附带projects写被拒绝，原业务回滚而fee-only保存indeterminate1/consumed0；forfeit entries附带projects写同样拒绝，原业务与费用完整回滚并返回503/原阶段错误/次费用错误，原预留和原executing请求保持。无账单不伪造未发送；同transport重入全application rows保持、每分支实际SDK总次数1。

另一个本地contract使用真实原dispatch但不调用provider：caller打开事务被精确拒绝且不commit/改变rows，caller projects INSERT拒绝hook在调用后仍触发一次，普通unknown调用成功后变化错误重放不增费用/改rows。这是本地写入边界证明，不计真实SDK验收。application comparator排除SDK诊断表及sqlite_sequence，费用保存时允许两个金融表；不单独声称全部rowid/日志/安装状态已证明。

相关58/58（73.96秒），含原late/真实transport/日志、Source角色准入与在途取消、Reviewer未知结果重入。代码未再变化，未重复该集合；其后严格Clippy、导入39和exact退役1通过。没有全Rust/安装/URL验收。

- clippy-final：exit0，wall20.77秒；`/tmp/oviraptor-specialist-phase-isolation-clippy-final.log`。
- importer-final：exit0，wall32.87秒；`/tmp/oviraptor-specialist-phase-isolation-importer-final.log`。
- literal-final：exit0，wall1.46秒；`/tmp/oviraptor-specialist-phase-isolation-literal-final.log`。

集合1163 SHA `f00203536e17d90f9d561c75df52253ea2a49100b61eaae6e51e7933de25bc97`，1159原范围外保持；before/prior/reviewed-merge-diffs在`/tmp/oviraptor-specialist-phase-isolation`前缀，逐文件全文已审查，diff --check0，新叶独立fmt。tests SHA2ab7cd1e、production SHA5a9a7e80。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 129 | 否 |
| `src-tauri/src/commands/agent_tests_specialist_phase_isolation.rs` | 90 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/specialist.rs` | 356 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/phase_writer.rs` | 56 | 是 |

## 未完成与风险

notsent与普通/晚received仍用原caller writer，需要各自精确权限和业务回执证明；本批只隔离uncertain（含晚Uncertain fact）。非法usage、响应/持久化间强杀、精确provider对账/显式继续、动态预算、全角色/Broker/真实并发、人工聊天、整体UI/旧九E2E/正常paid删除、最后门禁/安装/URL均未完成。原clock失败误调用字符串notsent仍要按typed before-transport单列；不得拿此隔离冒充已修复。InputParser既有自动审批拒绝保持，不重试或绕过；Goal active。
