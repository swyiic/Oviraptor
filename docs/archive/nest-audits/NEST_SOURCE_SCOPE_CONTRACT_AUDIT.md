# 源码请求范围冻结合同审计

状态：已实现工作台发布事务中的按 attempt 源码请求范围冻结及源码执行入口校验。文件读取加固后的最终流水线 session 50125 已退出 0：新增范围合同 12 项、完整 Rust 1046／30、严格 Clippy／fmt 均通过；前端 146／构建／localhost 已取得通过终态。**这不是实际 diff 分析视图、源码独立 Reviewer 或整个 Master Plan 已交付的声明。**

## 1. 缺陷与修复边界

之前 `task.json` 有 `scopeMode/diffBase`，但 `WorkbenchStartRecord` 不接收这两个字段，发布事务没有不可变范围合同。源码线程接受调用者的源码根目录、扫描类型和 base；在同一个合法 CI attempt 下传入另一个目录，仍会捕获并分析。传入 `code` 也能绕过本应加载的 CI 发布策略。

这轮先解决“请求的执行范围由谁决定”，不把它混同于“实际分析文件集合”。原来的整个来源快照校验继续保留；下一增量仍需把真实分析视图与 CI scope 接通。

## 2. 实现及不变量

实现文件：

- `commands/native_source_scope.rs`：范围合同及发布／运行校验。
- `commands/workbench_startup.rs`：接收范围字段，同一事务内保存合同，全部后续写入后再次验证。
- `commands/native_source_scan.rs`：执行前加载，校验线程参数；每个分析器前后、取消探针及最终投影验证当前绑定。
- `db_schema.rs`：`source_scope_contracts` 及不可变触发器。

合同绑定 scan／attempt、原始选择路径、canonical root、scan type、请求的 scope mode 和有效 diff base。

1. 只允许 `code/greybox/cicd` 和 `full/diff/auto`；源码根目录必须存在且为目录。显式 diff 不接受空 base；非 full 的 base 长度上限 1024 字节且不含 NUL。
2. Full 的有效 base 固定为空，忽略 UI 中隐藏的旧分支。Diff/auto 对 base 仅做首尾空白规范化，不以全局设置替代。
3. 发布时检查真实 `task.json` 的 scan／attempt／source／type／scope／base 与记录一致；缺失、损坏、软链接、目录／FIFO、超过既有 16 MiB 上限或字段不匹配拒绝发布。读取复用工作台已有的限量读取器：打开后的文件句柄检查类型与长度、限制读取字节，Unix 使用 NOFOLLOW／NONBLOCK／CLOEXEC；不只依赖读取前的路径元数据。全部发布写入后再次核对，不允许后续触发器改变绑定却成功提交。
4. 合同与 attempt、CI 策略和派发记录处于同一个发布事务。写入失败或被 `RAISE(IGNORE)` 忽略会整体回滚，不留下可执行的半发布任务。
5. UPDATE、DELETE、REPLACE 被拒绝；删除父 attempt／scan 时允许外键级联清理。新尝试拥有独立合同，不修改旧尝试的范围。
6. 运行时不再从可变 task JSON 推断授权。必须存在当前 scanning scan／attempt 的合同，scan type/source path 必须仍与合同一致；历史任务缺少合同必须创建新 attempt，不能自动补授权。
7. 线程传入另一源码目录／扫描类型／非 full 的其他 base 会被拒绝。捕获使用合同中的 canonical root，选择路径的软链接改指会使检查或取消探针失败。
8. 分析器的取消探针能观察到合同缺失或绑定变化；不能据此承诺取消已经完成的外部效果。分析后重新检查，失败时不继续下一分析器、导入或发布 CI 结论。
9. 报告新增 `requestedSourceScope`，明确标为请求合同，不把它伪装为实际分析 manifest。

此合同不等于整个工作台冻结派发：work_dir、工具／规则版本、人工限制、成本预算和其他入口仍须按主计划完成各自绑定。

## 3. 回归与失败历史

`commands/tests_source_scope_contract.rs` 新增 12 项：

1. 真实发布事务保存绑定 attempt 的合同。
2. 替换源码或把 CI 改成 code：零分析器调用、零快照。
3. Full/diff/auto 保存及 base 规范化；非 full 不接受换 base。
4. task 字段缺失／类型错误／目标不一致、非法 mode／空 diff 拒绝且回滚。
5. 合同 INSERT 报错或忽略时，发布整体回滚。
6. 后续派发写入触发器改源码绑定／attempt 状态时，发布整体回滚。
7. 合同不可变、旧 attempt 保留、新 attempt 独立，父记录删除可级联。
8. 合同在执行前／执行中缺失，或 scan 绑定改变，不导入、不回退默认、不投影 CI；执行中取消探针返回 true。
9. 发布后的 task.json 被损坏不改变合同，Full 忽略旧 base；真实报告保存请求范围。
10. Unix 选择路径的软链接在执行前／执行中改指不能换到另一仓库。
11. task.json 被替换为目录或稀疏超大文件时拒绝发布、完整回滚。
12. Unix task.json 软链接拒绝，FIFO 非阻塞拒绝；不挂住发布事务。

日志：

- `/tmp/oviraptor-source-contract-red.log`，session 96359 退出 101：原实现下 2 项回归失败（缺少合同表、另一目录仍被执行）。
- `/tmp/oviraptor-source-contract-green.log`，session 82952 退出 101：给共享测试夹具加参数后遗漏旧调用方导致编译失败；保留无参共享入口，新增按 kind 的夹具入口修复，没有放宽生产校验。
- `/tmp/oviraptor-source-contract-green-v2.log`，session 57662 退出 0：原 2 项回归通过。
- `/tmp/oviraptor-source-contract-focused.log`，session 96227 退出 101：57 通过／1 失败；旧 source-only 发布夹具缺少现在必需的 task.json。补齐其源码选择文件，保留原“无 Web worker／无 Web branch”断言。
- `/tmp/oviraptor-source-contract-focused-v2.log`，session 73546 退出 0：源码相关主库 59、导入器 2 通过。
- `/tmp/oviraptor-source-contract-workbench.log`：工作台 40 通过。
- `/tmp/oviraptor-source-contract-routing.log`：Stage 4 路由 3 通过。
- `/tmp/oviraptor-source-contract-retirement.log`：残留守卫 4 通过。
- `/tmp/oviraptor-source-contract-reader.log`：读取器加固后全部新增 12 项通过（2.88 秒），已继续进入完整流水线。

## 4. 残留白名单审核

只更新两个已有测试夹具的摘要，没有新增条目、历史 literal 或可执行后端。用只读脚本逆转本轮夹具前置条件，逐字节重现先前已审摘要：

- `agent_tests_backend_residual.rs`：旧 `55571451c922a205b146c62a869df54c81888ff14e356350d3027714219ab947` → 新 `283731e7860364eb79da886c9bf212f48f0df7e3f52731fb69def7ef7581d5dd`。为三种源码任务补齐 attempt 与源码合同；CI 原策略不变，17 个 literal 不变。
- `tests_native_routing.rs`：旧 `ae72a9c7c8e4316fe11cf10709a8e8968dc796655622d54baed8666b62157fd3` → 新 `4fe7634783f11d1adbaed032bbe0fc328fe4c14f7a5a8cc877c5e4a593434d81`。补齐源码绑定与合同，code=full、CI=diff/main；原不可用 base、退出码和拒绝断言不变，4 个 literal 不变。

## 5. 最终组合验证

首版 Rust 顺序流水线 session 84379 已退出 0：工作台 40 → 路由 3 → 残留守卫 4 → 完整 Rust 主库 1044（445.49 秒）、导入器 30（2.01 秒）→ 全目标全特性 Clippy `-D warnings` → fmt 均通过。日志 `/tmp/oviraptor-source-contract-full.log`、`...-clippy.log`、`...-fmt.log`。

此后加固文件读取，复用既有 16 MiB 上限而非新设 1 MiB 限制，并增加目录／超大文件／symlink／FIFO 测试。最终流水线 session 50125 已退出 0：范围合同 12 项 → 完整 Rust 主库 1046（439.54 秒）、导入器 30（2.01 秒）→ 严格 Clippy → fmt 全部通过。日志 `/tmp/oviraptor-source-contract-reader.log`、`...-full-v2.log`、`...-clippy-v2.log`、`...-fmt-v2.log`。这是文件读取加固后的终态验证。

前端／localhost 流水线 session 68638 已退出 0：七组实际 SFC 146 通过 → vue-tsc／Vite 通过 → 本地浏览器回环 `passed=true`、匿名／比较采集 complete、8 个观察请求、身份隔离 true。日志 `/tmp/oviraptor-source-contract-ui.log`、`...-build.log`、`...-loopback.log`。主 JS 828.82 kB，既有 >500 kB 拆包警告仍保留。

验证期间冻结 Rust 输入，不因观察超时重启测试。未部署、未探测用户给出的外部 URL、未增加 Host Agent。

## 6. 接下来必须实现，不能用本轮替代

后续进展：以下为本审计当时的边界。独立实际分析视图、分析器真实输入、输出挂载隔离、恢复校验及 CI scope/fileCount/digest 已在 `NEST_SOURCE_ANALYSIS_VIEW_AUDIT.md` 实施并单独记录验证；SourceBroker 与源码独立 Reviewer 仍未完成。保留下面的历史缺陷，不作为最新实现状态。

1. 原整仓来源 manifest 保留；另建实际分析 manifest／材料化视图，绑定请求合同、来源 tree hash、解析后的 HEAD/base、选中路径／删除路径／未覆盖原因及自身摘要。
2. 分析器必须收到该分析视图的真实目录与字节。显式 diff 不隐式扩大为 full；auto 的回退语义需要用户可见且与冻结选择一致。当前 `AnalyzerSpec.repository` 仍使用整仓 `snapshot.frozen_root`。
3. 恢复必须校验请求合同与已保存来源／实际视图一致，不能只检查源码路径。材料化、校验或持久化失败不得回退其他范围。
4. 删除、排除、超限、未知清单、空增量都需真实覆盖结果。CodeQL 的项目上下文单独定义，不给残缺目录却声称完成项目分析，也不偷偷扩范围。
5. 当前 CI 仍硬编码 `CiScope::Diff`，fileCount 仍来自来源快照；下一增量必须让 CI scope／fileCount／manifest digest 对应真实分析输入。请求合同报告字段不是实际范围证明。
6. SourceBroker 的读权限也需绑定实际范围；随后接真实 source assignment→child-run→mailbox→独立 Reviewer，并修复 CI root／attempt／当前候选 revision 资格和去重。
7. Git 读取的统一 deadline／输出上限、完整工作台派发冻结与预算、其他执行入口撤权及全部 Master Plan 要求继续进行。
