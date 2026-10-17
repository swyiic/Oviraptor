# 源码覆盖材料与工具缺口贯通审计（2026-09-27）

## 1. 本次增量与未完成项

新增从冻结源码材料、四阶段真实回执及候选 Reviewer 交付重建的覆盖材料索引。修复源码工具阶段后来发现的缺口只停留在局部结果、未完整进入根结案和 CI 判定的问题；缺口现贯通普通源码报告、正式审查页及 JSON/SARIF/整体审查包。

这是 **覆盖审查材料准备，不是总体覆盖独立 Reviewer**。输出固定为 `prepared_not_reviewed`，`independentReviewCompleted=false`、`coverageSufficient=null`、`executionEligible=false`；保留 `source_coverage_review`。没有增加模型调用、assignment、工具权限或预算版本，不改变旧候选审查材料的字节合同。

不能据此勾选整个 Master Plan §10.2、CI 或项目完成。真实总体覆盖 Reviewer、阶段恢复、费用与性能等仍需继续。

## 2. 证据合同

`agent_runtime/multi_agent/source_coverage.rs` 的 `audit` 只在调用方数据库事务内执行，从保留材料重建，不读取 TerminalReduced 展示 JSON 作为权威，不续租、不修复投影、不派发模型。

索引绑定 scan/attempt/root、源码计划 hash、选中分析视图与结果摘要、现有候选审查材料摘要，并保存：

- 请求与有效 scope、fallback、选中文件及没有内容的变更路径；后者不一律解释为删除。
- 冻结仓库 tree/head/base、diff 状态和文件数量。
- 计划语言/分析器/工具及真实分析器运行记录。
- 四个初评/工具 assignment 的回执目录，以及带作者、轮次、调用序号的工具证据摘要。
- 实际候选复核交付的 assignment/run/message、payload SHA 和裁决身份。
- 逐来源、逐 assignment 的报告缺口与去重后的剩余缺口。

选中文件清单和分析器运行记录**都不是逐文件/逐行覆盖证明**。索引不重复嵌入整份源代码；底层材料和回执仍须通过原审计路径解析。摘要按本实现的紧凑 serde JSON UTF-8 字节计算 SHA-256，不宣称是签名或其他标准化 JSON 协议。

候选复核尚在派发/接收阶段时拒绝生成准备完成的材料；损坏的 ACK、用量、工具回执、计划或审查材料同样拒绝。已完成旧 attempt、过期租约及原始仓库移动后可按保留证据只读核验，不恢复执行资格。

## 3. 缺口合并规则

1. 合并仓库快照、选中范围、计划、分析结果及**已核验的工具 finish 结果**中的缺口。不从自由文本猜测权限或机器结论。
2. 相同缺口码去重并固定顺序，但 `reportedGaps` 保留不同报告者的归属。
3. 只有候选 Reviewer 真实完成时才移除精确的 `source_review_not_completed` 占位；其他限制不得被 Reviewer 总结抹掉。
4. 候选裁决存在 `insufficient_evidence` 时保留 `source_candidate_evidence_incomplete`。
5. 总体覆盖阶段尚未实现，始终保留 `source_coverage_review`。零候选不创建虚假候选 Reviewer，也不代表覆盖通过。
6. 展示、CI 和根结案使用脱敏后的同一缺口列表，避免错误文本经其他出口重新暴露。

## 4. 生产消费与界面

- `native_source_completion.rs`：在原完整预算/回执校验后生成材料；真实缺口进入 completion 和 TerminalReduced，写后重新验证证明未变化。
- `native_source_coordinator.rs`：返回 `coverageReviewPreparation`，不新增阶段。
- `native_pipeline/source_ci.rs`：用真实覆盖缺口替代只读取初始 plan gaps 的做法；候选裁决、冻结策略和正式发现计数约束不放宽。
- `native_source_scan.rs`：普通非 CI 报告也保留准备材料的缺口。
- `native_source_findings.rs`：在同一读取事务内核验覆盖材料，正式裁决页及三种导出带 `review.coverage` 摘要；核验失败不返回伪造的已核验覆盖。
- `SourceReviewEvidence.vue` / `src/types.ts`：新增可选摘要字段，保留旧响应兼容；核验 scan/attempt/root、固定未完成状态、scope、非负计数、摘要格式与必需缺口。分页中材料改变会撤销旧视图。

界面明确显示「覆盖材料已准备，不代表覆盖审查已完成」，并显示范围、文件数量、缺口和材料摘要。页面最多展开前 20 个缺口，完整列表可导出。此摘要仅附于当前可审计的正式候选裁决页面；零候选任务的准备材料仍在根输出/事件中，本增量没有新增独立的零候选覆盖详情 API。

历史 JSON/SARIF/整体包继续只读导入，不因携带本摘要取得 Native 权威；没有升级导入格式或旧任务运行合同。

## 5. 回归与证据

日志为本机 `/tmp` 文件，不替代正式 CI 制品。计数按实际终态输出记录，不将嵌套断言当作独立测试。

| 检查 | 当前结果 | 日志 |
| --- | --- | --- |
| 后端初始红测 | 1 失败：预期准备材料状态，旧输出为 Null | `/tmp/oviraptor-source-coverage-preparation-red.log` |
| 后端首次绿测 | 1 通过 | `/tmp/oviraptor-source-coverage-preparation-green.log` |
| 首批后端专项 | 6 通过；早于追加的第 7 项和消费断言 | `/tmp/oviraptor-source-coverage-preparation-suite.log` |
| UI 初始红测 | 新增 3 项失败 | `/tmp/oviraptor-source-coverage-ui-red.log` |
| 实际 SFC 专项 | 16 通过 | `/tmp/oviraptor-source-coverage-ui-green.log` |
| 全部现有 UI 测试 | 196 通过 | `/tmp/oviraptor-source-coverage-ui-full.log` |
| vue-tsc / Vite | 通过；主 JS 848.11 kB 分包告警保留 | `/tmp/oviraptor-source-coverage-build.log` |
| 源码广域 `source_` 回归（主库） | 237 通过，0 失败，466.69 秒；包含 7 项新增覆盖测试 | `/tmp/oviraptor-source-coverage-source-regressions.log` |
| 全 target / all features 严格 Clippy | exit 0 | `/tmp/oviraptor-source-coverage-clippy.log` |
| cargo fmt --all --check | exit 0 | `/tmp/oviraptor-source-coverage-fmt.log` |
| 退役字面量守卫 | 4 通过，0 失败 | `/tmp/oviraptor-source-coverage-retirement.log` |
| 默认 features Rust 基线 | exit 0；主库 1267 通过，0 失败，801.08 秒；未含 feature-gated 导入器 | `/tmp/oviraptor-source-coverage-rust-full.log` |
| 后续工具链摘要修复后的精确 Rust 全量 | exit 0；all-targets／all-features 主库 1268／导入器 30 通过；包含本增量 7 项覆盖测试 | `/tmp/oviraptor-toolchain-digest-all-targets-all-features.log` |

新增 7 项后端测试覆盖真实本机模型 endpoint 的调用链、full/diff 区别、工具缺口进入 CI/三种导出/非 CI 报告、零候选、历史只读和伪造展示 JSON、回执与外来材料损坏、候选交付中断后的拒绝且不重放。新增 UI 测试执行实际组件，覆盖真实摘要展示、伪造/外来摘要拒绝、分页期间摘要改变。

保持单 Cargo、`nice -n 15`、`-j 1`、`--test-threads=1`；`nice` 不是 CPU 硬上限。session 99537 已收取 exit 0；该命令未包含 `--all-targets --all-features`，不能替代 Master Plan §15 的完整门禁。后续工具链摘要 CPU 修复及精确全量门禁已通过，另见 `NEST_TOOLCHAIN_DIGEST_CPU_AUDIT_2026-09-27.md`，不将默认基线改称全量。后续也重新通过严格 Clippy/fmt、UI 196、构建、Native 本机浏览器回环和空白检查。未运行真实 WebView/IPC、跨平台分发包、真实强杀/掉电或用户目标 URL 验收；没有访问外部模型、部署、启用主机执行或扩大授权。

## 6. 下一步与性能限制

当前覆盖审计重新调用现有阶段和候选审计，根收口、CI、详情/导出间仍有重复核验。尚未完成大仓库/多候选性能基准；不得以缓存展示 JSON 或减少证据校验掩盖开销。后续可在同一事务内复用不可伪造的类型化审计结果，但须证明跨事务、材料变化、缺失回执和旧轮次均不串用。

真正总体覆盖 Reviewer 须新增独立 subject、冻结输入/输出合同、显式版本化预算和阶段、真实 assignment/child-run/model receipt/usage/mailbox+ACK 及不可变裁决。不能静默扩大旧 v1/v2/v3 预算。覆盖“审查已完成”和“覆盖充分”分开；模型不能删除确定性缺口，也不能把准备摘要当最终裁决。

随后仍须推进初评/工具阶段恢复、未知效果核对、实际强杀重启与并发、费用账本、其他 Findings 消费者、Strix 引用与跨平台发布、指令/聊天端到端及 Master Plan 其余项。主机功能继续维持低优先级设计阶段。

### 6.1 下一阶段已核对的代码耦合

以下是对当前代码的检查结果，不是新增阶段已经实施：

- 阶段合同前置增量已经将 `source.rs::model_request_limit`、角色准入、候选 Reviewer 启用、裁决发布与根结案统一到 `SourcePhaseContract`；仍只接受 v1=8、v2/v3=9，但不再用“上限为 9”代替阶段权限。v1–v3 拒绝未来覆盖阶段字段，注册在发布前同样校验。测试及未完成范围见 `NEST_SOURCE_PHASE_CONTRACT_AUDIT_2026-09-27.md`；这不是新 coverage 阶段已经交付。
- `native_source_coordinator.rs` 的发布/恢复只接受 v1–v3；新版本必须原样恢复旧计划，而不是重新注册时给旧 root 补字段。
- `source_reviewer.rs` 的 prepare/progress、`native_source_reviewer.rs` 的边界恢复和 `native_source_completion.rs` 的数量证明按 `role='evidence_reviewer'` 假定只有一个候选审查者。后续若复用同角色，必须用经核验的 subject/phase/trigger 区分两项审查，并继续拒绝陌生或重复 assignment；不能靠忽略多余行解决计数冲突。
- `scheduler.rs` 与 `specialist.rs` 的源码 EvidenceReviewer 路由目前都指向候选审查合同。新 subject 必须贯穿 admission、派发前后复核、回执和 child 终结，不能只在命令层另发一个模型请求。
- `native_source_tools.rs` 为候选 Reviewer 保留调用份额；新增覆盖阶段必须同时保留其 token/request 预算。根结案和重入的精确用量/行数证明也须按实际签发阶段更新。
- 当前准备索引依赖候选交付，但不能反向依赖未来覆盖裁决，否则会形成自引用摘要。新覆盖裁决应引用独立准备材料摘要，最终显示摘要另外组合，不回写原材料。
- 零候选场景仍应跳过候选 Reviewer，但可以执行独立总体覆盖审查；该路径须有自己的真实调用数、消息数与终态验收，不制造空候选裁决。

后续验收必须包含旧版本不升权、未知版本拒绝、两个审查对象互换拒绝、零候选、预算不足、保存响应后重入不重放、ACK/用量/材料损坏、取消/过期租约及最终 CI/UI/导出消费。不允许先把 `independentReviewCompleted` 翻成 true 再补证据链。
