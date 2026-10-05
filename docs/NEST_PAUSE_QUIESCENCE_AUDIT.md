# 暂停真实性与旧尝试执行所有权：实现及验收

状态：暂停/重试可靠性增量，验证结果见第 5 节。不是完整未知效果结案、自动恢复、所有进程强杀或 Master Plan 完成声明。衔接 `NEST_REQUEST_OPERATOR_REVIEW_AUDIT.md`。

## 1. 发现的问题

旧暂停入口在发出停止信号后立即把任务写成 `paused`，并清空 `sentinel_processes`。状态标签不能证明 Web pipeline、前端生产线程或脱离生产线程运行的 native recon 已返回。旧 PID 也不能证明当前仍属于本次执行。于是 UI 可能显示已暂停、允许新尝试，但旧请求仍在途。

普通进度更新过去只保护 `pausing + scanning`，晚到的其他状态仍可覆盖暂停。通用 attempt 同步忽略 SQL 失败，也不足以作为暂停已落盘的证明。

## 2. 生产路径变化

- 暂停请求使用 lifecycle 所有权与 IMMEDIATE/FULL 数据库事务，先记录 `pausing`，不声称已经退出，不按历史 PID 发送信号。
- Web/source 分支使用原有真实 invocation 所有权。前端 producer 与其脱离队列运行的 native recon 新增独立 RAII 所有权；只有实际执行函数返回才释放，不能由 producer 代替释放。
- 新 producer/recon 领取所有权后检查当前 attempt；native recon 在线程真正开始时再检查。已有在途调用通过原有取消/超时机制退出，本次不把入口检查声称为每一个网络请求的完整原子撤权。
- `finish_sentinel_pause` 只收口精确的当前 `pausing` attempt；检查当前及历史 attempt 的 Web/source 分支、producer、target、native recon 所有权，包含只剩在 `agent_runs` 中的历史 URL。
- 有进程登记或 cleanup 尚未 confirmed 的容器回执时拒绝收口，保留原记录。不能把没有登记等同为任意宿主机/浏览器后代进程全部清理完毕。
- 最后一个 worker 释放实际锁后尝试收口。收口通过数据库事务串行化，不争抢 UI lifecycle try-lock，避免“用户正在暂停时最后退出回调恰好丢失”的竞态。
- 普通 `sentinel_scan_update` 不再改写 `pausing/paused`，也不会顺带刷新它们的 attempt。显式恢复仍走专用新尝试入口。
- Web 启动、工作台复用启动、熔断区返回原 Web 任务的路径检查旧 worker 所有权；锁在相应 admission/publication 阶段保留。移除这些重试路径的无条件进程登记删除；底层 Web retry helper 也拒绝未确认清理记录。
- 暂停 UI 改为“请求已接收，等待执行线程退出”，明确未知效果不被撤销、请求不自动重发。`pausing` 时仍可再次请求检查；不能把该按钮描述成已实现的强杀/对账。
- native 分支诊断与真实聊天投影区分 pausing 和 paused；worker 尚未释放所有权时只显示正在等待退出，不提前宣称已经暂停。

## 3. 数据与故障约束

暂停 request/finalization 均在事务中验证 scan 与 attempt 的实际写入；attempt 缺失、静默忽略、SQL 拒绝或读回不符均回滚。attempt 0 仅保留旧任务没有 attempt 记录的兼容，不为正数缺失记录补造历史。

在任何状态修改前捕获期望的状态、阶段、checkpoint、stop reason、用量差值及时间戳，写入后再次验证。已经记录的 stop reason、对应 checkpoint 和 finished_at 不由暂停清空。用量差值按持久化累计值与起始值计算，不退款。

保护摘要覆盖 scan、全部 attempt、targets、branch/dispatch/binding、agent runs/budget/request claims/operator reviews、process/container receipts。只允许当前 attempt 的明确状态投影发生变化；历史 attempt 不排除任何字段。触发器改写当前用量、既有停止原因、历史 attempt 或目标结果均必须导致回滚。

摘要是这些明确表的防回归校验，不是对数据库所有表或任意触发器行为的全覆盖证明。暂停不撤销目标已发生的效果，不结算未知请求，不解除执行授权停止，不派发请求，不启动模型，不自动重放。

## 4. 测试覆盖与仍需继续

专项用例包括：五类单独存活所有权；最后一个 worker 才收口；真实 localhost HTTP 工作线程比 producer 更晚退出；旧 attempt/仅 agent_runs 中的历史目标；部分锁获取失败后的释放；数据库 ABORT/IGNORE 与 AFTER corruption；进程/容器记录保留；精确 attempt 和丢失 attempt；晚到普通状态更新；UI lifecycle 锁与最后回调竞态；真实 Web/source guard 在 armed/disarmed 情况下释放锁后收口；既有停止历史保留。

localhost HTTP 测试使用真实阻塞请求与生产 `ScanWorkerOwner`，不是完整真实目标扫描或真实浏览器暂停验收。已有 helper 取消/pipe-drain、branch、历史导入和启动测试继续作为全量回归运行。

后续必须完成：

1. dispatched/未知效果任务的完整人工结案回执与新任务交接，不把暂停或人工 attestation 作为可重放依据。
2. 全通道请求/浏览器/代码执行统一停止与回执、崩溃重启、真实 sandbox/browser 后代进程生命周期及清理失败处理。后续删除增量已移除任务删除入口按登记 PID 停止和删除文件的行为，改为无活所有权/未结义务后的数据库事务删除，并修复后台同步绕过；范围及验证见 `NEST_SCAN_DELETION_AUDIT.md`。它不是强杀或磁盘清理能力。
3. Workbench/source 启动全部副作用的原子化与故障注入；本次只补旧所有权准入，不声称其已等价于 Web startup 的完整事务。
4. 大量历史 attempt/target 的锁枚举性能与文件描述符上限。当前按历史集合做保守检查，取得锁失败会拒绝收口，不会降级为假已暂停；仍需优化并压测。
5. Stage 9 新合同采证/revision/再审、Stage 10 剩余专家、资产/知识/skills 生命周期、沙箱供应链、UI 组织和安装包/真实目标验收。
6. Web→主机语义边界与审批后端：没有新增或启用 Host Agent，没有访问外部授权 URL。

## 5. 验证记录

最终暂停版本包含 11 项新增专项测试；`/tmp/oviraptor-pause-quiescence-authoritative-full.log` 记录主库 940／导入器 30 项通过，包含最后的诊断文案修改。该链式命令后续 Clippy 在编辑下一增量时遇到尚未创建的 include 文件，整体退出 101，不能表述为该组合全部通过。文件就绪后严格 Clippy/fmt 已独立通过；后续删除增量包含所有暂停代码，最新完整组合验证统一见 `NEST_SCAN_DELETION_AUDIT.md`。

暂停轮前端最终 117 项通过、build 通过（主 JS 811.92 kB，既有 chunk 警告保留），对应进程退出码为 0。后续删除 UI 的 120 项、构建与 localhost 浏览器回环证据见删除审计。没有访问外部授权 URL。

REM-012 仅更新已逐处复核的 `tests_scan_lifecycle.rs` 和 `SentinelBoard.vue` 两项完整内容摘要/理由，旧后端字面量仍分别是历史测试夹具和只读显示标签；没有新增豁免或放宽活动后端规则。

日志前缀：`/tmp/oviraptor-pause-quiescence-*`。首轮与最终运行必须区分，不能用前一版结果覆盖后续修改。
