# Oviraptor 加固会话 · 问题汇总与修复状态

> 本文件由本轮自主会话维护。记录：真实复现的失败、根因、已修 / 未修 / 不该改的判断依据，以及后续计划。
> 约束(全程遵守)：**绝不改真实库** `~/oviraptor/oviraptor.sqlite3`(只读 `?immutable=1` 诊断)；仅用合成/中性数据；全部本地；**不 push**。

最后更新：见 git 历史。版本号固定 `1.1.117`(未随构建递增,靠行为/发现区分新旧,不是版本问题)。

---

## 0. 背景

目标：扫描真实站点 `http://www.hjrich.cn/`(nginx/1.24.0 的纯静态生成企业站)。
扫描管线：协调器(coordinator，同质 root tick，v3 实际成本路径)→ 子任务(spa_api_mapper 映射 / external_surface 公开面 / deep_investigator / web_executor 深度 Web 调查)。
核心痛点(用户原话)：模型分支"费用未决"反复把扫描卡死；希望**智能化**而非写死规则,因为大应用耗时/用量更高。

---

## 1. 已修复(有真实失败数据佐证，均编译通过 + 测试全绿)

### 1.1 费用未决:逐维账本写死额度 → 据实弹性计费
- **症状**：`budget_indeterminate_requires_reconciliation`（"原 Root 模型费用未决"）。
- **根因**：子任务预算按**固定额度**预留(如 mapper 8000/类)。模型报告且自洽的实际用量只要**总额略超**该额度(实测 8045 vs 8000，仅超 45)，结算就把整笔预留**没收进 indeterminate**，随后阻塞所有后续预留 → 整条分支卡死。
- **修复**：`src-tauri/src/agent_runtime/multi_agent/budget/receipts/write.rs` — 报告且自洽的成本即判为 KNOWN，据实消费；额度只是**预留提示**，真正上限是 Root 硬顶(每类 90 万 token / 22 次请求)，由 `entries.rs::persist_inner` 在**每次账本写入**时强制(`budget_hard_limit_exceeded`)。
- **证据**：真实失败数据 SQL 复算通过。

### 1.2 费用结算:汇总账本同样的写死额度
- **症状**：`child_budget_settlement_exceeded_or_stale`（1.1 修复后暴露的下一层）。
- **根因**：系统有**两条并行账本**——逐维 `agent_budget_entries` 与**汇总** `agent_budget_ledger`。后者的结算 UPDATE 条件同样要求 `已花 ≤ 子任务预留`。实测 8690 > 8000 即失败。
- **修复**：`src-tauri/src/commands/multi_agent/child_accounting.rs` — 去掉汇总账本的**单任务**额度上限，保留**真正的** Root 总闸(`spent+reserved-?1+?3 <= total_tokens`)。
- **证据**：用真实失败 run 的账本行跑 SQL：旧条件匹配 0 行(=那次失败)，新条件匹配 1 行(结算通过)，花费超 90 万时仍 0 行(安全闸完好)。

### 1.3 单类别超额:弹性从 Root 预算补足
- **症状**：`external_surface` 子任务额度仅 2000，真实分析一个页面用了 5191 输入 token —— **单个 token 类别**就超额 → 仍没收 → 未决 → 卡死。
- **根因**：这是 1.1/1.2 之外的**真·单类别超额**。原三道守卫都"有硬顶就禁止在回执时扩预留"。
- **修复(3 处同模式守卫一并放开，仍由 Root 硬顶兜底)**：
  - `receipts/write.rs`：报告且自洽的单类别超额 → 从 Root 预算**补足预留**再消费；
  - `budget/entries.rs::persist_known_cost`：允许在有硬顶时扩预留(由 persist_inner 兜底)；
  - `budget.rs::validate_received_transition`：同上放开回执期 Reserve。
- **效果**：mapper 与 external_surface **均能完整跑完**(`child_completed`)。

### 1.4 公开面分析:模型结构化输出被误判非法(纯校验闸)
- **症状**：`public_surface_assessment_invalid`（偶发，取决于模型这次把 observations 写成字符串还是对象）。
- **根因**：校验器把 `observations`/`coverageGaps` 写死为 `Vec<String>`，但提示词没限定元素类型，模型合理地产出结构化对象 → 解析失败 → 整扫失败。
- **修复**：`src-tauri/src/commands/multi_agent_external_surface.rs` — 对象/字符串都收，只保留数量与大小上限。**纯校验闸**(解析结果不被下游消费)，故直接放宽安全。附 6 条回归测试(全过)。

### 1.5 协调器汇总帧:同类结构化输出(载荷性，改用强制转换)
- **症状**：`root_frame_mapper_schema_invalid`（1.3 修复后暴露；两个子任务已完成，协调器在汇总帧挂掉）。
- **根因**：`NativeCoordinatorMapperFact.priorityContracts/risks` 写死 `Vec<String>`，模型产出对象 → 失败。提示词只写 `[]`，未限定元素类型。
- **修复**：`src-tauri/src/commands/agent_native/coordinator_frame_rows.rs` — 对象项**转紧凑 JSON 文本**后存入，保持下游 `Vec<String>` 契约不变(与 1.4 不同，这里是**载荷性**，故用强制转换而非改类型)；元素长度上限放宽到 2048 以容纳结构化项。

### 1.6 Root tick 回执体积上限:大/长扫描余量
- **症状**：`root_tick_receipt_too_large`（历史单次；当前 web 扫描峰值 331KB，未触发）。
- **根因**：决策回执**内联全部花费/派发凭证**(每条预算分录)，随扫描时长线性增长；大站点可能超 1MiB。
- **修复**：`budget/root/model/tick/storage.rs` — 纯存储边界 1MiB → 8MiB(非模型上下文，不动财务逻辑)。*正解应是不再逐轮内联整份凭证，见 §4。*

### 1.7 UI:从扫描一键"交给 Agent 分析"
- **症状(L7，早先推迟)**：handoff 只挂在结果弹窗 `AgentDialog`；用户常看的运行状态/失败页 `NativeRunStatus` 没有入口。
- **修复**：`src/features/sentinel/components/SentinelBoard.vue` — 扫描操作头部新增「交给 Agent 分析」按钮(非草稿即显示)，复用现有 `analyze-scan` → `App.vue` → `AgentHostChat` 新建/打开该任务对话并跳转的只读链路。前端 `vue-tsc`+`vite build` 通过。

---

### 1.8 WAF 误判:正常登录页含"验证码"字样被当成人机挑战,整扫 abort
- **症状**:换 URL 到 `http://222.92.38.52:18091/WMS/login`(若依 RuoYi 保税仓 WMS 登录页)后 `失败`;run `7deb2beb`。检查点 `web: failed · 流水线没有有效完成目标：可重试无进展 1`。
- **排查**:非预算问题。recon 正常(HTTP 200),mapper 子任务**已 `child_completed`** 并产出完整契约报告;但协调器 `plan-*` 以 `confirmed_waf_or_challenge` / `public_surface_protected_target_waf` 终止 → 整扫 abort,把已完成成果一起丢。external_surface 抓取记录里该 200 页被打上 `"protected": true`,而 `securityRelevantHeaders: []`(无任何 WAF 头)。
- **根因**:`src-tauri/src/commands/scan_execution_metrics.rs::is_directory_block_signal` 把 `captcha`/`验证码`/`人机验证` 当强 WAF 信号——**只要页面正文出现就判拦截**。该登录页有 `placeholder="验证码"`、`captcha/captchaImage`、`captchaDiv`(一个带验证码输入框的普通登录表单)→ 误判 `protected=true`。同文件的兄弟函数 `hard_fuse_reason` 早已把强/弱信号分档,弱信号(验证码)仅在确属拦截响应时才算;`is_directory_block_signal` 一直缺这个纪律。
- **修复**:给 `is_directory_block_signal` 套上同样两档——强 WAF 标记(cloudflare/aws waf/js challenge/429/rate limit…)照旧拦;`captcha/验证码/人机验证/verify you are human` 仅在**确属拦截响应**(403/429、强标记,或非 200/404 的纯挑战页)时才算;正常 200 页面渲染出验证码表单/图片(`<img`/`src=`/`captchaimage`/`<form`/`<input`/`placeholder`)不再误判。只传裸响应体的调用方(无状态码上下文)行为不变,不放松真实检测。
- **证据/测试**:4 条单测(该登录页→不拦、cloudflare/429/waf→拦、裸挑战体→拦、404/普通页→不拦)全绿;Rust 全量测试通过。

---

### 1.9 优雅降级(A):只停该目标、绝不因单点拦截把整扫判死
- **动机**:用户核心诉求——"换个 URL 就失败"。根因不止"认错 WAF",更在于**单点判定拖垮全局**:一个目标被确认拦截(WAF/挑战)就把整扫判 `failed`,连已捕获的侦察/证据/已完成子任务一起当成死失败,不再交付。
- **查实的机制**:目标结果在 `agent_terminal_legacy_consumer.rs` 按 `AgentTargetOutcome` 归档——非拦截的"受限"(本地模型容量)**早已降级为 partial**;只有**确认拦截的 fuse**(`Limited` 且 `requires_fuse()`)计入 `tally.limited`。而 `agent_backend.rs::terminal_summary` 的最后 `else`:当没有任何目标"完成"时,`limited`-only 也被打成 `failed`。分支 `failed` → 整扫 `failed`(`native_scan_branches.rs`),交付侧按"blocked"而非"可复核结果"处理。
- **修复(结构性,保安全)**:`terminal_summary` 最后分支拆分——`failed==0 && limited>0`(只被可重试的拦截/限制打断、无硬失败)→ 报 **`partial`**("均停在拦截或资源限制,可重试;已捕获证据已保留"),而不是 `failed`。**fuse 本身不动**(仍然正确地停在真 WAF,不硬闯),只是不再让一道墙把整扫连同已完成成果一起判死;硬失败(persistence/integrity/模型不可用)仍 `failed`。
- **效果**:任何 URL 只要捕获了侦察/证据,就以"带缺口/部分完成"交付并可 `继续` 重试,而不是一场空的 `失败`。这就是"让它全自动跑到底"的那一层。
- **边界(未放宽)**:预算硬顶、请求重放/账本完整性、凭证/越权、真 WAF 不硬闯——这些硬门禁一条未动;降级只改"单点结果如何汇总成整扫终态",不改"该不该停该目标"。
- **测试**:`agent_backend.rs` 新增 `pipeline_tally_degradation_tests`(拦截-only→partial、硬失败→failed、拦截+硬失败→failed、completed/gaps 不变、completed+拦截→partial)。

### 1.10 web_executor 预算无窗口 → 不再硬失败整扫(优雅降级,保安全)
- **症状**:换 URL 到 `http://222.92.38.52:18091/pc/login`,run `07ba08a3`,`web: failed · ...失败 1...multi_agent_bootstrap_failed:budget_hard_limit_exceeded`。**这次是硬失败**(failed=1),§1.9 的降级正确地没掩盖它。
- **查实**:mapper **child_completed**、external_surface **child_completed**(且这次没再误判 WAF——§1.8 生效);只有 **web_executor** 在 bootstrap 预留阶段 `budget_hard_limit_exceeded`,传导成协调器 `tool_failure`,整扫 failed。
- **根因(有运行时数据)**:聚合账本显示**只花了 12,796 / 900,000 token、2 / 22 请求,reserved=0**——预算 98% 空闲,却报硬顶超限。真因是 `budget/root/shared.rs::available()` 的 `occupied = 子预留 + root.consumed + max(held_input,held_output)`:当协调器某次 tick 还**持有一大笔临时预留(held)** 时去给 web_executor 定额,`occupied` 瞬时 > 900k,`available()` 直接返回 `budget_hard_limit_exceeded`。这是 §2.1 记的**预留时序/双账本口径**老问题,首次拿到确凿数值(spent 极低却撞顶=held 瞬时占满)。
- **修复(优雅降级,不动预算完整性)**:`coordinator_executor_allocation.rs` 给 web_executor 定额时,把 `remaining_child_capacity` 返回的 `budget_hard_limit_exceeded`(读时的"此刻没有预算窗口")翻译为既有的 `multi_agent_executor_budget_unavailable`——与"容量不足"同路,走 §2.1 的降级:web 分支**延后**、map/surface 成果保留、整扫经 §1.9 变 `partial`。**不改任何预留、不改硬顶**,只改"读时容量不足如何归类"。真正让 web 调查**跑起来**的时序正解仍是 §2.1/§4 的预算完整性工作(安全边界相邻,需谨慎,非盲改)。
- **效果**:该 URL 不再整扫失败,交付 coverage/API/指纹结果;web 深度调查暂以"预算延后"收尾。

---

## 2. 当前拦路点(本轮处理中)

### 2.1 web_executor 预留失败 fails 整扫 — **应优雅降级**
- **症状**：`child_budget_reservation_exceeded_or_stale`。**两个主分支(mapper/external_surface)已成功**，但第三个分支 web_executor(深度 Web 调查)预留失败，导致**整扫 `tool_failure`**，把已完成的成果全丢。
- **已排除**：不是租约/fencing 过期(epoch/token 都一致)；不是预算耗尽(消耗 ~16k–104k / 90 万，请求 2–8 / 22)。
- **根因(诊断)**：web_executor 额度在 `prepare.rs:315` 按 `remaining_child_capacity()`(逐维账本、计入 root 消耗)算得，但实际预留走 `reserve_child_budget` 的**汇总账本** UPDATE(只计子任务消耗)——两条账本口径不同；叠加 proposals 子任务并行预留与中途可能的 root tick 消费，使按旧口径算出的额度在真正预留时不再满足。属**预留时序/双账本一致性**的细微老 bug，因前述修复让扫描首次推进到 web_executor 而**首次暴露**。
- **本轮处置**：
  1. **优雅降级(立即)**：web_executor 预留失败不再 fails 整扫，而是以**不完整/有缺口**收尾，保留已完成分支的成果(mapper/external_surface/暴露探测)。这是产品上正确的行为——2/3 分支成功不该因第三分支预算打嗝而全盘失败。
  2. **时序/双账本正解(较深，谨慎)**：在实际预留点按**当前**剩余容量重新夹取 web_executor 额度，或统一两条账本口径。见 §4，需仔细、带验证。

---

## 3. 查实"不该改 / 当前未启用"的(有依据，未动)

- **指纹报 Nginx 而非 DedeCMS**：**非 bug**。`www.hjrich.cn` 是 nginx 后的**纯静态生成站**(`/html/html/.../*.html`)，运行时 DedeCMS 痕迹全无(`/plus/`、`/dede/`、`/templets/`、`/data/admin/ver.txt` 全 404，正文无 `dedecms`)。活体指纹报 Nginx 正确。坐实需从已抓到的 `.git` 泄漏做**代码深挖**(见 §4)。
- `root_local_tool_id_invalid`：正确的完整性守卫(格式 + 去重/防重放)，且网关上游已自动净化工具 id。放宽=削弱重放安全。**不动**。
- `mailbox_saved_result_mismatch`：正确的恢复完整性守卫(`_=>false` 兜底拒绝非预期 (角色,类型))。无当时失败载荷,凭空加分支=猜。**不动**。
- `business_logic` / `source_*` 校验器：**载荷性**(解析结果被下游消费) + **带版本号的契约**(模型被明确要求遵守该 schema) + **当前未派发**(web 路径不走它，dispatch 是死代码)。应在真正启用、能观察真实输出时按证据修(public_surface 就是这么修成的)。**不动**。

---

## 4. 待办 / 后续(较大，需谨慎或设计)

- **web_executor 预留时序正解**(§2.1 第 2 点)：统一逐维账本与汇总账本的"已用"口径，或在实际预留点按当前剩余重新夹取额度，使 web 调查在预算允许时真正运行。
- **自适应上游预留**：子任务预留额度随应用大小/上下文体积自适应(目前 mapper 固定 8000 等)，但该额度是**哈希校验的冻结合约**(root 决策 + 合约)，改动面大、需成套更新与测试。
- **Root tick 回执瘦身**：不再逐轮内联整份花费/派发凭证(§1.6 的正解)。
- **`.git` / OSINT 代码深挖**：从泄漏的 `.git` 仓库反查框架/源码(坐实 DedeCMS 这类静态站)，靠纯指纹做不到。
- **Agent 能力增强(向 opencode 靠拢)**：见 §5。

---

## 5. Agent 能力增强计划(向 opencode 对齐)

> 目标：让内置 Agent 具备更强的自主工程能力，向 opencode/claude-code 这类终端编码 Agent 靠拢。
> **重要定性(自主会话判断)**：`packages/agent-host` 已经是一套**成熟**系统，不是"缺基础能力"。盲目大改它的核心(尤其是刻意设计的只读/consent/有界委派的安全模型)在你不在时**风险过高**，所以本节是**清醒的差距分析 + 安全可增量的计划**，供你回来定向推进，而非这一轮就把 Agent 改成 opencode。

### 5.1 现状(已具备，接近编码 Agent 架构)
- 工作区工具：`workspace_list / workspace_read / workspace_write / workspace_run`（读/写文件、跑命令）**＋本轮新增** `workspace_edit`（精确补丁）/`workspace_grep`（正则检索）/`workspace_glob`（通配定位），见 §5.4。
- 子 Agent 委派：`host_task_delegate` / `host_task_await`（**只读** research/review，最多 2 并发 / 每轮 8 个；不可写、不可递归、不可新 consent）。
- 任务校验：`task-verifier.mjs`（criteria：file_exists / file_contains / tool_json_equals / tool_read_equals / execution_success，要求真实当轮回执）。
- 记忆 `memory.mjs`、技能 `skills.mjs` / `paired-skills.mjs`、MCP 桥 `mcp-bridge.mjs`、provider 循环 `provider.mjs`（并行工具调用、responses/chat 两种协议）、consent/access-policy 权限模型。

### 5.2 与 opencode 的主要差距(需产品定向 + 测试，不宜盲改)
1. **写/跑的授权粒度**：`workspace_write/run` 受 consent/scope 约束(本应用是安全工具，默认保守)。opencode 在项目内自由读写跑。→ 建议作为**显式"工程模式"**(用户明确授权的项目范围)开放，而非放宽扫描器的安全默认。
2. **子 Agent 仅只读**：无写委派。opencode 的子 Agent 可执行更多。→ 增加**可写子 Agent**(仅在工程模式、带 consent、有界)。
3. **精确编辑**：目前是 `workspace_write`(整文件)。opencode 有 old_str/new_str 精确补丁。→ 加 `workspace_edit`(精确替换，带唯一性校验)。
4. **结构化检索/导航**：缺 grep/glob/符号检索/LSP 诊断。→ 加 `workspace_grep`、`workspace_glob`，可选 LSP/诊断桥。
5. **循环自主性/自愈**：有 task-verifier，但迭代深度与"失败→自动修正"回路可加强。→ 在工程模式下引入"计划→执行→校验→重试"的有界自治循环。
6. **大仓上下文**：缺代码库索引/导航以在大项目中定位。→ 轻量符号/文件索引。

### 5.3 建议的安全增量路线(优先级)
- **P0｜工程模式开关**：✅ **查实已基本具备**。`access-policy.mjs::ProjectAccessPolicy` 的 "full" 模式(operator 明确确认、按项目、带 runtime-identity 绑定与 revision)就是工程模式闸：只有它授予 `workspace.write`/`workspace.run`；扫描路径默认 "ask"=只读。write/run 授权与安全扫描路径**已隔离**。本轮沿用该闸,不新开削弱安全默认的旁路。
- **P1｜`workspace_edit` 精确补丁** + **`workspace_grep`/`workspace_glob`**：✅ **本轮已实现**(见 §5.4)。
- **P2｜有界自治循环**(plan→act→verify→retry) + **可写子 Agent**(工程模式内)：未做。已有 `task-verifier.mjs` 可作基座,但"失败→自动修正"回路的深度与可写委派需产品定向。
- **P3｜代码库索引/LSP 诊断桥**：未做。

### 5.4 本轮已落地(P1,带测试)
> 原则:只加能力,不动既有安全模型。三个新工具完全**复用**现有 consent/权限/路径封闭/版本记录/写锁,靠 `workspacePermission()` 映射继承 full-mode / per-conversation grant / 交互授权的同一套闸。

- **`workspace_edit`**(→`workspace.write`):精确 old/new 替换。`oldString` 必须唯一(除非 `replaceAll`);`no_match`/`not_unique`/`no_change`/`invalid` 分别报错;以 slice/split+join **字面**拼接(newString 里的 `$&`/`$1` 不被当作替换模式);复用与 `workspace_write` 同一条原子写(staging+O_EXCL+O_NOFOLLOW+rename、prior/new 版本记录),并入每根目录的写锁串行化。
- **`workspace_grep`**(→`workspace.read`):正则检索,`literal`/`caseInsensitive`/`include`(通配过滤路径)/`maxResults`。跳过符号链接、硬链接、二进制(含 NUL);文件 5000 上限、单文件 1MiB、单行测试截断 2000 以防病态正则跑飞;非法正则报 `workspace_grep_pattern_invalid` 而非崩溃。
- **`workspace_glob`**(→`workspace.read`):锚定 glob(`**` 跨目录、`*`/`?` 段内、其余字面转义),按搜索基目录匹配、返回工作区相对路径、排序去界。
- **集成点**(全部一致更新):`service.mjs` 只读分类加 grep/glob;`provider.mjs` REFERENCE_READ_TOOLS 加 grep/glob;`chat-files.mjs` 文件外显 IN 加 workspace_edit;`quality-prompt.mjs` 系统提示词 bullet14 + UI 状态标签;`subagents.mjs` **只读** research 子 Agent 增配 grep/glob(**不**给 edit,子 Agent 保持只读)。
- **测试**:`packages/agent-host/test/workspace-tools.test.mjs`(9 条)——glob 语义、edit 唯一/全替/无匹配/不唯一/无变更/字面 `$`、grep 正则/字面/忽略大小写/include、非法正则、以及三工具的路径封闭(`../` 拒绝)。全绿;agent-host 全量 22 测试通过。

> 下一步若继续:P2 的有界自治循环可在 `task-verifier.mjs` 基座上,以工程模式(full)为前提逐步加"计划→执行→校验→重试",可写子 Agent 同样仅限 full 模式、带 consent、有界并发——仍不触碰扫描器只读默认。

---

## 6. 本会话改动文件清单(均未提交，都在 `main`)

- `src-tauri/src/agent_runtime/multi_agent/budget/receipts/write.rs`
- `src-tauri/src/agent_runtime/multi_agent/budget/receipts/owner.rs`
- `src-tauri/src/agent_runtime/multi_agent/budget/entries.rs`
- `src-tauri/src/agent_runtime/multi_agent/budget.rs`
- `src-tauri/src/commands/multi_agent/child_accounting.rs`
- `src-tauri/src/commands/multi_agent_external_surface.rs`(+6 测试)
- `src-tauri/src/commands/agent_native/coordinator_frame_rows.rs`（§1.5 结构化输出强制转换；§1.11 根帧映射抢救式降级 +11 测试）
- `src-tauri/src/commands/scan_control.rs`（§3.1 新增只读命令 `list_sentinel_vulnerability_counts`）
- `src-tauri/src/lib.rs`（注册 `list_sentinel_vulnerability_counts`）
- `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/storage.rs`（§1.6 回执余量）
- `src-tauri/src/commands/multi_agent_external_surface.rs`（§2.1 web_executor 预留失败优雅降级）
- `src/features/sentinel/components/SentinelBoard.vue`（§1.7 交给 Agent 分析）
- **字体一致性**(全部 CSS)：`src/shared/styles/foundation.css`(新增 `--font-sans`/`--font-mono` 令牌)、`src/shell/styles/desktop-theme.css`(统一 :root 基准字体)，以及把**未加载的** `IBM Plex Mono`/`Manrope`(无 @font-face、无字体文件、无 googleapis，全在回退)统一替换为 `var(--font-mono)`/`var(--font-sans)` —— 覆盖 `chrome.css`、`project-profile-cards.css`、`security-rules.css`、`evidence.css`、`findings.css`、`workspace.css`、`browser-auth.css`、`nativeRunStatus.css`、`sentinel-*.css`、`asset-*.css`、`theme-enhancements.css` 等及若干 `.vue` 作用域样式。效果：此前 `font:9px IBM Plex Mono;`(无通用族→回退衬线)这类不一致全部变为一致的系统等宽/无衬线，离线可用。
- `docs/SESSION_HARDENING_STATUS.md`（本文件）

### 续：2026-10-30 下午（结果页拆分 + 后端)
- **§1.11 根帧映射抢救式降级**:`native_coordinator_mapper_semantic` 原来用 `deny_unknown_fields` + 严格 `Vec<String>`,模型 JSON 形状稍不对(多字段/缺字段/非串项/超长/>16 项)就在 bootstrap 处以 `root_frame_mapper_schema_invalid` 把**整单**判死(0 token)——失败任务 `9b634d9f`(`http://222.92.38.52:18091/pc/login`)即此根因。改为:泛型解析 → 非对象才真失败;summary 清洗+截断(空给占位),列表项任意类型强转为文本(对象/数组→紧凑 JSON、标量→字符串、null/空丢弃),去控制符、折叠空白、单项截断 2048、列表封顶 16。该事实为**咨询性**,不入预算回执/物理事实哈希(另走 `closed_rows`),放宽零完整性影响。`mapper_salvage_tests` 11 条全绿。
- **§3.1 只读计数命令**:`list_sentinel_vulnerability_counts(projectId?) -> Vec<{scanId,count}>`,复用与 `list_sentinel_vulnerability_scan_ids` **完全相同**的可见性谓词(policy 要 proof 时才按 `proof_finding_is_visible` 过滤),仅计数不去重;`sentinel_scan_contexts.scan_id` 为主键故 LEFT JOIN 不会笛卡尔放大;count=0 的 scan 不返回。配合结果页拆分里「任务列表头号数字=漏洞数」。
- **结果页拆分(UI)**:交给 Opus 5.5 会话落地(拆「结果与证据」为任务列表+任务详情、过程并入执行概览、总览加任务条、导航「更多」平铺)。进行中,共用同一 checkout。

### 安装状态
- `/Applications/Oviraptor.app` 为上一轮版本(含:全部预算修复 + frame_mapper + 回执余量 + web_executor 优雅降级 + 字体一致性 + 「交给 Agent 分析」按钮)。
- **本轮(§1.11 降级 + §3.1 命令 + UI 拆分)尚未重新构建安装**——待 UI 完成后一次性 release 构建 + `ditto` 到 /Applications 统一验证。
- Rust 相关测试通过(`mapper_salvage_tests` 11/11,`cargo test --lib` 过);UI 侧由 Opus 会话跑 `npm run build`。

### Agent 能力(§5)
- **P1 已落地**(用户回来后定向"改一下 agent"):新增 `workspace_edit`/`workspace_grep`/`workspace_glob` 三个工具,完全复用既有 consent/权限/路径封闭/版本/写锁,**不**改动安全模型;只读 research 子 Agent 增配 grep/glob(不给 edit)。详见 §5.4。
- 新增/改动文件:`packages/agent-host/src/workspace-tools.mjs`(核心)、`service.mjs`、`provider.mjs`、`chat-files.mjs`、`quality-prompt.mjs`、`subagents.mjs`,新增测试 `packages/agent-host/test/workspace-tools.test.mjs`。
- P0(工程模式)查实已由 `access-policy.mjs` 的 full 模式承担;P2/P3 待产品定向(见 §5.3)。
- agent-host 打包经 `desktop-distribution.json` 摘要校验,`beforeBuildCommand=npm run build:desktop` 会随构建重算摘要,故经正常 `tauri build` 打包即一致,无需手改安装包。
