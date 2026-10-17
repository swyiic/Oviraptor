# Native 后端剥离复核记录 · 2026-09-26

## 结论和边界

本记录对应 `NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md` §11、§13.1，记录本轮具体修复、可重复验证的证据及仍未完成的验收。它不是完整 Master Plan 的完成报告，也不替代安装包、隔离沙箱、真实多智能体交互或授权目标环境的验收。

当前执行权限继续保持 `web_only`。本轮没有新增主机执行器、主机审批后端、Windows 主机能力，没有部署应用或访问外部测试 URL。

## 补充：历史展示声明校验与业务模块拆分

- 收尾验证：历史导入器 **30/30**、全目标/全特性严格 Clippy、Cargo 格式检查、两个新增 include 文件独立 rustfmt 和空白检查均通过。Cargo 串行运行、离线、单构建作业，测试单线程；低调度优先级不是 CPU 硬上限。日志 `/tmp/oviraptor-history-views-{importer,clippy}.log`。
- 三个历史展示读取入口原先只核对声明值，未约束 canonical schema 与布尔类型。负向回归先得到 **1 通过 / 1 失败**，确认未知 schema、数字 1/0 可以进入这些展示入口；这是展示资格缺口，不据此声称已经发生执行提权。
- 三处 SQL 均补齐 `oviraptor.artifact.v1` 与真实 JSON `true` / `false` 类型检查，保留来源关联、删除约束、数量限制和脱敏。导入报告文件 **518 → 278 行**，展示模型、查询与原命令迁入 **275 行**的 `commands/historical_import_views.rs`，调用接口不变；没有增加公共杂项模块。
- **148 行 / 3 项**业务回归复用原数据库夹具与 Native 快照。非法声明通过追加测试修订和回滚保存点构造，不覆盖不可变历史；另验证真实只读连接、原始 canonical 内容不变，以及有效任务中只剔除无效候选。
- 退役登记仅审查两个条目：展示迁移的 **5** 个历史名称命中从旧文件移到新文件；前轮轨迹隐私测试变更的 **6** 个命中用途仍为合成历史夹具，补齐之前遗漏的摘要。未批量重生成登记表、扩大运行时豁免或放宽门禁。
- 原 `STRIX_COMPATIBILITY.md` 仍包含已失效的 CLI 探测和升级指令，现就地改为历史产物兼容边界；没有新增同类审计文档。
- 已收取 exit 0：历史相关 **47/47**、包导入 **12/12**、退役登记 **9/9**、残留检查 **6/6**。筛选集合可能重叠；日志为 `/tmp/oviraptor-history-views-{related,bundle,retirement,residual}.log`。前序 Rust 全量不覆盖本次变更，未把专项结果记作完整验收；未部署、启动扫描或访问外部 URL。

## 2026-09-30 补充：源码、构建回执和安装目录的只读联查

- 发布专用入口 `tools/release/verify_build_observation.mjs` **120 行**，复用 `snapshotBuildInputs`、文件摘要与 `compareAppBundles`，没有复制构建流程或新增全局公共模块。新增六项测试沿用现有合成构建夹具，`tools/test_build_receipt.cjs` 共 **251 行**；没有新测试命令、依赖或独有用例删除。
- 回执必须为成功的 `build_observed`，退出码/信号、输入前后摘要及固定路径相符；保存的三份清单须与各自摘要匹配。JSON 读取有界，拒绝符号链接、非普通文件、畸形/缺失记录及重定向的输出目录。分别检查当前输入、旧构建包和安装文件树，不修改任何证据或应用。
- 六项覆盖一致观察、源码/包/安装漂移、失败及不一致回执、清单摘要/畸形/缺失、符号链接/超限记录、CLI 三类退出码。先红 **17 通过 / 6 失败**（新入口缺失）；实现后 `npm run test:release-bundle` **23/23**、两个变更 JS 文件语法与空白检查通过。日志 `/tmp/oviraptor-release-observation-{red,green}.log`；本轮未重跑 UI/Rust 全量。
- **2026-09-30 00:10 +0800** 实际只读联查返回 **exit 2**。使用 `/private/var/folders/md/445m9vhs4jq9mpfvq29vxndh0000gn/T/oviraptor-build-qJ9bKL/receipt.json`：`currentInputsMatchRecorded=false`、`builtBundleMatchesRecorded=true`、`installedMatchesBuiltBundle=false`。报告 `/tmp/oviraptor-release-observation-current.json`。这些临时记录不替代永久归档。
- 旧包仍为 **16 文件**，摘要 `bd8483d31d3592a1f6406097297de25e2118808b1f3d62e3e60fc55019eba353`；安装态 **15 文件**。五处差异为主程序、两个前端 worker、签名资源文件内容不同，以及安装态缺少浏览器位置配置。没有据此推断签名无效或实际运行后端。
- 所有输入和产物均为未签名本地观察；即使三项匹配，`sourceBinding` 与 `releaseAcceptance` 仍是 `not_verified`，签名/运行验证仍为 `not_performed`。工具不抵御整套证据一致篡改或并发恶意写入，不记录完整依赖/工具链闭包。**Master Plan §18.15 未通过**；未修改原始回执、重新构建、安装、启动应用或访问外部 URL。

## 2026-09-29 补充：项目内 Cargo 配置树的静态退役检查

- 既有静态遍历未覆盖根目录及 `src-tauri` 内的可选 `.cargo` 配置树。本工作树当前没有这两个目录，因此这是检查范围缺口，**不是发现仍在运行的 Strix 配置**。
- 复用原递归检查，纳入两处配置树，包括无扩展名 `config`、`config.toml` 和嵌套特殊扩展名文件。只有目录不存在可以跳过；其他读取错误及符号链接（包括悬空链接）必须失败。测试仅写入惰性标记，不执行配置、子进程或网络请求。
- 新增两项回归先得到 **0 通过 / 2 失败**；修复后退役/残留联合专项 **15/15**，历史导入器完整 **30/30**、发布工具合成测试 **17/17**、严格 all-targets/all-features Clippy、fmt check 和空白检查通过。Rust 离线、低优先级、单作业，测试单线程；本轮没有重跑主库全量。
- 复用现有测试文件和入口，文件由 307 行增至 **353 行**，没有增加依赖或生产执行能力。复核后只更新退役登记表中该测试文件的既有摘要及说明（字面量 10→11），没有扩大运行时豁免。临时夹具在测试结束前删除，未删除既有独有回归。
- 日志为 `/tmp/oviraptor-retirement-cargo-{red,green,clippy,importer}.log` 和 `/tmp/oviraptor-retirement-release-tools.log`。这些证据不覆盖父目录/用户目录 Cargo 配置、环境和 CLI 覆盖、依赖构建行为；合成发布测试不等于实际打包。本轮未构建或安装应用，旧包仍不能代表最新源码，也不能据此宣布完整退役或 Master Plan 完成。

## 2026-09-29 补充：统一历史预览的任务归属与只读展示合同

- 任务详情此前直接采用历史预览返回，模板统一标为未审核/只读；全局历史台账虽验证权限标记，却未复核返回的 `scanId`。模拟异常 IPC 返回的新增回归得到 **20 通过 / 3 失败**，复现权限标记矛盾仍展示、任务详情接收其他任务数据和台账接收其他任务数据。后端已有查询约束，本次证明的是前端合同缺口，不是后端越权或实际数据泄露。
- 两处共同使用历史结果业务目录的 `historicalPreviewContract.ts`（**12 行**），检查数组、最多 300 条、请求任务身份以及 `unreviewed/readOnly=true/executionEligible=false` 标记，不符合则整页拒绝并显示固定错误，不把矛盾数据重新标成安全。任务详情的其他独立区块仍可展示。
- 校验的是 API 规范化投影，**没有要求旧 JSON 添加字段，没有修改历史导入格式、原件或后端权限**。允许同一任务包含不同历史 attempt，也接受空页与恰好 300 条；历史 attempt 不改变当前 Native 轮次选择。前端校验不能取代后端 bundle/projection scope 绑定、脱敏及只读资格核验。
- 详情模块 **196 行**，历史台账模块 **80 行**。执行历史测试从 **400 行**拆为 **317 行**用例与业务内 **100 行**夹具（`tools/execution_history/harness.cjs`），原 18 项完整保留；详情/导入测试分别 **208/252 行**。新增 4 项用例，未删除独有回归、未新增测试入口或依赖；夹具加载真实共享校验，不复制一套业务判断。
- 联合前端专项 **41/41**，完整 UI **438/438**，类型检查和生产构建 exit 0；主 JS **309.57 kB**、任务中心 **38.85 kB**。Rust `canonical_history` 名称过滤 **2/2**，历史导入器完整 **30/30**，均离线、低优先级、单作业单测试线程、exit 0。没有修改 Rust，也没有重跑 1435 项主库全量。
- 日志 `/tmp/oviraptor-history-contract-{red,green,ui,build,rust,importer}.log`；前端/导入器局部证明不替代真实桌面联合验收或完整 Master Plan。本次也在下述 macOS 冷构建之后，旧 `.app` 不包含本次前端变更，未安装、启动或访问外部目标。

## 2026-09-29 补充：本地构建观察记录（实际构建已成功，未安装验收）

后续版本边界：此记录对应下述冷构建时的快照。之后已修改前端任务详情独立读取及历史预览合同（见聊天异步审计及本页顶部），因此本回执和 `.app` **不代表最新工作树产物**；构建前后输入一致仅对当时快照成立，不得延用为当前源码一致性证明。

- 增加发布业务模块 `tools/release/build_receipt.mjs`（133 行）和回归文件（148 行），把两个发布工具真正复用的文件摘要/遍历提取到同目录 `file_inventory.mjs`（44 行）；原比对器缩至 65 行。复用 `test:release-bundle` 入口，不引入依赖、业务执行器或新的全局公共层。
- 记录器固定调用本地 Tauri CLI，以单作业、锁文件、Cargo 离线和独立全新输出目录构建 macOS `.app`。保留开始态、项目输入前后清单、退出码/信号、日志、产物摘要；源输入变化、非零退出、缺失新产物、异常或不支持的输入均不能记成功。受限继承环境不隐式传入密钥、编译 wrapper 和 Tauri 配置覆盖。不中断其他构建、不安装、不启动 App；低优先级由文档中的 `nice -n 15` 调用提供，单作业不是 CPU 硬配额。
- 发布测试 **17/17**（原 7 项 + 新 10 项），均正常退出且无跳过；覆盖未跟踪源码/资源、锁文件、生成目录排除、缺失输入/链接/环境覆盖、正常观察、输入变化、失败/信号/空构建、项目配置拒绝、进程异常和真实子进程退出日志。测试用合成产物，不作为真正编译证明。原真实应用比对报告在共享代码提取后逐字段保持一致，仍是 5 处差异。日志 `/tmp/oviraptor-build-receipt-tests.log`。
- 真实冷构建会话 **92640 已退出 0**，北京时间 2026-09-29 19:50:44 至 20:01:51，约 11 分 7 秒；回执状态 `build_observed`，退出码 0、无信号。记录路径 `/private/var/folders/md/445m9vhs4jq9mpfvq29vxndh0000gn/T/oviraptor-build-qJ9bKL/receipt.json`。743 个输入文件构建前后摘要一致：`0f37229f4f8e1357ca92ebf66450ddc7c8e317765d3bf52db93e277d0d530102`；构建后再次核对当前输入与产物，仍匹配记录。
- 新产物位于同一证据目录的 `target/release/bundle/macos/Oviraptor.app`，约 37 MB、16 个文件；整个冷构建 target 约 1.6 GB。产物树摘要 `bd8483d31d3592a1f6406097297de25e2118808b1f3d62e3e60fc55019eba353`，主程序摘要 `ae33484bd2abc3c7d55494ebc5604ebe663b8d100ddaca5616e11e50e11c7cec`。没有安装、启动或删除该产物及证据。
- 单独执行 `codesign --verify --deep --strict --verbose=2` 通过本地签名完整性检查；显示为 **ad-hoc** 签名、无 TeamIdentifier，构建日志明确跳过公证。这不是 Developer ID 信任、公证、Gatekeeper 或源码来源证明，记录器自身也不执行验签。
- 新包与 `/Applications/Oviraptor.app` 比对 exit 2，**16 对 15 个文件、5 处差异**：主程序、两个 worker 和签名资源内容不同，安装目录缺少 `browser-locations.json`。报告 `/tmp/oviraptor-recorded-build-vs-installed.json`；安装目录未改动。下文旧构建目录 15 对 15 的比对是历史记录，不能混为本次新包结果。
- 构建后定向退役回归：`retirement_literal` **4/4**、`retirement_package_scope` **3/3**，均 exit 0、离线单作业单线程；日志 `/tmp/oviraptor-recorded-build-retirement.log` 与 `/tmp/oviraptor-recorded-build-package-scope.log`。这是两组名称过滤检查，不是重跑完整 Rust 或历史导入器。
- 明确边界：这是未签名的本地观察记录，`sourceBinding` 继续为 `not_verified`；不覆盖已安装依赖/用户工具链配置，不是完整依赖闭包、hermetic 构建、签名验签或可复现构建证明。前后快照不能发现中途改动后恢复，Cargo 离线不是网络隔离。临时目录记录不会自动成为持久发布归档；中断遗留 `in_progress` 绝不能通过验收。§18.14/15 及整个 Master Plan 仍未完成。

## 2026-09-29 补充复核：历史台账展示合同

- 任务中心的历史台账读取拆入 `results/useHistoricalImportLedger.ts`（**83 行**）；父组件从 396 降至 **340 行**，不抽取全局公共层、不增加依赖或测试入口。两组原有真实 SFC 测试加载实际业务模块，执行历史测试 **325 行**、历史导入测试 **239 行**，没有删除独有回归。
- 三项新增回归先失败：收起等待中的预览后加载态未清除，读取失败暴露后端原始异常文本，矛盾的历史权限标记仍被统一标成只读。修复后收起/切换/卸载作废迟到响应，列表与预览错误使用固定提示；预览拒绝非数组、超过 300 条及不符合 `unreviewed/readOnly/executionEligible=false` 的规范化返回。
- 校验的是当前后端 API 的规范化只读模型，**不要求旧 JSON 增加字段**。后端现有查询继续用 bundle hash 和 projection ID 绑定作用域；同一导入范围含多个 attempt 是有效合同，不按列表最大 attempt 误过滤。前端标记校验不是新的授权机制，也不能独立证明后端查询正确。
- 新增共 6 项 UI 回归，覆盖关闭/切换、错误文本、异常合同、多轮次展示、分页失败后重试及卸载。初次专项 **25/25**；补齐卸载与异常数据案例后完整 UI **426/426**，类型检查和生产构建通过。主 JS **309.52 kB**、任务中心 **38.49 kB**。日志 `/tmp/oviraptor-history-ledger-{red,green,ui,build}.log`。
- 已有 Rust `canonical_history` 只读预览/台账兼容测试 **2/2** 通过，采用全目标/全特性编译、离线单作业单线程；这是名称过滤回归，不是重新跑完整 Rust/导入器测试。日志 `/tmp/oviraptor-history-ledger-rust.log`。未修改 Rust、原始历史文件、执行链或授权门禁，未部署；整体 Master Plan 和安装态来源绑定仍未完成。

## 2026-09-29 安装态身份只读核对

同日再次比对：发布比对器回归 **7/7**；实际两个应用仍各 15 文件、5 处差异，CLI exit 2。报告 `/tmp/oviraptor-release-bundle-recheck.json`；回归 `/tmp/oviraptor-release-bundle-recheck-test.log`。当前 `src-tauri/build.rs` 仅调用 `tauri_build::build()`，现有发布工具不记录或验证本次构建的源码输入绑定，因此不能通过版本号、修改时间、目录比较或手工补写一份清单追认 §18.15。后续须由受控构建流程采集输入身份、构建命令/环境和输出摘要，验证源码在构建期间未改变，再核对安装产物与该构建输出，并独立完成桌面验收；本次没有生成这种证明，也未安装或运行应用。

Master Plan §18.15 尚未通过。实际读取 `/Applications/Oviraptor.app/Contents/Info.plist` 与构建目录的同名文件：两者可执行文件名均为 `oviraptor`；安装态版本为 1.1.59，当前打包配置也为 1.1.59，版本号不足以证明源码一致。

| 文件 | 修改时间（+0800） | 字节 | SHA-256 |
|---|---|---:|---|
| `/Applications/Oviraptor.app/Contents/MacOS/oviraptor` | 2026-09-24 15:26:31 | 30270016 | `cc9fa6e5d76c631876f00cee57efb89c1f732f92caab74680778928779ddfe24` |
| `src-tauri/target/release/bundle/macos/Oviraptor.app/Contents/MacOS/oviraptor` | 2026-09-29 13:25:25 | 36113552 | `f635dacf5252729a823ba30e75a861c2cb26c820c8546feeec97b7ab0d917300` |

上述由 `plutil`、`stat` 和 `shasum -a 256` 实测，证明两个二进制不同，不证明其中任一个与当前源码一致。构建目录二进制的记录时间也早于本轮聊天修复。没有重新打包、替换安装应用或启动真实任务；时间戳不是可复现构建来源证明，不据此判断安装态的活动后端。后续发布仍须绑定源码/依赖/资源清单和产物摘要，并验证实际安装文件与经过验收的产物一致，不能仅检查产品版本号。

### 同日补充：可重复的只读产物核对

- 发布专用工具放在 `tools/release/compare_app_bundles.mjs`（94 行），独立回归为 `tools/test_release_bundle_comparison.cjs`（91 行）；未抽取无实际复用的公共层，未新增依赖或修改生产代码。`npm run test:release-bundle` **7/7** 通过。
- 两个上述应用目录各读取 **15 个文件**，比较返回 **exit 2**（已发现差异，非检查异常）：主程序、`8_js_ast_analyzer.cjs`、`9_frontend_runtime_probe.cjs` 内容不同；安装目录缺少 `resources/config/browser-locations.json`；安装目录额外包含 `Contents/_CodeSignature/CodeResources`。资源路径均相对 `Contents/Resources/`。原始报告为 `/tmp/oviraptor-release-bundle-comparison.json`，临时日志不作为永久归档。
- 工具比较文件内容摘要、大小及执行权限位，拒绝符号链接和特殊文件；只用于静止的应用目录，不是对抗并发文件系统修改的安全边界。不比较空目录、所有权、ACL、扩展属性或全部权限位，也不验证签名；签名资源文件有差异不能据此认定签名无效。
- 报告明确保留 `sourceBinding: not_verified`、`runtimeValidation: not_performed`。即使两个目录相同，也不能证明来自当前源码或通过桌面运行验收。README 已补充用法，并纠正仓库中不存在的自动构建 workflow 说明。
- 本增量后定向退役检查 **7/7** 通过（全目标/全特性编译、名称过滤；不是重新执行完整 Rust 测试），未修改 allowlist。此前完整 Rust **1435/30** 与 UI **413** 的生产基线仍见实施状态表；本批没有重新打包、安装、部署或访问外部 URL。

## 2026-09-29 补充复核：打包输入来源检查

本批仅加强测试期的剥离检查，不改生产执行器、实际打包配置或用户任务的运行门禁。

- 原固定目录检查未约束打包配置引用的文件来源；资源列表/映射、平台配置可能引用扫描范围外的文件。首次负向回归中两项测试失败，证明来源检查缺失。自动读取的 `Info.plist` 又通过独立失败回归证明未纳入原扩展名清单。
- 根据本地安装的 `@tauri-apps/cli` **2.11.4** 的 `config.schema.json` 核对配置形式。资源和图标仅接受已递归检查的 `resources/`、`icons/` 来源，支持列表、资源映射及根下通配符；拒绝越界、绝对路径和不支持的来源形式。前端产物路径保持当前 `../dist`。
- 检查主 JSON 配置及平台 JSON 覆盖配置；JSON5/TOML 配置明确要求先接入经过审查的解析器，不静默跳过。非空 sidecar、安装脚本、平台额外文件等未接入的输入明确失败；Windows WiX/NSIS 配置采取保守整体限制。需要这些功能时应补充来源审查，不应直接豁免。
- 根级构建输入新增 `plist` 覆盖，包含隐式平台元数据。有效配置回归同时验证特殊扩展名资源及 `Info.plist` 内的惰性标记均可检测。
- 三项新增测试覆盖 **24 个拒绝案例、9 种配置文件名**及有效列表/映射/通配符和平台选项。夹具只创建本地临时文件，断言前清理自身目录，不启动工具或联网。
- 代码仍在原 `commands/agent_tests_backend_literal.rs` 测试模块，**307 行**；没有新增文件、依赖或公共层抽象，没有删除独有回归。仅人工审核并更新该文件一条 allowlist 理由、完整摘要及字面量计数（8 → 10）；两处新增字面量是惰性测试标记，活动符号豁免未增加。
- 本批定向退役检查 **7/7**、活动残留检查 **6/6** 通过。它们使用全目标/全特性编译，但按名称过滤测试，**不是 Rust 全量测试通过**。Cargo 串行、低优先级、单编译作业、单测试线程运行。
- 全目标/全特性严格 Clippy（`-D warnings`）、fmt、`git diff --check` 通过；另对本批三个文件逐一检查尾随空白和末尾换行，覆盖未跟踪文件。

边界：这是静态来源策略，不是完整通配符解析器，也不验证所有匹配文件存在。未覆盖 CLI/环境注入的配置覆盖、后续复制步骤、依赖构建行为、最终二进制及安装后的应用；没有验证跨平台路径规则的全部边缘情况。本批没有重跑前端或部署应用、访问外部目标。Master Plan §18 仍未完成，不能据此宣称 Strix 的安装包级剥离已全部验收。

## 2026-09-29 补充复核：历史导入根目录边界

以下是当前补充批次的证据；后文 2026-09-26 的测试数量、构建大小和运行描述属于当时快照，不代表当前工作树已重新全量验收。

- 发现 `artifact_import/discovery.rs` 原入口通过 `Path::is_dir()` 判断根目录，会跟随末端符号链接，并静默跳过非目录。三个新增回归先失败，其中链接指向的结果实际进入了临时导入数据库。
- 现在先对仅作语法规范化的根路径调用不跟随末端链接的 metadata 检查；拒绝目录/文件链接、悬空链接、带尾随 `/` 或 `/.` 的链接写法，以及 FIFO 等特殊文件。普通文件报 `root_not_directory`，其他读取失败报 `unreadable_root_metadata`；不存在的默认历史目录仍可跳过。诊断和来源路径保留原始输入。
- 新增四项回归，覆盖健康兄弟目录继续导入、缺失默认目录兼容、FIFO 根拒绝，以及已导入目录被链接替换后不撤销历史结果、恢复目录后可重试。拒绝根不会产生 bundle outcome 或新增结果对象/修订；服务仍可能登记来源，不宣称数据库完全零写入。源树指纹保持不变。
- 实现 **299 行**、相关测试 **240 行**，均位于原 `artifact_import` 业务模块。没有新增文件、依赖、公共层抽象或重复测试入口；没有删除独有回归。本轮失败用例留下的可再生 FIFO 夹具已按确切路径清理，未删除用户数据。
- 只人工审核并更新 `backend_retirement_allowlist.json` 中该实现文件的一条完整 SHA-256 与用途说明；历史文件名字面量仍仅用于只读结果识别，没有放宽扫描范围或增加活动符号豁免。
- 本批历史导入相关 **90/90**、修改后精确字面量守卫 **4/4**、活动残留守卫 **6/6**、导入 CLI 全部 **30/30** 通过；严格全目标/全特性 Clippy（`-D warnings`）通过。Cargo 使用低优先级、单编译作业、单测试线程，且串行运行。
- 本批未改前端，没有重跑 UI 构建或 Rust 主库全量；此前 UI **374/374** 不是本批重新运行的结果。未部署、未访问外部目标，也未验收安装包、Linux/Windows 或真实模型与沙箱联合运行。

边界：本次证明的是所选根目录末端链接的拒绝，不是对祖先目录链接或并发目录替换竞态的全面隔离；不能据此将 COR-006 或 Master Plan §18 标记为全部完成。

## 1. 删除仍在生效的旧恢复协议

原 `llm_hook_context.rs` 会识别历史恢复文案，在上下文没有超限时也触发压缩，并将工具限制为生命周期工具。先红测试显示：34 个工具被缩减为 1 个；短请求也被修改。这不是历史只读兼容，而是仍在改变当前模型请求的运行逻辑。

修复后的规则：

- 删除历史恢复文案识别函数、专用工具过滤函数及其命名解析辅助函数。
- 上下文压缩只按已有的上下文压力阈值触发，不按请求中的旧提示词触发。
- 保留现有历史压缩、描述缩短和审计记录能力，不删除工具名字或参数 schema。
- 审计 `reason=context_headroom`，兼容保留 `filteredTools=0` 字段。
- 旧恢复文案只保留在负向回归 fixture 中；小请求保持字节一致，大请求的 34 个工具名字和参数全部保留。

先红日志：`/tmp/oviraptor-20260926-retired-hook-red.log`。定向通过日志：`/tmp/oviraptor-20260926-retired-hook-green.log`。这些测试不宣称压缩策略能够无损保留任意长对话的全部内容。

## 2. 配置迁移纠偏，不改动 Master Plan 的要求

Master Plan §11.6 的正式字段是 `legacyArtifactDirectories`。前一轮记录里的 `historicalImportDirectories` 是实现偏差，不应据此修改计划。

现在按以下顺序读取并移除旧字段：

1. 正式 `legacyArtifactDirectories` 已存在时，以它为准，包括空串和 null。
2. 正式字段不存在时，接受中间版本 `historicalImportDirectories`。
3. 两者均不存在时，接受最早版本 `strixRunsDirectory`。
4. 输出只保留正式字段，确保已保存的旧 JSON 和中间版本 JSON 都能继续迁移。

删除 `agentBackendPolicy` 默认值、配置迁移输出及前端后端选择器。`strixExecutable` 继续仅作为待移除的历史配置键，不参与可执行程序选择。代理和前端证据预算的旧字段兼容收敛到统一归一化函数，不在执行消费者中保留散落 fallback。

## 3. 历史更新说明不能冒充当前功能

`ReleaseNotesDialog.vue` 原先把历史安装、升级、恢复和工具限制说明直接展示在当前版本标题下面。现在先展示 Native 当前路径说明，旧正文完整保留在默认折叠的“历史更新说明归档 · 2026-08-24”中，并明确声明它不是当前功能清单。

不改写历史正文，也不把旧版本曾经支持的功能当作当前能力。历史结果来源的只读标签继续保留。

## 4. REM-012 全文残留守卫

实现位置：

- `src-tauri/src/commands/agent_tests_backend_literal.rs`
- `src-tauri/tests/backend_retirement_allowlist.json`

审阅后的登记为 **59 个文件**：14 个 importer、6 个 migration、34 个 fixture、5 个 historical_label。每个文件记录具体用途、完整内容 SHA-256、大小写不敏感的字面量次数以及文件路径是否含旧名称。对于 UTF-8 文本，仅归一化 CRLF；不忽略注释、空白或匹配行周围的代码。

固定扫描范围包括前后端源码、工具与脚本、运行资源、capability 清单、图标、public，以及根目录和 src-tauri 下的构建/配置输入。目录扫描不按文件扩展名过滤，能检查嵌套文件和二进制内容中的字面量。缺失/不可读输入、符号链接、未知文件类型均失败，不跳过。新增匹配、失效登记或已登记文件上下文改变都会失败。

扫描范围不包括构建产物、依赖安装目录、设计文档和历史数据 fixture 目录；后者由既有冻结 manifest 与导入验收覆盖。allowlist 自身不参与递归摘要，避免自引用；其类别和理由格式在测试中检查，内容仍需要代码审阅。

这是静态字面量审查，不是任意混淆代码、动态构造命令或所有网络行为的完备性证明。必须与旧活动符号零豁免守卫、运行陷阱、迁移测试和最终安装包验收一起使用。

### 守卫自身的验证

- 混合大小写、只在路径出现、非 UTF-8 二进制内容、非常规扩展名均能命中。
- 同一匹配行但周围代码改变，摘要改变，不能沿用旧审阅。
- CRLF/LF 等价，避免仅因跨平台检出产生误报。
- 缺失输入和符号链接明确报错。
- 本轮修改 CLI 陷阱测试后，未更新摘要的守卫真实失败，精确指出 `agent_tests_backend_residual.rs`。审核该修改后只更新对应登记。

失败对照日志：`/tmp/oviraptor-20260926-allowlist-context-red.log`。扫描器定向日志：`/tmp/oviraptor-20260926-literal-gate.log`。

### 后续维护要求

禁止写“一键把所有当前匹配登记为允许”的 CI 修复或自动放行逻辑。修改命中文件时，先检查该文件所有旧名称用途是否仍属于四种合法类别；发现活动逻辑必须删除/重构，再更新该文件摘要。若旧字面量已全部移除，删除失效登记。活动符号 baseline 必须保持空对象，不能添加豁免。

## 5. 修复 REM-001 测试的假阴性

原测试只在临时目录创建了名字为 `retired-backend` 的脚本，没有将它以真实 CLI 名称放入 `PATH`。因此哨兵不存在并不能证明运行时没有尝试查找/调用旧程序。

新增 POSIX 子进程验收：

1. 在专属临时 bin 目录创建名为 `strix` 的可执行哨兵。
2. 只给测试子进程设置 PATH；不修改父测试进程及并行兄弟测试的环境。
3. 用产品的 PATH 解析器确认解析到假 CLI，再用真实进程调用写入哨兵，作为正向对照。
4. 删除对照哨兵，执行真实 Native Web 每目标入口，要求 Native 日志存在且无 CLI 哨兵。
5. 分别执行 Code、Greybox、CI 的真实源码分支，要求非空源码被处理、Native 冻结计划持久化、缺少可选分析器时留下缺口，且不调用旧 CLI 兜底。
6. 子进程完成所有分支后才写完成回执。父进程验证回执，防止测试名不匹配导致“0 个测试成功退出”被误判通过。

定向日志：`/tmp/oviraptor-20260926-cli-trap.log`。最终全量日志还会覆盖新增回执检查。

范围限制：该 PATH 陷阱为 POSIX 测试，当前在 macOS 运行；不是 Windows 安装包验收，也不是所有系统调用的进程追踪。Greybox 此处验证源码分支，Web 每目标入口单独执行，不能据此宣称完成整个 UI 创建 Greybox 任务的端到端验收。源码缺分析器分支不等于实际分析器镜像已执行或安装成功。

## 6. 仍需继续的总体验收

- 对照 REM-001…012 逐项聚合源码、配置、入口、迁移和运行证据；不能把单个静态守卫通过写成 Stage 5 整体完成。
- 发布安装包内容、启动网络行为、Linux 运行环境和实际隔离沙箱尚未在本轮验证。
- assignment → child-run → lease → mailbox → 独立 Reviewer → 用户指令 → 真实聊天时间线仍需按 Master Plan 做逐项闭环核验，不能只按已有测试数量判断。
- 当前前端主 chunk 大小告警仍然存在，需要在保持任务状态、页面边界和懒加载正确性的前提下另行处理。
- 真实授权 URL 测试、登录态测试与长期任务稳定性不在本轮完成证据中。

## 7. 本轮最终验证命令

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
npm run build
node tools/test_native_runtime.cjs
git diff --check
```

最终代码验证结果：

- Rust 主库 **655 项**、历史导入工具 **30 项**全部通过；最终日志 `/tmp/oviraptor-20260926-retirement-verified.log`。包含新的 CLI 陷阱、完成回执、全文残留守卫和旧恢复文案回归。
- 严格 Clippy（`-D warnings`）通过；日志 `/tmp/oviraptor-20260926-retirement-clippy.log`。
- fmt 与 `git diff --check` 通过。
- 前端 build 通过；日志 `/tmp/oviraptor-20260926-retirement-build.log`。主 JS chunk **767.50 kB** 告警仍在，没有通过调高阈值隐藏告警。
- Native 本地浏览器回环通过：**8 次请求**，匿名采集与身份隔离通过；日志 `/tmp/oviraptor-20260926-retirement-native.log`。

前端和浏览器验证之后的额外修改仅涉及 Rust 验收测试、对应 allowlist 和本审核文档，没有再修改前端或浏览器 worker。Rust 最终日志是在 CLI 陷阱和回执修改完成后重新全量运行，不能用较早的 `retirement-final.log` 替代。
