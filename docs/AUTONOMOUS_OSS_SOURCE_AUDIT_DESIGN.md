# 自动开源源码审计 + URL 复现 — 设计草案

> 目标:URL 扫描识别出目标在跑某个开源系统后,**自动搜集该开源项目源码 → 源码级分析挖洞 → 在授权目标 URL 上复现**,全程不越授权边界。
> 状态:设计讨论,未实现。先定边界再落地。

## 0. 结论先行

- **「分析」这步**:**可以**在沙箱里跑,且有**两档**可选(见 §3.5):
  - **容器档(现有)**:SAST 分析器已容器化重度隔离(`--network none --read-only --cap-drop=ALL`,源码 `/src:ro`,`analyzer.rs:278`),含内存硬上限。
  - **轻量档(用户倾向,macOS Seatbelt)**:`sandbox-exec` 锁网络+文件系统,零镜像零 Docker,仓库已有探针雏形 `tool_supply/sandbox/macos_static_parser.rs`。代价:**无内存硬上限**(用 rlimit+超时+大小封顶近似),且**只能跑非执行型分析器(Semgrep,不能 CodeQL 编译型)**。
- **「搜集/下载」这步**:**不能**在那个沙箱里(它断网)。必须在沙箱外单独做一个受控 fetch 阶段 —— 这是唯一真正的新增 + 敏感部分,也是本设计的重点。
- **「复现」这步**:已有。`greybox` 把源码主张和运行时事实关联(support / contradict,见 `src-tauri/src/native_pipeline/greybox.rs`)。

## 1. 现状拼图(已有的积木)

| 能力 | 现状 | 文件 |
|---|---|---|
| 识别开源产品 | ✅ 指纹库(699+ 条),记为 info finding | `scanning/exposure_probe.rs:919` |
| 版本探测 | ⚠️ 部分指纹读版本文件(如 dedecms ver.txt),非普遍 | `resources/rules/fingerprints.json` |
| 源码分析(SAST) | ✅ 容器化、断网、只读挂载 | `native_pipeline/analyzer.rs:278` |
| 源码↔运行时关联 | ✅ greybox 图(support/contradict) | `native_pipeline/greybox.rs` |
| greybox 入口 | ✅ 需同时给 source_path + urls | `commands/code_analysis.rs:68` |
| **产品→仓库→版本 解析** | ❌ 不存在 | 新增 |
| **自动联网拉源码** | ❌ 故意不做(Nest 只对授权目标发请求) | 新增 |
| **识别→拉源→审计→复现 自动编排** | ❌ 不存在,无 web→source 自动桥接 | 新增 |

## 2. 目标流水线

```
[1 web 扫描]      指纹识别 → product = "ruoyi", versionHint = "4.7.x"(若可得)
      │
[2 解析 resolve]  product → 仓库URL + 版本/tag(查本地 curated 映射表)
      │           ← 无映射 → 停在"已识别未审计",正常产出,不报错
[3 搜集 fetch]    ★沙箱外·联网·最敏感★ allowlist 主机 → 浅 clone 指定 tag → 隔离目录
      │           ← 需用户授权(下载=审批动作)、大小/时间封顶、校验来源
[4 分析 analyze]  ★现有沙箱★ 隔离源码只读挂进 --network none 容器,SAST + agent 推理
      │
[5 复现 greybox]  源码主张 → 对授权 URL 打请求验证 → support/contradict 图
      │
[6 产出]          已复现漏洞 / 仅源码主张未复现 / 魔改矛盾点
```

## 3. 关键设计:三个新组件

### 3.1 产品→仓库→版本 解析表(curated,像指纹库)
- 新增 `resources/rules/source_sources.json`(或用户文件 `~/oviraptor/source_sources.user.json`,复用指纹库那套「用户可自行添加 + 校验」的机制)。
- 每条:`{ productId, repoUrl, host, versionStrategy, pathHint }`。只给**少数关心的产品**起步(ruoyi / dedecms / …),绝不自动给 699 条配仓库。
- `productId` 必须能对上指纹库的 fingerprint id,保证识别→解析链路闭合。

### 3.2 受控 fetch 阶段(沙箱外,唯一联网,最高风险)
硬约束:
- **主机 allowlist**:只允许从映射表里登记过的、且在全局白名单(如 github.com / gitee.com)的主机拉。agent **不能**自由决定去哪拉。
- **用户授权**:下载是需审批动作;首次对某产品/某仓库拉源,必须用户确认(可记住该产品)。
- **隔离落盘**:拉进一个**全新空目录**(quarantine),视为不可信;绝不在该目录里跑解释器/构建(遵守「下载内容即不可信」原则)。
- **封顶**:clone 深度=1、指定 tag、仓库大小上限、超时、磁盘配额。
- **可复现**:记录 repoUrl@commit,审计留痕。
- **失败降级**:拉不到就停在"已识别未审计",不影响 web 扫描本体。

### 3.3 自动编排(web→source→greybox 桥接)
- web 扫描收口时,若有已识别产品 + 映射命中 + 用户已授权 → 触发一个 greybox attempt,source=隔离目录,url=原授权目标。
- 复用现有 greybox 流水线,**不新造分析/复现逻辑**。
- 预算:greybox 两分支更烧,沿用 maxBudgetUsd 封顶;fetch+analyze 的额度要并入总预算上限,撞顶走现有 partial/可继续。

## 3.5 分析沙箱:轻量级(macOS Seatbelt)vs 容器 — 两档

用户倾向**轻量级、不要 Docker**。macOS 原生 Seatbelt(`sandbox-exec`)可做到,且仓库已有探针雏形 `src-tauri/src/tool_supply/sandbox/macos_static_parser.rs`。

| 维度 | Seatbelt 轻量 | Docker 容器(现有 analyzer.rs:278) |
|---|---|---|
| 网络隔离 | ✅ profile deny network | ✅ `--network none` |
| 文件系统 | ✅ 源码只读、只许写 scratch | ✅ `/src:ro` + `/out` |
| 提权/能力 | ✅ Seatbelt 限制 | ✅ cap-drop + no-new-privileges |
| **内存硬上限** | ❌ 无(仅 rlimit 近似) | ✅ cgroup |
| 依赖 | 系统自带,零镜像 | 需 Docker + 镜像 |

**关键前提(两档都适用,但轻量档更硬性)**:分析器必须**纯静态、不执行目标代码**。
- **Semgrep**:✅ 只解析,从不跑被测代码 → 适合轻量档。
- **CodeQL `database create`(编译型语言)**:❌ 会真的跑 build = 执行不可信代码 → **禁止进轻量档**,只能容器档或对下载源码直接禁用。

**轻量档落地要点**:
1. 补完 `macos_static_parser.rs`:从"探针"变成真正的 runner —— `sandbox-exec -f <profile.sb> <semgrep argv>`。
2. 写 Seatbelt profile:`(deny default)` 起步,仅放行 读源码目录、写 scratch、执行 semgrep 自身依赖;`(deny network*)`。
3. 补偿内存短板:`RLIMIT_AS` + `RLIMIT_CPU` + 墙钟超时 + **输入仓库大小封顶**。如实标注这不是"已证明的硬内存边界"(沿用现有代码 `macos_hard_memory_containment_unverified` 的诚实口径)。
4. 分析器白名单:轻量档**只允许**登记过的非执行型引擎(当前= Semgrep)。
5. Docker 仍保留为"强模式"(要 cgroup 硬边界 / 要 CodeQL 编译型时)。

> 现状提醒:`require_supported()` 目前**永远返回 Err**(故意禁用),因为硬内存边界未验证。启用轻量档 = 把它改成"FS+网络已隔离、资源用 rlimit 近似"的有条件放行,并在产出里标注隔离等级。

### 3.5.1 实现状态(本次会话)
**已做(验证原语,未接生产)**:`macos_static_parser.rs` 从探针补成真 runner —— `run_sandboxed()` + Seatbelt profile(allow-read / deny-network / write-only-scratch)+ rlimit(CPU 硬设、AS 尽力,macOS 对 AS 返回 EINVAL 已容错)+ 墙钟超时 + 源码大小封顶 + 路径/符号链接防逃逸。
**实测通过**:
- 单元测试 `seatbelt_isolation_tests`(2 条,macOS 门控):读源码可、写源码目录被拒(源码只读)、只有 scratch 可写、超大 checkout 启动前拦截。
- 手动网络验证:基线 `nc` 连 1.1.1.1:80 成功(exit 0);加 `(deny network*)` 后被拒(exit 1)。
**刻意没做**:未改 `require_supported`(现有调用点语义不变)、未翻转 analyzer.rs 的生产默认(host-binary 仍标 `network_disabled=false`);把 runner 接进分析器管线并翻转隔离标记,是下一步**评审后**才做的改动。
**没做(需你先拍板)**:§3.2 联网 fetch、§3.3 自动编排、§6 待定项。

### 3.5.1b 激活状态(后续会话 — 用户授权「拉源 + 自动编排 + 门禁放行 都做」)
三件均已接上并通过编译/测试:
- **拉源闸门(UI + 后端共识)**:每个扫描一行显式授权 `scan_source_fetch_consent(scan_id,allowed)`(默认拒),由任务创建页新勾选框「允许拉取开源上游源码」在建草稿后调 `authorize_scan_source_fetch` 写入;不勾选 → 无行 → 后端一切拉源拒绝。命令 `authorize/revoke/scan_source_fetch_allowed` 已注册。模型放弃「policy_json 夹带布尔」,改用独立可审计共识行(与本仓既有授权/财务共识一致)。
- **web→source→greybox 编排**:`source_orchestration.rs` 的 `orchestrate_source_audit(scan_id)`——先验共识(无则拒并记 tier3 决策),查指纹识别产品(`sentinel_findings` fingerprint/tech_identified)→ 匹配 `source_sources.json` 上游 → 受控 fetch 入隔离区 → 写 `source_audit/source_collected` 交接 finding(source_path + 授权 URL)+ 记阶段决策。**每次只拉一个产品**(网络动作可预期、可复核),幂等(已拉不重拉)。`source_audit_report` 只读。前端 `SourceAuditPanel.vue` 挂在 web 扫描执行概览:扫描收口后(有共识+有匹配+未拉)**自动跑一次**编排,并对每份已收集源码给「一键创建灰盒复现」。
- **花费边界**:灰盒复现会花模型额度,故**不自动启动**——保持一次点击,仍走既有预算/财务门禁(与 `automaticResumeAllowed:false`、财务续跑需批一致)。编排自动到此为止,不越过花费线。
- **门禁放行(§9.0 tier-1)**:`reducer.rs` 唯一的 `configuration_error → Failed` 出口加一道——brittle 非完整性信号(`*_schema_invalid`)改判 `Incomplete`(可恢复)+ 新码 `brittle_stage_absorbed`,不再整扫描硬失败;**完整性错误永不吸收**(仍 `Failed`/`EVIDENCE_INTEGRITY`),tier2 预算、tier3 安全各走原字段不受影响。3 条 reducer 测试覆盖三分支。裁决器时间线继续独立记录 tier-1 判定。

## 8. 离线会话状态:已实现 + 回来怎么测(重点看这节)

> 原则:只做"安全、additive、不弄坏现有可编译运行的 app"的;最敏感的联网拉源仍只在传了 opt-in 时才会触发,我**没有**替你无人复核地跑过任何真实 clone。**全部未提交**,留在工作树。

### 8.1 已实现 + 已验证
| 件 | 状态 | 位置 / 验证 |
|---|---|---|
| 轻量 Seatbelt 沙箱原语 | ✅ 实现+测试 | `tool_supply/sandbox/macos_static_parser.rs`;2 测试过(源码只读/scratch可写/超大拦截)+ 手动网络 deny 实测 |
| 受控 fetch 模块(gate/白名单/防注入/clone/隔离) | ✅ 已存在+测试 | `scanning/source_collection.rs`;8 测试过;催化表 `resources/rules/source_sources.json`(含 ruoyi) |
| **gated 命令 `scan_collect_source`** | ✅ 新增+注册+编译过 | 没传 opt_in 即拒;传了按白名单 clone 到 `~/oviraptor/source-quarantine/` |
| **gated 命令 `scan_sandbox_analyze`** | ✅ 新增+注册+编译过 | 把指定目录丢进 Seatbelt 跑静态分析器,返回隔离标志 |
| 工具"最小化 vs 完整"决策 | ✅ 分析完 | §7 |
| 沙箱模块对 scanning 可见 | ✅ | `tool_supply.rs` 改 `pub(crate) mod macos_static_parser` |

全套 `cargo check --lib` 通过,相关单测全绿。

### 8.2 回来怎么测(invoke 命令)
1. **验证门禁拒绝**(应失败):`invoke("scan_collect_source",{optIn:false,productId:"ruoyi"})` → 期望 `source_fetch_not_authorized`。
2. **验证受控拉源**(会真联网 clone 若依到隔离目录):`invoke("scan_collect_source",{optIn:true,productId:"ruoyi"})` → 返回 `quarantinePath`。只在你亲自点、确认联网时做。
3. **验证沙箱分析**(需本机有 semgrep):`invoke("scan_sandbox_analyze",{program:"<semgrep路径>",args:["scan","--config","auto","--sarif",".",...],sourceDir:"<quarantinePath>"})` → 返回 `networkDisabled:true, sourceReadOnly:true` + 输出。先拿任意小目录试隔离标志。

### 8.3 还没做(需要你在场/拍板,别无人复核上)
- **UI"拉源"勾选框**:前端 new-scan 表单加一个 checkbox,勾了才把 `optIn:true` 传给 `scan_collect_source`。这是产品级门禁的最后一环(后端已强制 gate,少的是 UI 开关)。属 ui-overview 会话的前端范围。
- **web→source→greybox 自动编排**:web 识别产品 → 自动拉源 → 自动起 greybox。这步会自动花预算+链式执行,要你在场看预算/流程,我不无人复核上线。接线点:web 扫描收口处读 `sourceFetch.enabled` + 已识别产品 + 催化命中 → 调上面两命令 → greybox。
- **把沙箱接进 analyzer.rs 生产默认**:现在 host-binary 分析仍标 `network_disabled=false`(视为 gap);翻转成"非执行型引擎走 Seatbelt 并标隔离真"要评审,别动共享 run 路径的默认。
- **催化表扩产品**:现仅 ruoyi(URL 已核)。加 dedecms 等要**核准确的上游仓库地址**,错的地址比没有更糟,不瞎填。
- **代码/灰盒规则扩充**、**"丰富 nest 任务"、"agent 接入每个功能"**:这几项你给的是方向不是具体点。回来各举 2-3 个"现在缺什么/哪不好用"的实例,我对症做。

## 7. 决策:工具"最小化 vs 完整版" + 按测试项目怎么调用(你让我分析的)

先区分**两种沙箱里跑的是两类工具**,结论不同:

### 7.1 分析类工具(SAST:Semgrep / CodeQL —— 在沙箱里读不可信源码)
- **结论:最小化、每引擎独立、digest 固定。** 现有 `analyzer.rs` 已经就是这么做的(每引擎一个 pinned 镜像),**保持**。
- 为什么不是"一个大全家桶":跑的是**不可信代码的分析**,最小权限最重要——每个引擎只挂它要的源码/规则/输出,镜像用 sha256 钉死保证可复现,冷启动快,还能按测试项目单独开关。全家桶=攻击面大、不可复现、难按需裁剪。
- 轻量档(Seatbelt)同理:只允许**非执行型**引擎(Semgrep),每个独立调用,不塞一堆。

### 7.2 Web 扫描探测工具(对**授权目标**打请求的那些 —— 浏览器/HTTP 原语、特定探针)
- **结论:完整安装、按测试项目"激活最小集"。** 不是给每种构造一个"最小化构建",而是:**二进制完整装一份,运行时按测试项目声明的能力清单只放行需要的那几个**。
- 为什么不是"每种最小化构建":你会陷入维护 N 个裁剪版的泥潭,且扫描是打**授权目标**(不是跑不可信代码),威胁模型没分析沙箱那么极端。真正要的是**调用期最小权限**,不是构建期最小体积。
- 落地形:每个测试项目(test-type)声明 `requiredTools: [...]`;Nest 执行器据此只允许调用清单内工具,清单外一律拒。工具本体仍是完整版,放行面是最小的。

### 7.3 一句话
- **分析不可信源码** → 最小化 + digest 固定(安全优先,已是现状,保持)。
- **探测授权目标** → 完整装 + 按测试项目声明激活最小集(可维护 + 调用期最小权限)。
- 两者都做到"每次只暴露该测试项目需要的工具",但一个靠**最小构建**、一个靠**最小放行**。

> 这条 §7.3 是你那个"好难抉择"的答案:别纠结所有工具都做最小化构建——只有进分析沙箱、碰不可信代码的那类才值得最小化构建;打授权目标的那类,装全、按需放行即可。

## 9. 设计:Agent 接管阶段编排("别再因门禁硬失败")

> 目标(你出门前定的 goal):每个阶段交给 agent 分析并给下一步;按 skills 跑完流程后**先别停**,让 agent 分析哪里不足;把 agent 的介入过程显示在「执行概览」的目标进度里;任务若在 agent 分析后仍停,用户能**对话续跑**并把结果**汇回同一任务**。

### 9.0 核心重构:门禁从"静默杀任务"→"agent 裁决"
把门禁按类分三档,只有第 3 档会停下来等人:
| 档 | 例子 | 新行为 |
|---|---|---|
| 1 误报/脆门禁 | WAF 误判、schema 形状、根帧映射 | agent 自动吸收 → 继续/重试调整,**绝不硬失败**(部分已在 §1.x 做) |
| 2 资源/预算 | 模型调用上限、$ 预算、无进展 | agent 降级为 partial 或给出"要不要加预算"的下一步,不杀任务 |
| 3 真·安全 | 花钱授权、不可逆写、联网外发(含拉源) | **永远升级给用户决策**,agent 不能自动放行 |

"取消门禁"= 把 1、2 档交给 agent,不是删掉 3 档。这样任务不再莫名失败,而真安全边界仍在。

### 9.1 阶段裁决器(stage adjudicator)
- 每个管线阶段产出**结构化结果**:`ok | degraded | blocked | gap`(不再只有 成功/失败)。
- 裁决器把 {阶段, 结果, 上下文, 覆盖账本} 交给 agent(有界调用)→ agent 返回决策:
  `continue`(下一阶段)/ `retry_adjusted`(带新参数)/ `degrade`(标 partial 继续)/ `skip`(记 gap 继续)/ `needs_user`(仅第 3 档)。
- 复用:`agent_runtime/reducer.rs`(终止决策)、`contract.rs` 的 coverage 账本、我做的预算降级(§1.9/§2.1)。

### 9.2 跑完不停 → gap 复查
- skills 流程跑完后,不直接 terminal,进入**有界 gap 复查**:agent 读 覆盖账本(covered vs required families)、机会收件箱、未验证项 → 提出下一步。
- 预算/turn 上限到了 → park 成 **"agent 已复查的 partial"**,附上 agent 的 gap 分析,而不是裸 "failed"。
- 复用现有预算/turn 上限机制,不新造无界循环。

### 9.3 执行概览里显示 agent 介入
- 新增每次裁决/gap 分析一条记录:`agent_stage_decisions(scan_id, attempt, stage, outcome, decision, rationale, ts)`。
- 「执行概览」目标进度旁加一条 **agent 介入时间线**:阶段 → 结果 → agent 决策 → 理由。让用户看到"agent 在这里介入了、决定了 X"。
- 前端属 ui-overview 会话范围;后端出数据。

### 9.4 对话续跑 + 结果汇回
- task park 后,用户在该 scan 的 agent 对话里说"深挖 X / 试试 Y" → 把这句当成一条 **gap followup 指令**续跑**同一 attempt**。
- 复用已有:`prepareGapFollowup` / `agent_gap_followup.rs` / 续跑-保留父 attempt 并合并 的机制。结果并回同一 scan 的 findings/coverage 账本。
- 这正好消解你说的"很繁琐":不是新流程,是把"对话 → gap followup → 续跑 → 合并"这条**已有链**接到 UI 的对话框上。

### 9.5 落地顺序(回来一起做,带花费的那步要你在场)
1. **[已做]** `agent_stage_decisions` 数据模型 + 执行概览时间线后端(§9.3)——纯 additive,不改控制流、不花钱。
   - 表:`src-tauri/src/scanning/agent_stage_decisions_schema.sql`(新表+索引,`CREATE TABLE IF NOT EXISTS`,下次启动自动建,不动任何现有数据)。
   - 模块:`src-tauri/src/scanning/agent_stage_decisions.rs`——`StageOutcome`/`StageDecision` 枚举、`record_stage_decision`(仅记录,**不移动控制流**)、只读命令 `list_agent_stage_decisions(scan_id)`(已在 `lib.rs` 注册)。4 测试通过。
   - bootstrap:`db.rs::initialize` 末尾 `execute_batch(SCHEMA)`。
   - **待前端(ui-overview 会话):** 「执行概览」目标进度旁渲染这条时间线(调 `list_agent_stage_decisions`)。
2. **[已做骨架 + 已接记录线,未接行为线]** 阶段裁决器(§9.1)——`src-tauri/src/scanning/stage_adjudicator.rs`。纯确定性、**不调 LLM、不花钱**:把信号按 §9.0 分三档。白名单用 substring,**同时兼容两套词表**——阶段级原始 stop-reason(`*_schema_invalid` 等)和稳定终态码(`contract.rs::terminal_code`,如 `hard_token_budget`/`confirmed_waf_or_challenge`)。tier1=`_schema_invalid`/`confirmed_waf_or_challenge`→自动吸收;tier2=`budget`/`no_progress`/`rate_limit`→degrade;tier3=`authorization_denied`/`not_authorized`/`reconciliation_required`/…→needs_user;benign(`finish_target`/`configuration`/`model_provider_failure`)→`None` 不表态。tier3 优先匹配,`auto_absorbed` 仅 tier1 为真。9 测试通过。
   - **已接「记录线」(安全,已上):** 在 `commands/agent_terminal_legacy_consumer.rs::record_agent_target_outcome_original` 里,**封闭 single-projection 边界提交之后**、用独立普通连接、每目标一次,调 `adjudicate_and_record` 把本次 attempt 终态码的裁决写进时间线表。**仅记录,不改 `outcome`/`published`/后续控制流**;吞掉所有错误,绝不会拖垮终态路径。
     - 关键安全点:`single_projection` 的写事务装了 authorizer **白名单**(只放行 `agent_events`/`agent_snapshots`/`agent_single_projection_receipts`/`agent_collaboration_events` 的 INSERT),是**故意封死的财务/发布边界**。没有去放宽它——记录改在边界之外做。
   - **故意没做「行为线」(要你在场):** 没有让 `auto_absorbed` 真的改变 fail/continue。原因:实测 `reducer.rs` 对 tier-1/2 信号**本就不硬失败**(WAF→Limited、预算→Incomplete/BoundedCompleted、无进展→soft stall),过去两次硬失败是在各自源头修的。所以"真放行"目前没有现成的硬失败可改;真要动,得对着真扫描边测边看预算,`auto_absorbed` 已备好作唯一开关。
   - **下一步增量:** 目前只在 attempt **终态**记一条(一次 attempt 一行);更丰富的**逐阶段**记录需要在 stage 转换处挂钩(更深),留作下一增量。
3. gap 复查(§9.2)、对话续跑汇回(§9.4)——会额外花预算,**要你在场看预算**。
4. 第 3 档"升级给用户"的 UI。

> 为什么不在你离线时全做:§9.1–9.4 改的是**任务控制流 + 会自动花预算**,且触及"门禁→放行"的判定。这类必须你在场边测边调,无人复核上线风险太高(无界花费/绕过安全)。离线我只做 §9.5 第 1 步那种纯 additive 的,或继续写设计。

## 4. 「魔改」现实(必须对用户讲清)
- 魔改 = 部署代码 ≠ 上游任何 tag。自动拉的**上游原版**当 oracle 不精确,greybox 的 **contradictions(矛盾边)会多**。
- 但矛盾边本身有价值:它标出"源码这样、运行时不是这样"= 魔改点,可作为重点人工复核线索。
- 最佳精度仍是拿到**实际部署源码**(用户有权限时手动给),自动上游拉只是"没有源码时的近似起点"。设计里要如实标注结论置信度(基于上游原版 vs 基于部署源)。

## 5. 分期落地建议
- **MVP**:1–2 个产品(ruoyi 起步)的映射表 + 受控 fetch(带授权+隔离+封顶)+ 自动起 greybox。先验证价值和花费。
- **强约束**:fetch 阶段默认**关闭**,显式开启 + 每产品授权;分析**强制容器模式**(若主机只有 host-binary 分析器、无容器运行时,拒绝对下载源码做分析——因为 host-binary 跑在宿主机上,没有 `--network none`/只读隔离)。
- **度量**:记录每次 fetch+analyze 的 token/时长/命中,校准是否值得扩到更多产品。

## 6. 待定问题(落地前要和用户敲定)
1. 容器运行时:目标机器有 docker/podman 吗?没有则分析沙箱保证不成立(见 §5 强约束)。
2. 映射表谁维护、初期覆盖哪几个产品?
3. fetch 主机白名单范围(仅 github/gitee?内网 git?)。
4. 授权粒度:每产品一次性授权 vs 每次确认。
5. 花费上限策略:fetch+analyze+greybox 合并封顶的具体数值。
