# Native Single 工具与 HTTP 原 Root 预算审计（2026-10-02）

Master 仍在开发中。本批完成已注册 Native Single 后端的工具/HTTP 原 Root 预算接线，不是 UI Single 模式、浏览器/写入/上传功能、真实模型供应商或整体功能验收。

## 问题与实际红测

- `/tmp/oviraptor-single-budget-red.log`：五项首次编译成功，四项真实 localhost 红测分别证明 target 费用为 0、每请求 timeout 未受原 Root 期限限制、真实 SDK HTTP 500 的未知模型费用未阻断新 HTTP、实际 handler 暂停 scan 后仍返回输出。第五项仅在初始化 1 秒期限夹具时过期，不作为漏洞证明；其孤立临时目录已逐项确认后清理。
- `/tmp/oviraptor-single-budget-expired-red-fixed-fixture.log`：仅修夹具为 4 秒/等待 4200ms 后，实际过期 Root 仍向原冻结目标发请求，0/1、exit 101。
- `/tmp/oviraptor-single-budget-exact-history-red.log`：2/3；六种非法 target 账目全部被装载接受。尤其未收到 headers 却已追加 reconcile 的夹具恢复了 live 准入。零写入/上传额度及真实 headers 后费用 INSERT 故障已经通过；不把它们声称为额外红测。
- `/tmp/oviraptor-single-budget-independent-review-red.log`：1/4；真实 SDK 绕准备入口接受 resume 祖先 Native checkpoint、生产 interrupted tool 后 generic timeout 仍放行、真实 GET 实际 headers multipart 绕 upload=0 均失败。fresh 任务排除旧 lineage 的正向保护通过。resume 红测先在 prior=0 失败；修复后 prior=0 和3均实际执行。
- `/tmp/oviraptor-single-budget-observation-postcheck-red.log`：0/1；共用 observation 提交路径漏了显式 active guard 的写后检查，实际 trigger 暂停扫描仍提交。
- 第一次严格 Clippy `/tmp/oviraptor-single-budget-final-clippy.log` 因 `cmp_owned` 失败。修复使用保存的 canonical 文本变量，保持原 JSON 字节比较；没有改成 Value 语义比较。找文件时几次猜测路径/通配符报错均为诊断失败，不算检查通过。

## 最小接线与数据作用域

1. 提取同一十维 frozen mapping，初始化/只读装载共用；模型零上限仍按当前 unlimited 语义，target/原 Root clock 与原算法不改。只读 `RootOwner::load_single` 绑定原控制 UUID、完整限额、原 clock origin 和双向精确 HTTP/费用历史；缺证据只拒绝，不写任何回填。
2. 首次生产准备在 `NativeAgentState::for_attempt` 之前。准备与直达 SDK 的原 IMMEDIATE 事务共用只读 Native parser、shared request usage 及预算 lineage 检查；原 owner 缺失时继承的任何 Native checkpoint/用量/旧 HTTP/partial limits 均不能建立新权限。fresh 排除旧 lineage，并逐字保留旧 Native JSON。Core 原语本身仍非任意调用者的完整 Native lineage 授权接口；当前两个实际生产 caller 已共用该边界，未来新增 caller 必须同样受约束。
3. Single fresh tool gate 检查原 Root、扫描、冻结计划、原期限和所有未知成本，涵盖本地检查/证据。模型已发出的 in-flight 复核仍跳过自己的 dispatch 债务，避免自己取消自己；后续新工作独立受 determinate gate 阻断。
4. 同一 private IMMEDIATE 事务持久原 HTTP claim 与 Root `target_requests` reserve/forfeit，再提交、发送。捕获 `RootTargetCall` 的原 UUID、原 HTTP immutable tuple/工具 allow 及 journal proof；发送前 typed timeout 重新证明原工具仍 running、原 pending claim和原期限，generic Single 没有原凭证一律拒绝。中断并不退款或重发。
5. 真 headers 到达后仅保存原 headers 与原 Root reconcile。晚到暂停/超期账单不借新 Coordinator/child、不授输出/新工作。receipt replay、binding漂移、IGNORE/ABORT和业务副作用拒绝并整事务回滚。reconcile INSERT 失败时 headers 也回滚，保留已发请求的 unknown=1。
6. Single observation/candidate 使用 IMMEDIATE 与写前/写后 Root 检查；private authorizer仅允许精确顶层 finding或原 HTTP/预算写入，拒绝 trigger/unrelated business/asset/DDL。共用显式 active guard 的 observation也在提交前重查。作者授权调用链没有内层清除外层的嵌套。
7. transport 与 claim 共用实际显式 headers 构造；新 claim hash 纳入合法显式 headers，不存原明文 headers/认证材料，也不重写已有 hash/Native JSON。所有实际合法 content-type（identity、extra和显式字段，包括重复字段/大小写/空白）任一 multipart 均按原 UTF-8 body bytes 计量。当前 frozen write/upload=0，在 claim/I/O 前拒绝。没有扩大为“所有 GET body 是写入”的规则。
8. 已注册 Single 的 host browser helper在尚无逐请求 Broker时明确拒绝，现有单 URL HTTP fallback 真正执行并只计费一次；helper marker始终不存在。现有 JS 同源限制仍存在；不能声称本批实现浏览器能力。

迟到 captured fee 仍是财务凭证：完整 limit/history、live、deadline及未知门禁属于 fresh/output 入口；不通过重建 control来收账。破损 DB 缺相关 target limit会失败，缺无关维度不能使后续 fresh装载成功。本批没有人为破坏真实库，也没有把未单独测试的破损 limit财务语义当验收证据。

## 作用域、结构与最后门禁

- 14 个已有文件最小增量，10 个新增文件，无删除、无范围外代码变化、无 git提交/重置/批量覆盖。所有新文件 ≤400 行，最大325；既有 browser（477）、executor（415）与共用 findings（422）的结构债保留，分别在逐请求 Broker、模式/执行循环、候选/观察事务责任拆分时处理；本批只加入窄调用/完整 journal责任提取。作用域格式化造成的既有排版变化已逐文件审阅，不宣称结构债清零。
- 基线890路径：`1210136b5ad12e14bb8945d0e80fea5833cfaaf32a1093d2d522ad2131f793e5`，`/tmp/oviraptor-single-budget-baseline.json`。逐文件 before、scope与增量在 `/tmp/oviraptor-single-budget-before.json`、`/tmp/oviraptor-single-budget-scope.json`、`/tmp/oviraptor-single-budget-diffs/`。
- 最后900路径：`512b8a98ff32fedd68b8e2cea90a449b245a63dfba32c1b895e7b226fba08ae6`，`/tmp/oviraptor-single-budget-code-snapshot.json`；所有门禁后 hash精确一致。HEAD仍 `59be3d86d25adda5b1f975759bf256ef3b94f32b`。
- 主库1755→1777，新增22项本批合同；真实localhost、actual SDK、原journal、全应用表快照及故障注入为隔离夹具，不是用户网站/真实供应商/安装App验收。
- 最后受影响 **316/316、exit 0、187.68秒**：`/tmp/oviraptor-single-budget-final-affected-postcheck.log`，包含新22项及 Root/worker/Multi/Web/Source/旧 HTTP/请求核对/Native loop消费者。前14/21/81/13/315为前序或重叠集合，不能相加或当最后状态。
- 严格全目标/全特性 Clippy **exit 0、28.32秒**：`/tmp/oviraptor-single-budget-final-clippy-postcheck.log`。
- 导入器 **39/39、exit 0、2.40秒**：`/tmp/oviraptor-single-budget-final-importer.log`。
- 严格退役字面量 **1/1、exit 0、0.54秒**：`/tmp/oviraptor-single-budget-final-literal.log`；24文件 scoped rustfmt --check/git diff --check及900路径最后复核通过。Cargo严格串行、nice15/-j1/tests单线程；编译期间未改源码。

新增文件：
- `src-tauri/src/agent_runtime/multi_agent/budget/limits/contract.rs`：69 行。
- `src-tauri/src/agent_runtime/multi_agent/budget/root/load.rs`：98 行。
- `src-tauri/src/agent_runtime/multi_agent/budget/root/target.rs`：216 行。
- `src-tauri/src/commands/agent_http_claim.rs`：179 行。
- `src-tauri/src/commands/agent_native/single_budget.rs`：230 行。
- `src-tauri/src/commands/agent_tests_single_budget_contracts.rs`：325 行。
- `src-tauri/src/commands/agent_tests_single_budget_target.rs`：298 行。
- `src-tauri/src/commands/agent_tests_single_budget_edges.rs`：229 行。
- `src-tauri/src/commands/agent_tests_single_budget_review.rs`：263 行。
- `src-tauri/src/commands/agent_http_wire.rs`：57 行。

## 未完成、风险与下一批

UI真实Single/Multi选择、Multi Root完整SDK/tick持久回执/本地发布、最终Root elapsed/精确费用对账/动态分配、真正browser/写入/上传grants和逐请求Broker、即时header取消、Single Source、持久worker恢复/安全Source及Reviewer重派/canonical owner、全角色真实执行/独立审查、聊天四态闭环、逐路实时日志与最终完整门禁/安装态/授权URL仍未完成。`concurrency_batches`当前还被child槽使用；未绑定worker的SRC race默认8/上限64及Dependency Cargo/Gradle声明误解析已有独立草稿，尚未应用/实际红绿，不能记角色完成。

本批仅隔离临时DB/localhost；没有真实业务库/CAS/资产/安装包/提交或用户两个URL操作。此前真实59行Nest清理由独立真实库审计记录，资产/CAS保留。本轮继续Master开发，Goal不标complete；工具当前卡片为paused不代表项目完成，也不撤销用户继续授权。
