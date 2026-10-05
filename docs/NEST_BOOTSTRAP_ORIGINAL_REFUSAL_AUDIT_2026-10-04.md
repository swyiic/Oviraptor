# Bootstrap原恢复拒绝链审计（2026-10-04）

Master未完成，Goal active。本批将一项裸回调旧夹具迁为实际生产者，未修改生产准入、权限或费用规则；localhost脚本provider通过真实SDK，不能当作真实模型脑力或完整恢复验收。

## 问题与实际作用域

旧测试仅对三个错误字符串调用multi_agent_bootstrap_outcome，再在无原Root、费用及退出证明的数据库上裸调用record_agent_target_outcome。原消费边界正确拒绝（此前原14中bootstrap失败），不能为让测试通过补零用量Root或松权限。本批保留原测试名、三代码/状态/人工复核而非工具失败语义，以及未知storage错误仍为Failed的断言；将每种拒绝移到真正原执行入口。

每场景由实际新Multi creator、startup/HMAC、persist_frozen_web_execution_plan和runtime_open_run产生原Root/mode/十维财务合同。只读既生C1，不用测试财务发行器。真实WorkerSupervisor持有父调用，native_coordinator_tick向localhost发1次SDK，原model_requests消费1、input/output消费>0；随后释放父调用，在临时SQLite加入不完整历史记录作为拒绝负向，再进入run_agent_target。历史行没有预算、能力、SDK或退出证明，不作为正向子任务执行证据，不证明真实旧代际带费恢复。

- readonly：旧spa_api_mapper行及epoch0历史assignment，实际guard产生readonly_fencing_changed_requires_fresh_attempt。
- target：旧web_executor目标运行行，实际guard产生target_execution_recovery_requires_fresh_attempt。
- public：旧external_surface运行行，实际guard产生public_surface_recovery_requires_fresh_attempt。

三个真实返回均ResumeIncompatible/resume_incompatible，detail精确为guard代码，owned捕获原Root不变。SDK累计仍1、HTTP0，没有后续角色/隐式重试；原七张预算/时钟/SDK账本/tick/mode表先确认存在且非空，再比全部typed row/rowid不变。九个非墙钟维度的consumed/reserved/indeterminate精确保持；墙钟照真实终结采样，原FinalClock退出证明验证通过。历史run/assignment原行不重标、不清理、不赋权或补费用；没有Native可续checkpoint、漏洞或覆盖。

owned消费使目标resume_incompatible、理由包含“需要重新执行”、manual_review1/failed0/count1，checkpoint顶层及nested stop同码，原Root/目标投影一致。owned重放完整数据库所有typed row/rowid/费用保持且不重计；伪Completed、改写拒绝理由、伪失败均被实际原消费边界拒绝，所有原行/费用/tally保持。再孤立把current attempt改为2作为迟到负向，旧owned回调拒绝、原及current行保持；这不是实际重启/取消/新attempt创建验收，没有补新Root授权。

## 修改与保护

2路径1已有/1新原bytes/Gitdiff留存并全文审查两项最终增量；已有external_surface347→318，仅原bootstrap正文换include，其他字节保持；新叶261行。生产源码、Native JSON及退役allowlist未改。1326原范围外SHA保持，1328集合SHA cf760f9c318a25b696b392e7d544d4f8ea80cad8a3bc656472e3af1f8228ddac，HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b保持。证据及保护记录/tmp/oviraptor-bootstrap-original-*。无reset、批量覆盖或自动提交；仅临时SQLite/localhost，不写真实DB/CAS/资产，不安装App或访问授权URL。

## 验证及实际限制

| 阶段 | 结果 | 日志 |
|---|---|---|
| related | exit 0，48.57秒，17过/0失败 | /tmp/oviraptor-bootstrap-original-related.log |
| original14 | exit 101，44.2秒，13过/1失败 | /tmp/oviraptor-bootstrap-original-original14.log |
| clippy | exit 0，9.03秒，all-targets/-D warnings | /tmp/oviraptor-bootstrap-original-clippy.log |
| retirement | exit 0，4.93秒，46过/0失败 | /tmp/oviraptor-bootstrap-original-retirement.log |

具名related选择=reported=passed17，原14选择=reported14；13过/1失败，无ignore。含本批三场景矩阵、原Root未知费用边界、原Single/Multi发布及Native policy、原公开面handoff。严格all-targets Clippy通过，退役46含exact及当前Native JSON原字节回环；新叶局部rustfmt及diff--check通过。首次编译发现测试费用dimension临时String不满足静态生命周期，改用原DIMENSIONS的静态str，未动生产API、断言或权限；日志first保留，不计通过。第二轮原单项1/1（含三场景，编译37.40秒、测试2.72秒），补错报/迟到负向后在最终related及original14再次通过。

剩余原失败commands::agent_tests::every_target_ends_in_exactly_one_terminal_state：旧测试用裸结果和手写plan，没有原生产者/退出证明，completed投影正确未写而target仍scanning。后续需七类真实生产者、原账本/owned消费/重放，保留取消语义，不能伪造正向回执或删断言销项。6项Source Broker基线债本批未处理/重跑；扩大325中断91过/6失败/228无结果仍为历史，不推断全跑或全绿。

Master14项仍未全部满足；删除/残余活路径、动态十维预算/全通道对账/恢复、Root六类真实监督、全15角色、实际并行、证据/独立审核、完整取消/崩溃恢复、用户聊天、逐路实时日志、整体UI、Code/Greybox/CI/真实数据规模及最终门禁/安装打开/两授权URL/真实模型质量均按Master继续。InputParser原拒绝不绕过。下一步继续七终态原入口及Source Broker原始子任务，再其余框架；不把本批回归迁移当整体功能完成。
