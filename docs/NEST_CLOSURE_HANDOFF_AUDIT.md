# 人工结案后的独立 Web 草稿交接审计

状态：已实现；最终组合主库 979、历史导入器 30、实际 SFC 143 项通过，严格 Clippy/fmt、构建与空白检查通过。首轮派发绑定偶发失败仍保留为未解释风险，详见第 5 节。不是远端效果已核实、原预算已结清或整个 Master Plan 已完成的声明。

## 1. 用户流程与权限边界

已派发 Web 任务人工结案后，用户可从原任务状态区进入“准备独立关联任务”。系统只读取原结案与精确目标集合，工作台只预填目标和中性任务名称；用户重新选择目标子集、身份、skills、指令与正数预算，并明确确认。

提交只生成 `draft`、attempt 为 0 的独立 Web 任务。它不启动 worker，不重放旧请求，不继承旧身份、能力租约、预算、run 或未完成指令。新的真实执行仍须经过已有控制组登记和明确启动流程。原来的未知效果和费用占用继续保留。

每个原结案目前只允许一个规范交接任务。再次进入可找回草稿；如果它已启动，只进入该任务详情，不重新启动。其他新任务可使用普通创建入口，但不能冒充第二个规范交接。提交后不提供撤销关联或物理删除；允许按现有入口归档。

## 2. 后端合同

- `preview_web_closure_handoff` 核验原结案回执与真实事件，绑定 task、attempt、closure、来源摘要、项目与精确目标集合。
- `create_web_closure_handoff` 要求明确确认、UUID、有效来源、明确任务名、有效模式/策略、正数有限预算（最多 10000 USD）、新身份范围。目标必须是原目标中非空、无重复的精确子集。
- 调用普通 Web 草稿创建函数，保留其项目活动状态、身份范围和目标熔断检查；不允许过滤/规范化静默改变用户所选目标。尾斜杠按来源保留。
- IMMEDIATE 事务内原子保存新草稿与不可变 `native_web_closure_handoffs` 关联；IPC 使用 FULL 同步。触发器拒绝 UPDATE、DELETE 及 REPLACE 绕过。
- 同 requestId、同输入的重放返回当前已保存任务；不再次消耗身份、不再次创建或启动。不同请求/不同输入不得另建关联任务。重启后只读预览可以找回规范交接。
- 回执严格核验来源、关联、时间与持久摘要；发现缺失、损坏或跨任务关联拒绝读取和重试。来源前后快照一致，只有精确匹配的新增交接事件被排除于该次保留比对之外；事件本身独立严格核验。
- 新旧任务的物理删除受到保护。状态 API 返回真实 predecessor/successor 关系，链式交接不伪造原任务已结清。

返回回执包含 9 个字段：`requestId/sourceScanId/closureId/sourceHash/scanId/createdAt/executionGranted/sourceExecutionSettled/scan`。其中执行许可与原执行结清标志必须都是 false；`scan` 是当前任务投影，不能硬编码成永远未启动。

## 3. 真实聊天与界面恢复

写入关联记录时，通过数据库触发器在来源任务原 attempt 写入一条真实 `closure_handoff` 事件。payload 绑定新 scanId、closureId 和两个 false 标志。时间线明确显示“独立任务交接 · 仅创建草稿”，不显示成扫描完成或未知结果已消除。新任务通过关联关系显示来源，不补造第二条聊天事件。

读状态与回执时，在游标过滤前验证唯一事件及 scope/attempt/entity/payload；无关联记录却有交接事件也拒绝。历史游标已越过损坏事件不能掩盖错误。

工作台保存使用单飞、固定输入与 UUID。响应不确定后，用户再次保存先读取既有交接，再对同一冻结请求明确重试。切换任务、切换来源、卸载、迟到响应不得引导到错误任务；确认前禁止保存，交接模式不出现直接启动按钮。Native 状态区和团队聊天遇到交接完整性错误清空缓存，迟到旧成功响应不能恢复未经核验的内容。

## 4. 回归覆盖

8 项后端测试覆盖：

1. 独立草稿、旧记录保留、无执行记录、幂等与重启恢复、已启动后不重放、删除/替换拒绝、前后关联及真实全量/增量事件。
2. 明确配置、UUID、确认、预算、身份、精确目标和过期来源。
3. 故障注入：忽略/中断关联写入、篡改旧预算、新草稿状态或目标、丢失事件时事务回滚。
4. 两个并发客户端只能生成一个规范交接。
5. 精确保留尾斜杠，拒绝同键改输入。
6. 持久回执关联损坏时状态读取/重试拒绝。
7. 归档项目与熔断目标不可绕过。
8. 游标之后的缺失、重复、跨 scope 或矛盾事件仍拒绝，无补造写入。

实际 SFC 新增工作台 5 项、状态区 1 项、聊天 1 项测试，覆盖确认/仅保存草稿、冻结重试、重新挂载找回、错误回执、重复点击、迟到响应、来源匹配、已启动任务只打开详情、真实模板及完整性错误后的缓存恢复。

中间失败如实保留：后端测试曾把数据库文件而非目录传给 `db::initialize`，5 项中 1 项失败；修正 fixture 后通过，未放宽生产条件。前端渲染 fixture 曾漏传 props，142 项中 1 项失败；传入真实 props 后复验。新增交接聊天 fixture 首次漏传 `stopDiagnostic`，143 项中 1 项失败；补齐实际状态结构后 143 项通过，未改变生产条件。前一人工结案全量曾因两项历史显示文件的审核摘要过期而 970 通过/1 失败；已复核具体历史标签及新导航语义后更新该两项摘要，没有扩大允许清单范围。

## 5. 验证记录

定向后端：`closure_handoff` 8 通过、971 过滤，进程退出码 0。实际 SFC 143 项全部通过；`npm run build`（vue-tsc 与 Vite）退出码 0，主 JS 828.06 kB，>500 kB 拆包警告保留。

最终组合（运行期间冻结 Rust 源码，不并行重编译）：

- `cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1`：主库 979 通过、0 失败、0 忽略（413.36 秒）；历史导入器 30 通过、0 失败（2.00 秒）；main 测试目标 0 项正常退出。
- `retirement_literal_allowlist_matches_reviewed_sources_and_packaged_inputs` 和 `strix_residual_baseline_matches_the_source_exactly` 均通过。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`：通过（9.70 秒）。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --all --check`：通过。
- 上述串行组合进程最终退出码 0。实际 SFC/构建组合退出码 0；`git diff --check` 通过。

首次组合主库为 978 通过/1 失败（387.49 秒、退出码 101），唯一失败是 `web_binding_live_dispatch_rechecks_real_settings_and_owns_exactly_one_claim` 恢复配置后的 claim 返回 `web_binding_inputs_changed`。这一轮没有并行 Rust 源码修改/重编译，不能用先前构建扰动解释；后续导入器/Clippy/fmt 因串行停止而未执行。已增加仅输出变化字段路径、不输出凭据值的测试诊断，独立重跑该测试通过（55.92 秒），最终带诊断的全量组合也通过。原因尚未证实，不能把复跑通过当作已修复偶发问题；没有修改生产派发合同、跳过测试或移除输入校验。后续若再现，应继续用字段级证据定位，不自动重试真实派发来掩盖拒绝。

本地真实浏览器回环通过：匿名采集完整、8 次观测请求、身份隔离对比完整；测试断言写请求未派发，跨 origin 子资源及重定向没有到达第二个本地服务。该测试仅访问 localhost，不是外部目标或桌面 E2E 验收。

本机临时日志（不是随仓库分发的永久工件）：

- `/tmp/oviraptor-closure-handoff-event-targeted.log`
- `/tmp/oviraptor-closure-handoff-combined-full.log`
- `/tmp/oviraptor-closure-handoff-combined-clippy.log`
- `/tmp/oviraptor-closure-handoff-combined-fmt.log`
- `/tmp/oviraptor-closure-handoff-combined-ui.log`
- `/tmp/oviraptor-closure-handoff-combined-ui-final.log`
- `/tmp/oviraptor-closure-handoff-combined-build.log`
- `/tmp/oviraptor-web-binding-diagnostic.log`
- `/tmp/oviraptor-closure-handoff-verified-full.log`
- `/tmp/oviraptor-closure-handoff-verified-clippy.log`
- `/tmp/oviraptor-closure-handoff-verified-fmt.log`
- `/tmp/oviraptor-closure-handoff-local-browser.log`

## 6. 未完成范围与下一步

- 不是费用/远端副作用结算；原执行未知义务继续保留。
- 不是完整 Stage 9 新合同采证、证据 revision 回写与独立再审闭环，也不替代 Stage 10 其余专家。
- 全通道请求核算、浏览器后代进程治理、source/workbench 启动原子性及跨进程异常恢复仍需逐路径验收。
- SFC 测试使用真实组件逻辑/模板，但 IPC 与宿主渲染为替身；桌面实机 E2E、大数据库性能、打包和真实授权环境验收尚未证明。
- 资产/知识/skills 生命周期、沙箱与工具供给及整个 Master Plan 仍未完成。`local_operator` 不是企业认证身份。
- 本增量未部署、未访问外部测试 URL、未启用 Host Agent。
