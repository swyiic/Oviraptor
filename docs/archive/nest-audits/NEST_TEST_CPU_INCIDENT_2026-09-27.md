# 测试进程 CPU 异常处理（2026-09-27）

## 结论

用户报告 `oviraptor_lib-4066a088024b7cd5` CPU 达到 360%。核对命令确认它是本次开发启动的 Rust 测试二进制，不是生产扫描任务。测试夹具存在已确认的后台监听线程泄漏；不能仅凭暂停后的采样，将全部 CPU 百分比精确归因于这些线程。

## 2026-09-29 23:34 运行中抽查（非旧事故根因结论）

终态补充：session **39098** 已返回 **exit 0**，主库 **1443/1443**（1566.31 秒）、导入器 **30/30**（2.39 秒）；原 PID 6664/6744 已不在进程表。没有重启第二个 Cargo，未进行全程资源峰值记录，不能据此声称旧 360% 根因已完全解决。

23:34 抽查当时完整门禁 session 39098 尚未退出；Cargo PID 6664 的子进程 PID 6744 为 `target/debug/deps/oviraptor_lib-4066a088024b7cd5`，由单构建任务、单测试线程、nice 15 命令启动。进程表该次读数约 99.4%，不是长期峰值或 CPU 硬上限。日志 `/tmp/oviraptor-csv-sidebar-rust-full.log` 持续推进，不因观察等待而重启。

只读的一秒采样 `/tmp/oviraptor-rust-tests-cpu-sample.txt` 记录时间为 23:34:37 +0800，包含主线程、当前测试线程及一个测试 HTTP 监听线程；每条栈仅四个样本。当前测试栈出现取消状态读取中的 `db::open` / SQLite schema 读取，但如此少的样本不足以认定持续热点，更不能解释旧 360% 或证明没有线程泄漏。没有基于该抽查修改数据库/取消门禁、终止进程、启动第二个 Cargo 或改动生产扫描路径。采样命令正常退出；完整测试结果另行收取。

## 2026-09-29 当前工作树全量与资源复核

全量 Rust 使用 `nice -n 15 cargo test --offline -j 1 --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1`；session 70643 已收取 **exit 0**：主库 **1435/1435**（1671.15 秒）、历史导入器 **30/30**（2.30 秒），main target 零测试。日志 `/tmp/oviraptor-20260929-full-quality-rust.log`。Cargo PID 96047、测试 PID 96061 均已退出；没有重启或并行启动第二条 Cargo。运行期间没有修改 Rust 源码，只修改聊天前端、测试夹具和文档。

全量结束后串行执行 `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` 和低优先级、离线单作业、全目标/全特性的 `cargo clippy ... -- -D warnings`，均 exit 0；Clippy 日志 `/tmp/oviraptor-20260929-full-quality-clippy.log`。最终 UI **413/413**、类型检查与生产构建、差异空白检查通过。这些是当前开发工作树的质量证据，不是 Master Plan §18 全部发布条目的完成证明。

运行中线程抽查为 3 个；CPU 点采样出现 21.8%、78.1%、92.3%、100%，后续另一次 89.6%。这不是连续峰值记录或 CPU 硬上限，也不是全任务资源验收。历史事故中 532 个累积线程的观察不能套用到当前进程。

本地 native runtime 浏览器夹具正常退出，匿名/比较采集 complete、身份隔离 true，观察到 8 个请求，日志 `/tmp/oviraptor-20260929-full-quality-browser.log`。聊天夹具原先因 composable 拆分失去传输隔离、缺少分页元数据，现已修复。进一步真实浏览器发现的旧页缓存淘汰跳动也已修复；有界快照、历史导航和显式已读边界见 `../tools/chat_dom/README.md`。最终前端结果以该记录及 `/tmp/oviraptor-chat-anchor-{ui,build}.log` 为准；不把夹具检查视为真实 Tauri/模型/数据库或安装包验收。

结构盘点范围 `src`、`src-tauri/src`、`tools` 的 `.rs/.ts/.vue/.cjs/.js`：605 个文件中 109 个超过 400 行（不包含 `.mjs`、文档或资源 worker 树）。本次聊天修复继续放在现有业务模块，未新增依赖/公共层/测试入口，保留独有回归；这不代表既存大文件已清偿。Master Plan §0.2 继续约束前后端和测试，整体目标未完成。未部署或访问外部目标，未移除授权、范围或预算门禁。

## 2026-09-27 源码证据页阶段的后续修复

重新核对 `/tmp/oviraptor-reentry-cpu-sample.txt`：两条残留 `accept` 线程准确地说分别是 `spawn_redirect_server` 和 `spawn_egress_canary`，不是两条重定向服务。两者已经接入调用方所有权控制的共用 HTTP 夹具；重定向仍返回真实 Location，出网哨兵记录请求并返回 502，绝不建立 CONNECT 隧道。新增释放监听端口回归，保留原重定向拒绝、代理正向自检及被禁域名断言。

完整回归 session 66703 已正常退出并收取 exit 0，约于 2026-09-27 18:46 结束；主库 1195/1195（676.89 秒）、历史导入器 30/30（2.27 秒），主程序 target 零测试。日志及每两秒的 Cargo 后代资源采样在 `/tmp/oviraptor-source-findings-full.log`。358 次采样中，测试进程最大读数 100%，包含编译阶段的进程树总和最大约 247.1%。这既不是硬限额，也不能排除采样间隔内峰值。

两类行为断言和新增释放测试在该全量中通过；中途两次线程抽查为两个线程，记录 `/tmp/oviraptor-source-findings-thread-sample.txt` 和 `/tmp/oviraptor-source-findings-thread-sample-late.txt`，另一次观察到三个线程，不代表所有时刻的上限。收尾进程检查未见残留 `oviraptor_lib`、Cargo test 或 rustc；没有再启动第二轮全量。最后的前端 165/165、类型检查及生产构建也通过。详细功能和验收边界见 `NEST_SOURCE_FINDINGS_READ_AUDIT.md`。

## 2026-09-27 18 时后复核

旧异常 PID 已不存在。同名后续测试 PID 72578 由 Cargo 72458 启动，使用 `nice -n 15`、`-j 1`、`--test-threads=1`；session 78274 已正常收取测试失败退出（exit 101），没有因高 CPU 强杀。主库 1187 通过 / 2 项登记失败，661.34 秒；两项登记修正后定向补测均通过，完整导入器另外 30/30，严格 Clippy/fmt 通过。没有马上再开全量回归，不将补测写成单轮全绿。详情见 `NEST_SOURCE_REVIEW_REENTRY_AUDIT.md`。

本轮全量日志 `/tmp/oviraptor-source-reentry-full.log` 含 340 次、约每两秒一次的资源采样：测试进程最高读数 100%，当时可见 Cargo 后代进程树总和最高 100.1%。不是 CPU 硬上限，也不能排除采样间隔内峰值。18:02 的线程抽查为 4 个，不是旧事故的 532 个；`/tmp/oviraptor-reentry-cpu-sample.txt` 仍看到两条旧重定向夹具线程阻塞在 accept，需后续清理，但本次样本不能将它们认定为高 CPU 原因。没有修改生产扫描门禁，也没有访问授权目标 URL。

后续段落均为此前事故处置及增量历史，不得用旧版完整回归替代当前版本结果。

收尾核对：`git diff --check` 通过；另检查 290 个未跟踪的 Rust/Markdown/JSON 文本，除 Master Plan 开头原有的 4 处 Markdown 双空格换行外，无尾部空白问题，保留既有换行。最终进程表未发现残留 `oviraptor_lib`、Cargo test/clippy/fmt、rustc 或 clippy-driver。没有删除用户文件、提交或推送。

## 处置和证据

- cargo PID 47438，测试子进程 PID 47482，session 46549；启动命令为 `cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1`。
- 先向两者发送 SIGSTOP；复查 CPU 均为 0%，子进程累计 CPU 时间保持 33:50.94。
- `/tmp/oviraptor-test-cpu-sample.txt` 中记录 532 个线程条目，其中 528 个落在旧 `agent_tests_harness.rs:106` 的 1 ms 轮询等待处。旧监听线程自己强持有停止句柄，调用方丢弃句柄不能自动结束监听。
- 再次核对精确命令后终止旧测试子进程，并恢复 cargo 以收取退出状态；session 46549 已 exit 101，两 PID 均已消失。该次运行属于人为中断，不能视为测试通过或新增断言失败。
- 保留 `/tmp/oviraptor-source-surface-full-final.log` 和采样；未删除项目文件或扫描产物，未停止无关应用，未进行外部扫描。

## 修复和验证边界

`src-tauri/src/commands/test_http_endpoint.rs` 提供共享的 std-only 测试 HTTP 夹具，原 `agent_tests_harness.rs` 已接入。监听和请求线程使用弱引用；调用方持有 stop 或请求历史任一个时维持服务，两者均释放后退出；显式 stop 优先。空闲轮询由 1 ms 改为 10 ms。保留只持有历史句柄的既有 harness 语义。

直接用 rustc 编译该源文件并低优先级运行，5 项生命周期测试通过；本轮再次运行 5/5 通过，耗时 0.07 s。覆盖监听 socket/handler 释放、只持有历史/stop、显式停止，以及 32 个夹具释放。任意请求 handler 自身阻塞的行为不属于本修复已证明解决的范围。

后续已重新编译实际工程，`http_endpoint::tests` 五项通过，日志 `/tmp/oviraptor-http-endpoint-integration.log`；严格 Clippy session 19381 也已 exit 0。源码相关回归 session 96167 串行、低优先级执行，运行中两次线程抽查均为 3 个，未见旧夹具的累积形态；仍非完整全量资源验收。`--test-threads=1` 不会回收泄漏线程；单作业和低优先级也不是 CPU 硬上限。禁止把这 5 项夹具测试当作全工程验收。

## 修复后完整回归终态

2026-09-27 已收取 session 41293：exit 0。以 `nice -n 15`、Cargo 单作业及单测试线程运行，主库 **1162/1162** 通过（496.76 s），历史导入器 **30/30** 通过（2.21 s），main target 无测试。日志：`/tmp/oviraptor-full-after-fixture-lifecycle.log`。收取后进程检查未发现残留 `oviraptor_lib`、Cargo test 或 rustc 进程。

该完整回归覆盖 HTTP 夹具生命周期修复及前序审查材料/身份修复；**不覆盖运行期间新增的 `source_review_contract` 代码**，该部分须另行验证。运行中的抽样不能替代全程 CPU 峰值监测，也不承诺以后编译/测试 CPU 始终低于某个百分比。

新增合同后续定向复测 session 17752 六项通过；严格 Clippy session 72427 exit 0，fmt/空白检查通过。新增合同首轮失败与修复单独记录在材料审计中，不混入 CPU 事故的完整回归结果。本轮不再追加全量测试。

## 同期源码 Reviewer 后续

最新后续（2026-09-27）：v2 真实 Reviewer 模型/assignment/ACK/聊天接线及心跳写后复核已经完成本轮验证。低优先级、单作业、单测试线程全量 session 51972 已 exit 0：主库 1172（526.88 s）、历史导入器 30（2.22 s）；日志 `/tmp/oviraptor-reviewer-heartbeat-full.log`。运行中两次抽查为 4 个线程；退出后未见残留测试/Cargo/rustc。没有进行全程 CPU 峰值记录，不能承诺硬上限。UI 155、构建、localhost 浏览器、严格 Clippy/fmt/空白检查通过。正式决策与 CI 消费仍未完成，详见 `NEST_SOURCE_REVIEW_EXECUTION_AUDIT.md`；下段为材料阶段历史。

生产源码收口已增加 receipt-bound review material，绑定 analyzer 来源、图候选、作者和工具回执。后续集成发现自然键算法不一致导致合法候选被拒绝，已修复并增加独立旧身份兼容回归；session 4250 四项通过，Clippy 的 `clone_on_copy` 修正也已验证。真实失败和最新验证范围见 `NEST_SOURCE_REVIEW_MATERIAL_AUDIT.md`。独立 Reviewer 的真实 assignment、模型裁决、ACK 和 CI 资格仍未完成，不因材料快照落地而宣布完成。
