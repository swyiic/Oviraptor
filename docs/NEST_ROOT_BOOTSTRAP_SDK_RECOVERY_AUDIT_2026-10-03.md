# Root 真 SDK、原付费恢复与稳定 Mapper 输入审计（2026-10-03）

## 状态与作用域

Master 开发中。这批交付普通 Web Multi Root 的真实 bootstrap 模型决策、原付费语义保存/本地发布恢复与 Mapper 重入输入稳定性，不交付完整 ReAct、持续全部触发、全角色或安装验收。当前 Native JSON、原 Root/control/C/十限额/createdAt 原点保留；旧无原凭证 Root 不补授权，不读取 UI 当前值作为财务权限。

所有行为运行于隔离临时 SQLite、真实 localhost HTTP SDK及测试子进程。SIGKILL 为实际进程强杀。没有真实业务库/CAS/资产、Oviraptor 安装包、两个授权 URL、提交/推送操作。原已有未提交改动逐文件保留；HEAD 仍 `59be3d86d25adda5b1f975759bf256ef3b94f32b`。

## 实际问题与最小修复

- 原调用可保存费用，但严格公开语义没有持久 request/decision/publication 闭环；付费发布失败或 SIGKILL 后，同一生产 API 不能恢复。现在先原 claim、真实 one-shot SDK、原发票与脱敏有界语义提交，再进行独立受保护本地发布；保存后恢复不重发 SDK、不重收费、不换 C/control。未知费用保留 indeterminate 并阻断新工作。
- Root SDK 的 tools 为空、输出上限至多2048，使用原 Root 时钟。只接严格五类公开摘要；raw/私有 reasoning、坏 schema/tool call/截断不能获得语义发布权，费用仍归原 owner。费用建议不是账单或目标权限。
- 原 evidence 文件在实际 SDK 中变为语义相等的 pretty JSON时，旧路径会把当前字节当成原事实。现在在 claim 和最后 publication 前后比较同一个捕获 basis；费用/合法原语义保留，原文件字节精确恢复后只本地发布。
- 已脱敏 auth 标记被全局 redactor 再散列，保存语义回放会误拒。只在原付费保存语义的验证中归一化合法单向标记指纹；首次 raw 脱敏和全局 redactor不改，残留 raw、错 kind/格式/大小写仍拒绝。
- 原 publication 未绑定 event 的 id/created_at，普通 UPDATE 后可被回放采用。现在原 immutable publication 捕获 event 全物理行（rowid/id/root/seq/type/payload/refs/time），末写与每次回放都精确比对；旧缺 eventProof 格式拒绝，不补写或重发。
- 实际扩大回归发现 Root 的 `replayed` 临时标志和后续 `multiAgent*` 派生投影进入 Mapper 输入，使已付费原请求哈希变化。只修 Mapper 输入 producer：派生结果不作新的前端源证据，保留原 Root 稳定公开摘要/引用，replayed留在诊断投影、排除于 Mapper 请求。原 specialist hash门禁不改，原 paid request JSON不重写；真正不同请求仍拒绝。
- 原 Reviewer/human预留从共享剩余容量分配，并扣除 Root 已付费/预留，不能用 child coarse ledger 漏掉 Root成本来增额。新 Root财务 owner仍仅真实 creator冻结，generic initialize/control wrapper是纯原始凭据/活权限读取。

## 实际红绿与未到达的断言

| 阶段 | 实际结果 | 日志（/tmp）与边界 |
|---|---|---|
| g2真实原入口 red | 5/11，4.34s | `oviraptor-root-tick-sdk-red.log`；actualSDK、付费 checkpoint与SIGKILL均执行。prepare先红于缺publication，未到headroom断言；expiry合同先红于缺decision，不能据此称已观察expiry漏洞。paid probe无显式env时是空 helper。 |
| g2初修+原control prepare | 10/12，6.34s | `oviraptor-root-tick-sdk-green-initial.log`；同API恢复/强杀恢复/headroom及原control prepare通过。两个故障过早安装导致request前被authorizer拒绝，不能算付费路径失败。 |
| 故障阶段诊断 | 0/2，1.70s | `oviraptor-root-tick-fault-phase-diagnostic.log`；明确 after_fence/collateral的seen0。SQLite prepare授权完整trigger程序，WHEN不豁免。 |
| 原付费阶段修正 | 12/12，6.87s | `oviraptor-root-tick-sdk-green-fault-phase-corrected.log`；publication先真实付费提交再装fault，本地0SDK；semantic fault由真实SDK handler在request提交后安装。生产guard不放宽。 |
| 原字节/脱敏回放 red | 1/4，1.15s | `oviraptor-root-tick-sdk-bytes-safe-red.log`；字节首拒断言及两真实secret回放红；raw/marker相邻负向已通过。 |
| 字节/脱敏修复 | 16/16，8.49s | `oviraptor-root-tick-sdk-bytes-safe-green.log`；含原SDK/强杀/权限/财务回归。 |
| v2与两个旧正向扩大 | 14/15，9.48s | `oviraptor-root-tick-sdk-v2-actual-positive-affected.log`；仅真实saved Mapper重入hash红。正向使用真实新creator/HMAC/原workdir，真实新增Root请求和费用如实计入。 |
| event物理proof red | 0/2，0.90s | `oviraptor-root-tick-sdk-publication-event-red.log`；普通id UPDATE首例红，timestamp红循环未到达；缺proof是明确隔离历史腐损夹具，不是普通可写活路径。 |
| event物理proof修复 | 18/18，10.97s | `oviraptor-root-tick-sdk-publication-event-green.log`；id/time两例恢复和旧缺proof拒绝均通过。 |
| Mapper派生输入独立red | 0/1，0.60s | `oviraptor-root-mapper-stable-input-red.log`；真实Root+Mapper2请求后same API重入报原hash冲突。 |
| producer修复相关 | 34/34，20.50s | `oviraptor-root-tick-sdk-final-affected.log`；原SDK18、新Mapper1、v2和两真实旧正向15，集合不相加。 |
| 格式/严格机械修正后最后相关 | 34/34，21.27s，wall57.26s | `oviraptor-root-tick-sdk-final-affected-after-strict.log`；当前快照最后行为证据。 |

第一次 strict all-target/all-feature Clippy发现两个 cmp_owned及一个大 enum；不把 Value语义相等替代 canonical JSON原字节验证，只命名canonical字符串并box已保存decision，最后严格退出0，28.99s：`/tmp/oviraptor-root-tick-sdk-clippy-mechanical-corrected.log`。失败保留在 `-clippy-initial.log`。两个未用 import先独立移除，未加allow跳过门禁。

最终导入器39/39（2.85s，wall46.54s）：`/tmp/oviraptor-root-tick-sdk-importer-final.log`；exact退役登记1/1（0.66s）：`/tmp/oviraptor-root-tick-sdk-literal-final.log`。退役allowlist未修改。22个本批新Rust叶fmt check退出0，未全仓格式化：`/tmp/oviraptor-root-tick-sdk-new-leaf-fmt.log`。前端源码本批不变；之前UI证据不重算作本批真实聊天/安装验收。

## 文件与差异保护

本批38代码路径：15已有、23新增，新文件最大281行。38份相对本批preimage差异共3287行，原dirty diff与逐文件前值另存。基线1005路径SHA `f26aed8438987957f6c5db162bf15f1f9f63bd2d35fdabfa84c905c6a6c7acaa`；最后1028路径SHA `5e016f64be24c681594bf291b8a0a63101a6758a48786482506ce50a5cd9b043`，范围外990原路径逐SHA不变，HEAD不变，git diff check退出0。

快照/前值/旧差异/本批差异：`/tmp/oviraptor-root-tick-sdk-{baseline,before,scope-final,code-snapshot}.json`、`-prior-diffs/`、`-reviewed-merge-diffs/`；最终三Cargo序列：`/tmp/oviraptor-root-tick-sdk-final-gates.json`。 collection为src、src-tauri/src、src-tauri/resources、tools、package/Cargo/tauri配置与退役登记，未含build.rs/icons/capabilities/dist/docs，不能据此称全仓内容摘要。

原g2 01与original-control bridge共享hunk因当前include/上下文不适用，gitcheck失败后未写；只重建相同include和唯一initialize→load_original式。实际rebased patch SHA分别 `077b8f28207bbc3bee228b34883b2083a2c1d6b63c9d82fa13cb4a783757c48f`、`dddf940717560469ea62fb3132eaf65633d1b3bc299c786c34373731b70dc533`，其它旧sealed包SHA不改。统一tick fixture用before-birth原plan/endpoint/evidence，删除过晚acquire/coarse INSERT、修不存在的f.evidence字段；不把旧夹具错误算feature红。

独立审查已返回Root费用/公开摘要/原C/父/共享headroom链与event物理缺口，根逐hunk核验应用并独占Cargo。本批不声称38文件又有完整独立最终审查；三个并行审查进程后来因账户usage limit结束，未完成的/tmp草稿不能计交付。

## 仍未完成与下一步

1. g3真实Mapper/独立Reviewer变化→后续Root SDK/相同fact0SDK/原事实最后调度证明；normal paid worker不能强要仅晚到费用才有的model_cost_facts。随后纯typed trigger/overlap/Rust utility及实际paid Investigator反馈。
2. 完整§7.3的其余触发、真实本地tools/ReAct、恢复/暂停/对账/动态分配/唯一Reducer；当前Root仍tools=[]，严格公开摘要不是完整Codex式工具循环。发送前request物理身份捕获时点仍列静态风险，正常immutable schema阻止普通UPDATE/REPLACE，未伪造破schema测试作为活漏洞。
3. 所有角色/Broker、Source v2 slots/8/9/10 headroom、Source唯一终态/全outcome finally、Single finally/暂停恢复/费用唯一键与新INSERT侧写保护。
4. 已确认人工消息逐项原worker/SDK/费用/ACK、ordered未启动/关闭收尾、Root真正付费聊天与逐路SDK/Web/安装日志；/tmp尚未应用草稿仍未完成。
5. 完整门禁、当前安装App打开/强杀恢复/真实UI与最后两个授权URL匿名只读验收，登录身份待用户后续提供。InputParser先前自动审批拦截的部分仍未交付，不绕过。

Goal工具旧记录paused不等于完成；用户已连续授权继续，按该授权开发。Master/Goal未标complete。继续开发，不停在这批绿色。
