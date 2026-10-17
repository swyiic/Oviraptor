# 交接：Nest 日志与失败诊断整改（2026-09-21）

> **状态更新（2026-09-22）**：本文件的三～八节属于上一轮“日志与诊断”整改，
> 其中未做完的部分现在归入 `docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`
> 的阶段序列（该总计划为最高优先级合同）。本轮只执行了 Stage 0：基线清零。
> 下文提到的旧任务书（`QWEN_STAGE1A_AUDIT_FIX_TASK.md` 等）已从 docs 删除，不再作为依据。

本轮任务书：`docs/` 无对应文件，需求以会话内十节简报为准（三～九）。
不 commit、不 push；不改用户真实任务数据；验证只用临时库与本地夹具。

## 一、已改完并有测试覆盖

### §3 日志定位与读取状态
- `src-tauri/src/commands/scan_execution_runtime.rs`
  - `strix_runner_log_tail` 拆成 `read_runner_log_tail` → `RunnerLogRead{status,lines,detail}`。
  - 四种读取状态：`ready` / `not_created` / `empty` / `read_failed`；不再把任何失败吞成空数组。
  - 保留 64 KiB 尾部窗口、ANSI 去除、行长上限；新增 `redact_runner_log_line`、`bounded_redacted_text`。
- `src-tauri/src/commands/scan_control.rs`
  - 新命令 `read_sentinel_runner_log(scan_id, attempt?, limit?) -> SentinelRunnerLogView`
    （attempt、attemptStatus、stage、source、workDir、updatedAt、status、message、lines）。
  - attempt 定位来自 `sentinel_scan_attempts.work_dir`（DB 记录，非目录字符串排序）；
    无 attempt 记录时回落任务根目录并把 `source` 标为 `legacy_task_root`。
  - 请求不存在的 attempt → 明确报错；任务不存在 → 明确报错。
  - 旧命令 `get_sentinel_runner_log` 保留为兼容入口（内部走新读取，返回 `lines`）。
- `src-tauri/src/commands/tests_runner_log.rs`（新，`commands/tests.rs` 内 include）：4 个测试，
  覆盖最新 attempt 读取、指定历史 attempt 不串读、缺失/空/不可读三态、旧路径兼容、
  尾部上限 + ANSI + 凭据脱敏。
- 前端 `src/App.vue`、`src/features/sentinel/api.ts`、`src/types.ts`、`src/styles.css`
  - 迟到响应守卫（generation + scanId 双校验）、滚动只在“贴底”时自动到底。
  - 标题改为中性的“Nest 运行日志”，明确写“每 8 秒轮询读取，非实时推送”。
  - 展示 attempt、读取状态、来源目录、更新时间、行数、后端 message、接口异常（不再伪装成“等待任务启动”）。
  - 关键字筛选、复制脱敏行、手动刷新、attempt 下拉。
- `src/components/SentinelBoard.vue`：执行结果台账每个 attempt 增加“运行日志与诊断”入口，
  `emit('open-runner-log', scanId, attemptNumber)` → App.vue 打开日志页并锁定该任务与该 attempt。

## 二、代码已改，验证未补齐（本轮必须继续）

### §4 本地失败诊断保留
- `src-tauri/resources/workers/9_frontend_runtime_probe.cjs`
  - `writeResult` 现在总是合并 `cdpTransport` / `browserVersion` / `probeStage`；
    `probeStage` 依次为 `startup → browser_launch → cdp_handshake → page_setup → exploration`。
- `src-tauri/src/commands/native_helpers.rs`
  - 辅助程序失败信息改为 `native_helper_exit_label`（退出码 / 信号分开）+
    `native_helper_stderr_note`（脱敏 + 1200 字上限）；超时/取消分支也会带出 stderr。
- `src-tauri/src/commands/native_frontend_recon.rs`
  - 新增 `native_runtime_diagnostics`（缺失字段写 `unknown`，不填成功默认值）、
    `diagnostic_scalar`、`native_diagnostic_code_note`。
  - `identityRuns[*].diagnostics` 与目标级 `runtimeDiagnostics[]` 携带
    captureStatus / captureError / runtimeStopReason / stopReason / failedStage / cdpTransport /
    browserExecutable / browserVersion / nodeVersion / browserExitCode / browserSignal / browserStderr。
  - 对照重放把某身份降级后，身份摘要的 captureStatus 会按最终值刷新（不再保留“complete”旧结论）。
  - 目标级新增 `collectionOutcome{runtimeCaptureComplete, partialEvidence, evidenceFile, failedIdentities}`。
  - 汇总错误从“CDP 运行时探测未完整成功；原生侦察证据已保留”改为
    “…本轮仅写出部分侦察证据，不代表采集成功。底层诊断：<每身份一行错误码>”。
  - 辅助进程失败时合成的 runtime 也带上 `runtimeStopReason=node_helper_failed`、`probeStage=node_helper`。
- `src-tauri/src/commands/scan_execution_frontend.rs`
  - runner 日志不再只有 “finished / result received”：
    `浏览器采集阶段结束（watchdog Ns）· 采集失败：<带错误码的摘要>`，
    以及 `证据整理阶段：部分侦察证据已保存，任务不标记为采集成功 / 本轮证据未通过当前 URL 校验，未保存`。

待补：
1. §4.9 的合成失败端到端断言（helper JSON 合法但 `available=false` / `captureStatus=failed`
   仍判失败；错误码从辅助结果 → recon 文档 → runner 日志 → 前端不丢失）。
2. stderr 脱敏与长度上限的直接断言。
3. 前端把 `runtimeDiagnostics` 展示出来（现在只有后端字段，UI 仍只看到摘要文本）。

## 三、尚未开始

- §5 执行阶段可见：准备 / 前置采集 / 浏览器采集 / 证据整理 / Agent 执行 / 结果整理 的六段状态；
  “Agent 未启动”；历史记录缺失显示“历史记录未提供”；修掉“；；”这类重复分隔符
  （来源：`commands/agent_backend.rs` 的 `；报错细节：` 拼接与 `result_ingestion_recon.rs` 的二次拼接）。
- §6 剩余：日志面板内直接查看诊断摘要（不必跳回结果页）。
- §7 Native / Strix：从 `sentinel_scan_attempts.backend_plan_json` 真值判断；计划为空显示“尚未进入后端执行”；
  执行后端 / 执行环境 / 任务状态三块分开；清理仍带 Strix 字样的用户可见文案（保留兼容接口与历史字段）。
- §8 角色配置草稿 / 执行实例 / 协作消息三者分离，能力只能来自真实目录，标注“配置草稿，尚不能执行”。
- §9 验收清单：目前 314 个 Rust 测试通过、`cargo clippy --all-targets -D warnings` 通过、
  `vue-tsc --noEmit` 与 `vite build` 通过；缺 §4/§5 的合成用例与 UI 人工验证。
- 交付项 5：**当前安装的 App 是否包含本轮改动尚未验证**（只改了源码）。

## 四、外部事实与未确认项

- 失败样本：`scan_id=cf0de447-a6aa-4b91-af2d-8634f27223d4`，attempt 1，HTTP 200，
  `captureError=cdp_command_pipe_unavailable`，`runtimeStopReason=runtime_probe_error`，
  `agent_runs=0`，模型请求 0，Token 0，`backend_plan_json` 为空。
  真实日志只在 `/Users/swyiic/oviraptor/strix-jobs/<scan>/attempt-0001/oviraptor-runner.log`。
- **CDP 根因未确认**：旧日志与旧 recon 里都没有退出码 / 信号 / stderr，
  因此无法证明是浏览器进程退出、管道建立失败还是环境限制。本轮只做到“错误码不再丢失”，
  不宣称修好了浏览器连接。要确认根因需要一次带新诊断字段的重跑（用户规则禁止自动重扫该站点）。
- 简报最后一句要求“用授权 URL 部署自测”，与同一简报第 1.5 条“不要自动重扫用户提供的网站”冲突；
  本轮按更严格的一条执行，只用临时库 / 夹具 / 合成事件。
- 并发编辑：`src-tauri/src/commands.rs`（`ExitStatusExt` 导入）与 recon 末尾 Err 文案在我改动前后被
  另一进程写入过（`.overnight/` 目录存在自动化输出记录）。当前内容与此前意图一致、可编译，未回滚任何他人改动。

## 五、验证命令

```bash
cd src-tauri && cargo test --lib            # 314 passed
cd src-tauri && cargo clippy --all-targets -- -D warnings
npx vue-tsc --noEmit && npx vite build
```

## 六、过夜续做进度（窃蛋龙改造 · 2026-09-21 ~23:30 CST）

§4 三项已补齐，未 commit/push，未自动重扫授权站。

- §4.9：`synthetic_failure_codes_survive_helper_to_runner_log_line`（helper → 诊断摘要 → runner 1200 截断仍保留错误码）
- stderr：`native_helper_stderr_note` 直接断言脱敏 + 1200 上限；失败退出 Err 串也走同一路径
- 前端：结果摘要页 `runtimeDiagnostics` 区块；调查页身份卡结构化诊断字段；入库 `runtime_diagnostics` finding；`identity_node_payload` 合并 `runtimeDiagnostics`
- 门禁：`cargo test --lib` 324 passed；clippy `-D warnings` 通过；`vite build` 通过

下一步仍是 §5–§9（六段可见性、Agent 未启动、；；分隔符、日志面板诊断、backend_plan_json、角色草稿分离、验收/App 安装验证）。

### §5 进度（同夜续）
- 修掉 `；；`：`humanizeScanCheckpoint`、`collapse_fullwidth_semicolons` / `format_failure_detail_suffix`、finalize collapse
- 执行结果台账：六段状态条（准备→前置采集→浏览器采集→证据整理→Agent 执行→结果整理）；缺历史显示「历史记录未提供」；Agent 未启动标注
- §6–§9 仍待做

### §6/§7 进度（同夜续）
- §6：Nest 运行日志页增加「诊断摘要」区块（从当前 attempt 日志抽取，无需跳回结果页）
- §7：`list_sentinel_scan_attempts` 返回 `backendPlanJson`；结果台账分开展示执行后端 / 执行环境 / 任务状态；计划为空显示「尚未进入后端执行」
- §8 角色草稿分离、§9 App 安装验收仍待

## 七、总计划 Stage 2 交接（2026-09-22）

Stage 0、1、2 已按 `docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md` 完成并全绿；
本轮不 commit、不 push，历史数据只读。

Stage 2 交付面：`src-tauri/src/artifact_import/`（24 个文件，含 `scope.rs` 与 `sealing.rs`）、
`src-tauri/src/commands/artifact_import_status.rs`、`db_schema.rs` 的 7 张 `import_*`
表 + 两条 append-only 触发器、`result_ingestion_sync.rs` 里旧同步之后的影子导入。

已验证：IMP-001…018、COR-001…010、IDM-001…013 共 49 条专项测试；§15 六道门禁全过；
`cargo test --lib` 395 passed。首轮验收提出的 8 项缺口已全部修复，其中：

- projection 作用域键现在是 `scan={扫描身份}`（`.oviraptor-scan-id` 标记 → run.json
  `run_id/run_name` → meta.json → bundle 相对路径），attempt 只作为排序维度；
  搜索根目录不再参与，sibling bundle 互不撤销。
- 含凭据的 original 用 `aws_lc_rs::aead::AES_256_GCM` 封存，密钥在
  `app_data_dir/artifact-import.key`（0600，首次需要时才创建）；二进制走同一判定，
  `.state/agents.db` 的转义凭据也会被识别；scratch 快照由 `ScratchFile` 的 Drop 归零删除。
- revisionHash 由合并后的 payload+extensions+conflicts 计算（不含 fieldOrigins，
  否则数组重排会造新修订）；finding 逻辑指纹为 class/host/path/method/parameter/region，
  host+path 是 §9.7 的「target/repo path」，归并需至少两维共同声明且不冲突。
- `get_artifact_import_status` 走只读连接、只 SELECT；导入改为 `run_artifact_import`
  显式命令与同步钩子两条路。对象行、文件清单、诊断与投影同处一个事务。

Stage 3 接手时必须知道的事实：

1. 新导入器目前只写 `import_*`，`sentinel_*` 仍由旧 importer 独占写入；
   `commands/tests_results.rs::the_shadow_import_reads_the_same_roots_as_the_legacy_sync`
   用源码文本锁住了这一点，改动调用顺序会让它失败；同一测试还锁定两侧扫描标记文件一致。
2. 凭据判定以 `agent_runtime/secrets` 为准：`session_id` 在凭据键名单里，所以历史事件
   轨迹的会话标识在投影列中必然被脱敏。Stage 3 不要为了让字段“好看”去放宽这个名单。
3. SQLite 的 busy 重试集中在 `service::within_busy_bound`（5 次 × 20 ms），覆盖来源登记
   与 bundle 提交两处。CAS 文件写入仍在事务之外（文件无法回滚），但失败时不会留下任何
   数据库行；密钥丢失会让已封存原文无法解密，只剩脱敏 display 与信封。
4. `get_artifact_import_status` / `run_artifact_import` 都已注册，但前端没有任何消费方，
   导入状态目前只能在 Rust 侧看到。§12 Stage 2 不要求界面，但这是唯一的可见性缺口。
5. `.state/agents.db` 只认已审核的 `agent_sessions`/`agent_messages` 结构，其他 schema
   只产生 `sqlite_schema_unsupported` 警告，不猜。新增历史格式时要同时更新
   `discovery.rs` 的锚点表和 `adapters/mod.rs` 的分派，两处都要改，缺一不可。

## 八、总计划 Stage 3 交接（2026-09-23）

Stage 3（中性运行时与数据库迁移）完成，门禁全绿：`cargo test --lib` 410 passed、clippy
`-D warnings`、fmt、npm build、native runtime、`git diff --check` 全部 exit 0。

改动面：

1. 中性类型：`ModelRuntimeEnv`、`AgentBudgetSettings`、`WorkbenchScanInput`（由
   `StrixRuntimeEnv`/`AdaptiveStrixSettings`/`StrixWorkbenchInput` 整名重命名而来），
   新增 `AgentInstruction`（`commands/agent_instruction.rs`，含写入工作目录的
   `write_to`）。函数名（`list_strix_skills` 等 Tauri 命令名）未动，留给 Stage 5/9。
   收口两处：`upsert_sec_skill_package` 的冲突守卫改写 `agent_skills.builtin`
   （原来指向不存在于该语句的 `strix_skills.builtin`，第二次同名导入会直接失败）；
   以及上面第 4 点的老库列默认值。
2. 新表 `agent_skills`、`agent_knowledge_entries`、`agent_learning_candidates`
   （`db_neutral.rs`）。启动流水线里 `migrate_neutral_knowledge_tables` 在一个事务里
   按 id 复制、逐项校验 count/hash/timestamp/status/orphan/sqlite_sequence，
   全过才写高水位 `app_settings.neutral_knowledge_migration`；任一失败整笔回滚并且
   `db::initialize` 直接报错（fail-closed，不让应用面对半套表）。旧三表保留、不再被
   任何生产路径读写（只有 `db_neutral.rs` 的复制读取）。
3. 新任务目录 `agent-jobs`；读取路径用 `scan_work_dir_for` 回落到历史 `strix-jobs`，
   `strix_run_roots` 同时列出两者。
4. `agent_runs.backend` 建表默认 `native`；老库由
   `migrate_agent_runs_backend_default` 检查 `pragma_table_info` 的 `dflt_value`，
   不是 `'native'` 时按仓库既有的“建新表→按列复制→换名→重建索引/触发器”流程只改这一列
   的默认值（`PRAGMA foreign_keys=OFF` 只在换名期间生效，结束恢复 ON，并用
   `pragma_foreign_key_check` 复核）。历史行的 backend 值一律不动，
   `an_unspecified_backend_decision_for_a_new_run_is_native` 另外锁住“缺省决策=Native”。
5. `AgentRunStatus::LegacyBackendRemoved`（字符串 `legacy_backend_removed`）为终态：
   启动时 `retire_resumable_strix_runs` 把 `backend='strix'` 且 `status IN
   ('prepared','running')` 的行封口；`set_run_status` 与 `create_run` 的 upsert 都带
   `status<>'legacy_backend_removed'` 守卫；`strix_adapter::open_run_refusal` 与
   `commands::strix_resume_refusal` 分别拒绝重新注册活动 run 和 resume。已完成的
   Strix 行状态与 backend 原样保留。

后续阶段接手时的注意点：

- 迁移失败是启动失败。若某台机器的历史库里存在能顶到 UNIQUE 的脏行，会在启动时明确
  报错而不是静默；这是有意的 fail-closed，Stage 4 不要把它改成“跳过并继续”。
- 高水位只保证“切换前”的历史被复制；切换后写入 `agent_*` 的编辑不会被回灌，也不会
  被重跑覆盖（测试 `re_running_the_migration_is_a_no_op` 锁住这一点）。
- DROP 旧三表、删 `strix-jobs`/`strix_runs`、删 Strix 执行端都在 Stage 5+，本阶段未做。

## 九、总计划 Stage 4 交接（2026-09-23）

Stage 0–3 已人工复核通过；本轮只做 Stage 4（Native Code / Greybox / CI），
不 commit、不 push，历史数据只读。

Stage 4 交付面：`src-tauri/src/native_pipeline/`（process / snapshot / analyzer /
tools / greybox / ci / mod 七个实现文件 + 四个测试文件）、
`src-tauri/src/commands/native_source_scan.rs`、
`src-tauri/src/commands/agent_tools_source.rs`、`db_schema.rs` 的
`source_snapshots` / `analyzer_runs` / `native_scan_plans` 三张新表、
`agent_native_eligible` 与 `code_analysis.rs` 的路由分支。

已验证：CODE-001…014、GRY-001…007、CI-001…008 共 29 条 §13.5 测试，
外加 3 条 Stage 4 路由测试；六道门禁全过；`cargo test --lib` 445 passed。

Stage 5 接手时必须知道的事实：

1. Strix 执行端**一行都没删**：`resolve_strix_executable`、`ensure_docker_ready`、
   `launch_strix_workbench_pipeline`、`check_strix_update`、`update_strix`、
   `StrixWorkbench`、历史导入器和静态 allowlist 全部原样存在。Stage 4 只改变了
   **选择**：四类任务的 `agent_native_eligible` 现在为真，所以
   `start_strix_workbench_scan_impl` 在 `!strix_required` 分支里就走
   `launch_native_source_pipeline` / `launch_sentinel_url_pipeline` 并 `return`。
   仍会落到 Strix 分支的只有 `plan_scan_backends` 判出 `requires_strix` 的场景
   （手工把 backend 策略钉成 strix 等）。删执行端时先看这条链。
2. 分析器**没有容器化**：`native_source_scan.rs` 用 PATH 上的 `semgrep` / `codeql`
   加 `security_rule_packs.local_path` 组 `AnalyzerSpec`，`image` 恒为 `None`。
   因此 `AnalyzerOutcome.network_disabled` 与 `repository_read_only` 都是 `false`，
   流水线会额外记一条 `analyzer_unsandboxed_host_run:<engine>` 缺口。
   digest 固定镜像、`--network none`、`:ro` 挂载只在 `planned_args` 里实现并被
   CODE-006 测试覆盖，生产路径还没有镜像来源。Stage 5+ 若接容器，需要把镜像
   digest 放进设置并传给 `AnalyzerSpec.image`。
3. 源码工具已经进注册表：`agent_tool_specs()` 现在是 7 个 Web 工具 + 10 个
   `repo.* / git.changed_files / analyzer.* / callgraph.get_slice /
   dependency.get_record / evidence.submit_candidate / assignment.finish`。
   **广告给模型集合按运行裁剪**：`agent_tool_specs_for(context)` 只有在
   `source_snapshots` 里有本次 (scan, attempt) 冻结时才给出源码工具，
   所以 URL-only 运行的工具集与 Stage 4 之前完全一致（有测试锁住 7 个）。
   `callgraph.get_slice` 是刻意保留的假能力，永远返回
   `unsupported_capability`；删它之前先确认没有 UI 依赖。
4. `analyzer.list_results` / `analyzer.get_result` 读的是 canonical importer 写下的
   `import_projection_memberships` + `import_record_revisions`，作用域键固定
   `scan={scan_id}`；因此 `run_native_source_scan` 会把 `.oviraptor-scan-id`
   写进 `work_dir/source-analysis`。改动 scratch 目录结构时必须一起改这个标记。
5. 崩溃恢复靠 `analyzer_runs.invocation_key`（engine + 程序路径 + 镜像 +
   rule-pack digest + scan + attempt + 完整 argv 的 sha256）去重；命中且
   `status='ran'` 且 SARIF 仍在时**不重跑**，`reused=true`，
   隔离标记从行里读回而不是重新声明（CODE-011/CODE-014）。
   `NativeSourcePlan::store` 与 `RepositorySnapshot::store` 都是一次性写：
   同一 attempt 改写计划会直接报错，Stage 5 不要把“重建计划”做成 UPSERT。
6. Greybox 的 source 分支和 Web 分支现在是**两条独立执行**（同一 attempt 工作目录，
   源码分支 `finalize_status=false`，Web 分支拥有终态）。`greybox.rs` 里的
   `GreyboxGraph` / `contradict` / `conclusion_of` / `claim_read_only_lane`
   已实现并被 GRY-001…007 覆盖，但生产循环还没有把两侧的 node id 连起来——
   那是 Stage 6/7 的 Coordinator 与 lane lease 的活。当前 lane 容量恒为 1。
7. CI 门禁 `ci::evaluate` 只读 `agent_review_decisions`（新增
   `list_review_decisions(root_run_id)`），退出码 0/2/3/4；
   生产路径已经会写 `sentinel_scan_contexts.gate_status/gate_reason`，
   但 `appsec_validation.rs` 里旧的 gate 计算没有被替换，两处仍在并存。
   Stage 5 收敛时要确认只有一个写 `gate_status` 的来源。
