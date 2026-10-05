# Web helper 真进程逐路日志审计 · 2026-10-03

Master开发中。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 保持，原脏树没有重置、覆盖或提交。只使用完整临时数据库、临时worker/localhost测试和现有前端测试；本轮未操作真实数据库/CAS/资产/安装或授权URL。

## 问题与原权限边界

原helper等进程退出后才读取stderr，无法在运行期间回放；真实正常退出的Node父进程留下继承pipe的后代可继续写marker。stdout是Native机器JSON，不能混入日志。调用必须沿已提交原Web branch claim，不能把Source plain control或随便一个scanId升级为Web授权。

新增可选诊断owner只读现有claim，仍由原NativeBranchGuard取得/保持OS调用owner。私有writer、read-only IPC及wake-only通知复用原process journal；Source不获Web诊断身份，SDK继续独立typed owner/stage日志，不混为子进程输出。日志不授Root/C/派发/重试权。

## 最小修改与数据合同

- JSON helper原主体提取到独立叶，原全文保存在 `/tmp/oviraptor-web-helper-original-json-helper-preimage.rs`。原大文件只薄委托/include，外部旧test-only drain使用全文保存的17行独立叶，生产不恢复EOF-only drain。
- 并发pump分开收stdout与stderr：stdout32MiB硬上限、stderr64KiB；stdout保持exact bytes及原JSON解析，绝不进入日志redaction/framing。stderr走16KiB整记录、64队列的现有observer/journal，持久缺口明确，数据库完全不可写时不能保证gap也保存。
- 私有helper执行身份包含原branch及helper/依赖/input/target/node路径/PATH/proxy材料摘要；只核Node路径，未证明Node二进制内容指纹。原当前scanning/pending/notdeleted和actualclaim前提保留，无创建Root或回填旧权限。
- Unix成功、失败、取消均清理原OwnedChild进程组；非阻塞泵停下后250ms收尾。每5ms核control，EOF继承pipe不无限阻塞。nonUnix恢复使用原force_stop/taskkill，原函数体不变；Windows退出父进程后的完整树/job object清理未验证。
- 两个packaged workers只写固定安全stderr阶段名；原stdoutJSON/probeStage未改。UI仅区分同阶段短executionId；无ID明确待核验。沿用主题、cursor和原只读IPC，未新增grant或假阶段。

## 实际 red → green

1. 00只抽取接口/原行为前提，不是行为red。加入六项真实完整临时应用DB/原branch guard tests后首1/6（8.20秒）：AST运行中无持久行、取消时无运行中行、父正常退出后真实后代marker泄漏、rawJSON完整后无持久stderr、packaged AST缺阶段。负向非法claim/Source/current/deleted/missingDB已经green。
2. 首red中AST断言先失败，未到browser分支；rawJSON byte断言已过，limit/queue子场景未到，不把它们宣称首red复现。真实取消的后续kill断言也未到。
3. observer第一编译E0425是外部test-only drain调用遗漏，保留原全文并放test-only叶后继续，非功能red。observer实际5/6（12.97秒），只packaged阶段仍red；这轮才真实执行32MiB±1stdout、64KiB±1stderr和5000条真实SQLite锁队列缺口。
4. Unix不用的原stop函数造成warning，未直接删除Windows活清理；cfg(not(unix))保留原函数和调用。worker固定stderr补齐、局部fmt后6/6（13.08秒），真实Node取消/正常父exit后marker均被阻止，durable queue gap明确1。Browser invalid-target在Chrome/CDP/HTTP之前止步，不算Broker/navigation功能完成。
5. 前端真实生产SFC/composable harness首2/3（同stage两process ID未显示），最小显示标签后3/3（444.23ms）；相关26/26（1972.44ms），默认818/818（33792.24ms）。transport/DOM模拟仅证明消费者与布局逻辑，不代表安装事件回放。
6. 根代理另用改前真实worker副本与当前worker运行AST有效/语法错、browser空/非法scheme四组，exit都0且stdout逐字节相同，stderr严格固定名单。保留 `/tmp/oviraptor-web-helper-live-direct-node/report.json` 与输出文件；两browser输入无远端网络/Chrome。

## 最后门禁

- `clippy-final` exit0，wall 36.88秒，`/tmp/oviraptor-web-helper-live-clippy-final.log`。
- `affected-final` exit0，wall 228.84秒，`/tmp/oviraptor-web-helper-live-affected-final.log`。
- `importer-final` exit0，wall 46.27秒，`/tmp/oviraptor-web-helper-live-importer-final.log`。
- `literal-final` exit0，wall 1.18秒，`/tmp/oviraptor-web-helper-live-literal-final.log`。

相关284/284 runtime228.23秒，包括原SDK/预算/Source/终态身份/派生事务合同及整个native_pipeline tests，不加总子集合。严格alltargets/allfeatures Clippy -Dwarnings，import-existing-results39/39、exact退役1/1。Cargo均nice15/offline/locked/j1/testthreads1，运行期间没有编辑Rust/resource。全部结束后npm run build（vue-tsc + Vite，Vite2.13秒）成功。7新Rust叶局部fmt/check及diffcheck0。旧9项未迁移失败仍保留，不算全量门禁。

## 原脏树保护

19路径10已有/9新，7新Rust叶max346，1128路径aggregate `684c56235f1d9d8cc8b5b22f1f614185a519b6fa1bfc1f1385f5b3b9755cef2f`；1109范围外原路径保持。`/tmp/oviraptor-web-helper-live-baseline/before/prior-diffs/reviewed-merge-diffs/scope-final/code-snapshot` 保存各阶段原文和逐文件增量。只应用已全文读取及SHA核验的review-current patches与根代理小patch，没有使用旧五表假夹具草稿或批量覆盖。

| 路径 | 类型 | 行数 |
|---|---|---|
| `src-tauri/src/commands.rs` | 已有 | 174 |
| `src-tauri/src/commands/native_helper_log_owner.rs` | 新增 | 87 |
| `src-tauri/src/commands/native_helper_process.rs` | 新增 | 224 |
| `src-tauri/src/commands/native_helpers.rs` | 已有 | 342 |
| `src-tauri/src/commands/scan_execution_frontend.rs` | 已有 | 496 |
| `src-tauri/src/commands/native_helper_live_fixture.rs` | 新增 | 77 |
| `src-tauri/src/commands/native_helper_live_limits.rs` | 新增 | 117 |
| `src-tauri/src/commands/native_helper_live_tests.rs` | 新增 | 346 |
| `src-tauri/src/commands/native_helper_observer.rs` | 新增 | 70 |
| `src-tauri/src/native_pipeline/process.rs` | 已有 | 347 |
| `src-tauri/src/native_pipeline/process_output.rs` | 已有 | 246 |
| `src-tauri/resources/workers/8_js_ast_analyzer.cjs` | 已有 | 612 |
| `src-tauri/resources/workers/9_frontend_runtime_probe.cjs` | 已有 | 1718 |
| `tools/test_web_helper_log_replay.cjs` | 新增 | 17 |
| `tools/web_helper_frontend_harness.cjs` | 新增 | 48 |
| `src/features/sentinel/components/NativeProcessLogView.vue` | 已有 | 27 |
| `src-tauri/src/commands/native_helper_test_capture.rs` | 新增 | 17 |
| `src-tauri/src/commands/scan_control.rs` | 已有 | 1297 |
| `package.json` | 已有 | 41 |

## 未完成和风险

Source all-outcome finally、完整Root本地ReAct/全部触发、所有角色/Broker/真实2/3批次并发、有序人工SDK/ACK/聊天、动态预算续跑/对账、canonical→derived重启恢复及9项旧正向迁真实入口尚未完成。诊断writer失败会停止helper，不自动重试；完全无法写DB时无法可靠保存最后gap。Windows清理、本轮Tauri实际回放、安装态和授权URL未验收。InputParser原自动审批拒绝不重试绕过；摘要未保存详细拒绝原因。Goal active，不标Master完成。
