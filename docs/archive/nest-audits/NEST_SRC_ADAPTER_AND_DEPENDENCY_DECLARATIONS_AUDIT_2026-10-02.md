# SRC 无授权发送退役与 Dependency 声明材料：2026-10-02

本批两项前置已实现并完成受影响门禁，Master 仍在开发。不是完整 Concurrency/Dependency 角色、最终全量、安装包或授权 URL 验收。

## 问题与真实作用域

- SRC/OAST receiver 在 Native run 注册前开放 `/raw`、`/race`；URL token 没有绑定 DB、attempt、原 Root/C/worker、身份、能力或预算。race 默认 8 个线程/16 次请求，最高 64 个线程；调用方的 cleanup JSON 和 invariant 文本不是业务合同或清理成功证明。
- 冻结 SourceBroker 的 Cargo 声明读取按 `=` 行提取，混入 package/metadata/profile，不能解析真实依赖表；Gradle 被 XML 解析器处理，普通声明丢失、注释 XML 成为假依赖。作用域只有已密封、已选中、hash 核验的源码材料，不授予专家执行权。
- 全部实际执行只使用唯一临时目录/SQLite 和 localhost；没有真实业务库、CAS、资产、安装包、提交或授权 URL 操作。失败的临时 Source 夹具留作诊断，没有按目录前缀批量清理。

## 实际 red 与修补

- `/tmp/oviraptor-race-boundary-red.log`：1/4，通过项为既有显式数量合同；三项真实失败分别看到四线程 4 次 I/O、缺省 16 次 I/O、非法 concurrency 变成 8 次 I/O。数量读取现在缺省 2、硬限 3，非法/不足/超限参数在 I/O 前拒绝，不 silently clamp；`attempts<=128` 只是保留的旧传输上限，没有授予请求权限。
- `/tmp/oviraptor-src-adapter-authority-red.log` 是夹具编译失败：当前 reqwest 未启用 Response.json，不是行为 red。改为文本解析后 `/tmp/oviraptor-src-adapter-authority-red-fixed-fixture.log` 实际 0/2，证明无 run/worker 的真实 raw 接口仍发出目标请求，manifest 还报告可用。测试先完成 raw/race × GET/POST 四种真实调用和有界 teardown，再断言；不留活 listener。
- 生产 receiver 现在对这两个未绑定入口返回 403/`adapter_worker_contract_required`，不创建 job、不解析请求合同再发送。被动 OAST callback/poll 保留；manifest commands 为空，raw/race/controlledWrite 明确 unavailable，Web 能力清单一致。底层旧 raw/race transport 整体只在 `cfg(test)` 编译，不能从生产包调用。后续须接真正的不可变 worker/Broker 合同后再开放新的发送路径。
- obsolete 的“未授权 raw 仍可发送且 callback 可用”正测已移除；对应 callback 正向合同保留，四种拒绝后仍可 callback 的真实 receiver 合同替代其语义。其余三项旧 binary/duplicate-header/显式数量/拒绝合同按原模块名完整移入独立 95 行文件，没有删掉字节保真覆盖。
- `/tmp/oviraptor-dependency-declarations-red.log` 实际 0/4：Cargo 假依赖、Gradle 注释伪依赖/声明缺失、非法声明成功、冻结 Gradle 答案为空均复现。先应用 TOML 表/静态 Gradle 解析后 `/tmp/oviraptor-dependency-declarations-green.log` 为 2/4；合法 Cargo 被错误的 Value 单值解析入口拒绝，不能记为 green。根据本机锁定 TOML 源码改用文档解析，`...-green-document.log` 最后 4/4（2.85 秒）。
- Cargo 只读取 dependencies/dev/build/target/workspace 声明，原版本要求不求解；Gradle 只接受 dependencies 块中的受限字面量/静态 named 参数/platform，动态/未知表达式明确拒绝。没有执行 DSL、包脚本、网络解析。原四个 Native 顶层字段及每行字段保留，没有 resolvedVersion；别名、平台、scope、classifier 等仍须后续 typed 材料，不能据此做精确 advisory 结论。NPM/Python/Go/pom 既存局限未由本批解决。
- Cargo.toml 仅新增已在锁中的 `toml = "=0.9.12"`；Cargo.lock 仅向本包依赖数组添加既有 `toml 0.9.12+spec-1.1.0`，不改包版本/checksum。Dependency 与最终门禁使用 offline/locked；SRC 最初红/绿沿既有锁编译，没有主动更新依赖或安装工具。README 两处旧格式/旧 backend 别名的过时说明已纠正，既有未提交内容保留。

## 最后门禁与保存证据

- 新增 9 个合同，移除上述 1 个 obsolete 正测，主库 1777→1785。最终受影响 `/tmp/oviraptor-src-dependency-final-affected-postfmt.log`：80/80、exit 0、80.96 秒，覆盖全部新增及实际 SourceBroker/权限/View/专家/Native/SRC 调用者。前 80/80（80.59 秒）在两叶格式化前；集合重叠，不相加，也不是完整 1785 项门禁。
- `/tmp/oviraptor-src-dependency-final-clippy-postfmt.log`：严格 all-targets/all-features、exit 0、12.42 秒；前 Clippy 39.37 秒为格式化前。`...-final-importer-postfmt.log` 39/39、exit 0、2.58 秒；`...-final-literal-postfmt.log` 1/1、exit 0、0.70 秒。曾以非完整名称加 --exact 导致 0 项，原日志 `...-final-literal.log` 不能作证明，已用正确过滤实际执行守卫。
- 14 个修改 Rust 文件作用域 fmt 和差异检查通过。初次检查发现两个既有叶的排版不符；仅格式化修改集，变化限 Web policy、code-rule tests 两叶，其余 12 叶字节不变。保存于 `/tmp/oviraptor-src-dependency-before-fmt.json` 和逐文件 fmt 差异，最后 80/Clippy/import/字面量均在格式后重跑。
- 代码 9 已有+7 新增，无删除/范围外代码变化；另有 README 两句修订。新文件最大 277 行，所有本批手写文件≤400；既有 transport 489→333，三项旧测试拆为 95 行叶。没有全局格式化、重置、批量覆盖、commit/push。
- 基线 `/tmp/oviraptor-race-boundary-baseline.json` 为 900 路径 `512b8a98ff32fedd68b8e2cea90a449b245a63dfba32c1b895e7b226fba08ae6`；最后 `/tmp/oviraptor-src-dependency-code-snapshot.json` 为 907 路径 `fa69137af0c65dad46375d0dbe057d61f10a01156e32fa6f9957138a3fb530f0`。README 单独 SHA-256 为 `3d425832300c3c526a41ba8b22560dfd2dc4256027b4ab9b80b32abbc2bbf499`。HEAD 保持 `59be3d86d25adda5b1f975759bf256ef3b94f32b`。
- 原字节、已有 Git 差异和本批差异分别在 `/tmp/oviraptor-race-boundary-before.json`、`/tmp/oviraptor-dependency-declarations-before.json`、两组 `*-prior-diffs`、`/tmp/oviraptor-src-dependency-diffs`；作用域/行数在 `...-scope.json`。只读检索中曾有错误路径/草稿 README 尚未存在，这些诊断不是门禁结果。

## 下一批与未完成

继续全新 Root 的显式 v2 budget sidecar：普通 lane slot 与真实 concurrency_batches 分开，同新 Root INSERT 原子冻结，不对任何旧 Root（包括零费用）回填授权；完整重派/保存回执/terminal/trigger/REPLACE 负测后再接普通用户模式与 Root SDK tick。

Concurrency 仍缺独立 worker、精确 batch 全量预算预留、实际 baseline/串行控制/2–3 同时请求/最终状态和清理义务；本批封旁路不等于角色完成。Dependency 仍缺 lock/SBOM/离线 advisory/可达性、独立 SDK/候选/Reviewer。其余角色、Root 完整费用恢复和 elapsed/动态分配、Source/Reviewer 重派/canonical、聊天 revise/reject/有序动作、AST/browser/Native 逐路真实日志继续开发。最后才完整门禁、当前安装态和两个已授权 URL；不在局部绿色停止，不标 Goal/Master 完成。
