# 原 Root/Source 创建财务凭证、期限与心跳审计：2026-10-03

## 状态与范围

Master 仍开发中。此批交付原财务创建与读取边界、Source 原时钟及心跳的有限写集合；不是 Source 全 outcome finally、完整 Root ReAct、全角色或整体功能验收。当前 Native JSON 保留，Source 既有 schema 1/2/3/4 与相应 8/9/10 请求语义没有被换成 v2 执行槽。

全部运行使用临时 SQLite、隔离 workbench/Git、localhost 实际 SDK 接收器或实际本地子进程。没有触碰真实业务数据库、CAS、资产、Oviraptor 安装或两个授权 URL；没有重置、提交、批量覆盖工作树。浏览器仅用于当前实际 SFC 的隔离视觉夹具，不是真实 Oviraptor 启动。

## 生产修复

- generic Root 初始化与直接 Single SDK 只读原 owner。旧无 Mode、零历史、零用量也不能通过准入、control 或 claim 补发财务身份。
- Source creator 在原 Root 尚不存在时捕获不可复制的 typed insertion loan；原 INSERT、首个 C1/UUID、原财务 control、十维限额、created_at 起点与 prepared 投影受同一个创建事务和 writer 保护。旧行与已有原账本不能取得该 loan。
- Source 顶层、实际专家 SDK、实际工具/初始 assessment 都核原 owner/C。真正正常调用费用使用原 budget receipts/entries、原 model journal/event/checkpoint；agent_model_cost_facts 是迟到结果分支，不能要求正常调用伪造该表记录。
- 新工作和恢复读取创建时的原时钟。started_at 是展示元数据；prepared 或 started_at 为空仍拒绝工具，过去/未来/invalid 的非空元数据不能重置或耗尽原限额。
- 专家和工具两路心跳的 C、worker、assignment、capabilities、Root/child 到期值不超过原期限。短任务仍活时可以工作；全部已到原期限上限时只读返回，不重复写心跳。
- 实际续租保留原 worker、准确权限集、全部 owned 行与统一期限证明，并要求 total_changes 差严格为 N+5：C1、capabilities N、assignment1、runs2、worker1。固定 lease/heartbeat 更新不发状态事件，任何额外业务/foreign/trigger SQL 写都拒绝并回滚；不安装、替换或清空调用者的 authorizer。此精确计数仅适用于该固定续租集合，不能外推一般 publisher。
- 原 financial 下层 prototype 正向测试显式发行 cfg(test) owner；没有生产 fallback。真实 Source 正向改用创建前配置及真正 same-INSERT issuer，历史材料和 missing-owner 负向保留。

## 实际红绿与失败收取

以下集合有重叠，不相加为功能覆盖率，也不称所有回归均绿。

| 合同 | 实际记录 |
| --- | --- |
| Source fresh financial issuer | 首 0/4 red，最初 14/14 green |
| 独立旧无 Mode Root SDK/control | 首 0/2 red，最小修复 2/2；扩大 103/104 的一个下层正向缺 owner，显式 cfg(test) issuer 后原 137 中通过 |
| original Source timer | 首 1/2，实际 mutable started_at 错误导致 SDK 早拒；原 clock 修复后两项在扩大回归通过 |
| 旧 Source direct SDK missing-owner | 首 0/1，旧原行确实到达 localhost SDK；纯原 owner/C gate 后相关 6/6 |
| 监督 stop 与锁竞争 | 旧 v1 正向缺原 owner；只此测试改 genuine before-INSERT schema1，实际 1/1（0.67 秒） |
| 扩大原财务/Source/Root 回归 | 112/137，1062.67 秒；完整 25 个失败保留，未宣称 137 全绿 |
| 新 born30 实际心跳 | 首 0/2（1.81 秒），原 deadline/短活后置/no-op 修复 2/2（5.03 秒） |
| 迁移实际失败族与新边界 | 35/41（276.76 秒）；其中五个新自然期限夹具误查 late facts，另一个仍期待较晚的 unknown 错误码 |
| 五个真正自然期限 + unknown 全行不变 | 正常原消耗账本/早拒精确码修正后 6/6（180.48 秒）；含六次真实 30 秒原 clock/C 自然到期，不 UPDATE 原起点或限额 |
| 心跳额外业务/foreign 写 | 第一 1/3 是诊断 foreign Root 的 scan FK 夹具错误，不能当 feature red；修正为真实 scan/attempt 下独立诊断 Root 后真 1/3 red，两个入口先观察 business 被接受，foreign 在 red 中尚未越过首断言；fix 后两者均实际验证，相关 7/7（21.54 秒） |
| 最后机械修正后相关原权限/创建 | 9/9（43.62 秒），含 Source fresh4、旧无Mode2、Source direct missing1、真自然到期1、共享 saved local replay1 |

五个旧 false-expired fixture 曾只修改 started_at='2000'，原 clock/C 未到期，不是自然 deadline 验收。现真实出生前冻结 quick30，实际 checkpoint 取得 1/3/4/6/7 请求后等原期限；全部应用表不变、已付事实和原 receipt 保留、不重发、不业务发布。候选/coverage 已付本地恢复明确更名为 ignores_mutable_started_at；原起点仍活时的 7/8 次费用保持，损坏账本/回执负向继续早拒。

## 最后门禁

- 严格 offline/locked、all-targets/all-features Clippy：前两处 canonical JSON cmp_owned 告警保留字节比较，移到命名 String；第二次 standalone importer 缺 nested module path，按既有模块加显式 path；第三次两个测试 helper 参数/tuple complexity，只合并配置参数及命名 tuple。最后退出 0（11.74 秒），没有删检查或加宽泛 allow。
- importer：39/39（2.59 秒），all-features standalone 也真实编译；精确退役字面量 allowlist：1/1（0.63 秒），没有更新/放宽登记。
- 十项结构化 UI 原相关合同 10/10（0.764621 秒）；当前实际 theme CSS 后 vue-tsc/Vite build exit 0（Vite 2.19 秒）。此前 792/792 默认 UI 属已封结构化 UI 批，先于此三个 CSS 变量修改，不能冒称最新又跑了 792。
- 13 个新增 Rust 叶文件 rustfmt --check 通过；53 路径 diff --check 通过。旧压缩文件保留格式债，只改必要钩子。

## UI 视觉补充

真实当前 RootDecisionSummary 使用 app-surface/app-ink/app-muted，深色卡片和背景有层次，明暗主题统一，窄窗单列。观察、缺口、下一步建议、风险、原账本用量分段；费用建议仍为建议，未添加假的思考、派发、结果或金额。

当前实际 SFC 的 before-dark、after-dark、after-light-narrow 图与 DOM 在 `/tmp/oviraptor-current-root-decision-visual/`，根代理已查看。窄窗实际 innerWidth/scrollWidth 均 592，card x32/right560，无横向溢出。最初白图是 /tmp 与 /private/tmp 的 Vite fixture allowlist 错误，修正后才构成视觉证据；430 像素截图受 headless viewport 限制发生裁剪，未当窄窗通过。隔离 headless 和 Vite 已关闭，资料保留。不是安装 WebKit、键盘/业务状态、真实 Root 付费聊天消息或 URL 验收。

## 未提交改动保护与指纹

53 代码路径：36 已有、17 新增，新文件最多 254 行。原基线是已封 Web 创建财务批的 988 路径 SHA87ec6e574923d64a7052c098953ea5cc02db3b9d08667afe688479de0c1b97ae。最终 1005 路径 SHAf26aed8438987957f6c5db162bf15f1f9f63bd2d35fdabfa84c905c6a6c7acaa，952 范围外原路径逐 SHA 不变；HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b。

collection 仅 src、src-tauri/src/resources、tools、package 文件、Cargo 文件、tauri.conf 与 backend retirement allowlist；不是 dist/icons/build.rs/capabilities/docs 或全仓库快照。每文件 before、原 dirty diff、最后增量 diff、scope-final 与 code-snapshot 在 `/tmp/oviraptor-source-original-finance-and-ui-*`。合并最早 before，未用后来镜像覆盖已有改动。十路径结构化 UI 已独立封存，当前 53 集合包含其前端变化但 Rust 验收与其视觉合同分列。

## 独立审查

并行框架审查完整逐读 53 份 delta（3729 行）并复核全部 after SHA，零漂移。核实 Source capture 在 INSERT 前、publish 在同 IMMEDIATE，私有 loan 与新行绑定，只有 born Web/Source token 进入非测试 initializer；旧 SDK missing-owner 负向和 lower prototype 的 cfg(test) 边界保留。无本批阻断，允许进入下一 g2 实测。自然到期时保存原 paid rows、不业务发布不等于金融 finally 或唯一 reducer 完成。

## 证据索引与机械失败

- born/source：`/tmp/oviraptor-source-born-finance-red.log`、`/tmp/oviraptor-source-born-finance-green-initial.log`。
- Core：`/tmp/oviraptor-original-root-only-red.log`、`-green-corrected.log`、`-affected.log`；最初 compile 的两个 path/shadowing 错误没有算绿。
- timer/direct：`/tmp/oviraptor-source-original-timer-red.log`、`/tmp/oviraptor-source-direct-original-owner-red.log`、`-green.log`。
- 137：`/tmp/oviraptor-source-original-finance-affected.log`、`-affected-failures.txt`。
- heartbeat：`/tmp/oviraptor-source-heartbeat-born-deadline-red.log`、`-green.log`；collateral `-red.log`（FK 夹具）、`-red-corrected-fixture.log`、`-green-affected.log`。
- 41/6/9：`/tmp/oviraptor-source-original-finance-targeted-final-corrected.log`、`-six-corrected.log`、`-last-nine.log`。第一 targeted-final.log 是 cargo CLI filters 位置错误，未编译或运行；诊断 assertion 机械 anchor 与未正确选择的 rg glob 失败保留，不是通过证据。
- 最终静态：`/tmp/oviraptor-source-original-finance-clippy-fixtures-corrected.log`、`-importer-final.log`、`-literal-final.log`、`-new-leaf-fmt.log`、`/tmp/oviraptor-root-decision-actual-theme-build.log`。

下一步直接合入 Root bootstrap 实际 SDK/原付费恢复，再变化事实的连续 Root tick、Rust utility 与真实调度；后续全部角色/Broker、Source 唯一终态 reducer/all-outcome 财务、Single finally/暂停恢复、用户有序逐角色执行和实际消息、全路 SDK/Web 日志仍未完成。所有 /tmp 草稿未 apply、未跑的 Rust 测试不算交付。最后才完整门禁、当前安装态和两个授权 URL。InputParser 先前自动审批拦截部分继续未完成，不重试绕过；Master/Goal 不标完成，Goal 工具仍 paused，按用户继续指令开发。
