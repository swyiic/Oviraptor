# Agent × Nest 智能化流程设计（灵活分配 / Coordinator 调度）

> 本文是我们设计讨论的合并定稿,**取代**早先那版"策略师/监督者/分析师"角色卡设计——角色卡是错误的拆法。本文给出:别人怎么造(有依据)、正确的核心模型(灵活分配而非固定角色)、映射到 Nest 现状缺哪几块、控制面与护栏、持续验证层、落地顺序。
>
> 配套文档:实时事件管道与门控反应的现状见 [AGENT_NEST_LOOP_PROPOSAL.md](AGENT_NEST_LOOP_PROPOSAL.md);确定性地板(浏览器采集)的故障诊断见本文 §7 与会话记录。

## 1. 结论先行

1. "让 LLM 盯扫描的每一步"是错的——**没人这么干**。所有能打的系统都把 LLM **包在确定性里**(持久工作流、模型调用前先确定性分诊、确定性复盘与持续验证)。
2. 最能打的那套(XBOW)不是角色卡,而是**一个持久 Coordinator 带预算动态调度一堆短命、同质、窄目标的执行体**——这就是"灵活分配"。**你的直觉是对的,而且 Nest 的骨架(Coordinator + 同质 worker)本来就是这个形状。**
3. Nest 现在缺三块:①Coordinator 的分配决策还是死启发式(该换成看证据的智能分配);②几乎没有**持续验证**层(这正是 XBOW 的核心竞争力);③**确定性地板没通**(现代 SPA 的浏览器运行时采集坏了,导致根本没证据可供分配决策)。

## 2. 别人怎么造(landscape,有依据)

三种档次:

1. **全代理驱动**:LLM 开全程。贵、慢,用在高价值、要"验证过的真漏洞"的场景。
2. **AI 辅助**:确定性流水线,只在个别步骤上模型(分诊/解释)。**市面商业产品大多是这档**。
3. **混合编排**:确定性内核 + LLM 做**编排/分配决策**。Nest 属于这档,也是本文主张的方向。

**跨所有档次的通用铁律:确定性裹着 LLM。**
- Inspectra:每次扫描是**可持久化工作流**,**任何模型调用之前先确定性分诊**;工作流提供重试/超时/取消/重启存活,这些是裸 agent SDK 给不了的。
- XBOW:用**确定性逻辑**给 agent 复盘、提炼、排下一步;**持续验证**(validator 用生产安全方式确认可利用,不是最后才验)。

**"灵活分配"是 SOTA,不是角色卡:**
- **XBOW**:持久 Coordinator + 成千上万**短命 agent**(fresh context、各一个窄目标);协调 agent 维护全局视图、**动态决定下一步测哪里**。"角色"是临时派出去的目标体,不是固定人设。
- **Strix**(开源,仓库里 `strix-jobs` 很可能是它):更偏角色卡——一张 agent 图,测注入/映射面/验证各一个。
- 学术界"每漏洞族一个 agent"是最硬的角色卡。

→ 越灵活(Coordinator 动态派发)越强;越角色卡越死。

## 3. 核心模型:Coordinator 调度(灵活分配)

**不设角色。** 给 Coordinator 三样东西,让它动态决定把智能花在哪:

1. **一份共享证据状态**(黑板):侦察/探测/验证的结构化证据都写进去,Coordinator 和执行体都读写同一份。
2. **一个预算**(token / 请求 / 时间):Coordinator 在总预算内**自由重分配**,不能凭空扩预算。
3. **一组可派发的窄目标**:"对目标 X 做带认证的运行时采集"、"验证这个 IDOR 假设"、"对这批低信号目标做一次价值 triage"——都是**同质执行体 + 一个目标**,不是固定岗位。

Coordinator 做的事(**是状态流动,不是三个 agent**):
- **分配**:看证据,决定在哪些目标、花多少预算、派什么执行体。
- **复盘(确定性)**:执行体回来,先用规则提炼/去重/判进展,再决定要不要继续。
- **验证(持续)**:对疑似结论即时派验证执行体确认可利用,而不是攒到最后。
- **收敛与合成**:无新增价值时收尾,把证据合成发现+报告。

> 我之前说的"策略师/监督者/分析师"**只是 Coordinator 在不同时刻在做的事**,它应按证据在这些状态间自由流动,**不该被钉成三张卡、三个 prompt**。

**LLM 用在哪:** 只用在**分配与判断**(该深挖谁、派什么、新证据怎么解读、是否升级/收尾、结论是否成立)。**干活(爬取/指纹/抓包/探针/去重/存证)全是确定性内核,LLM 不碰。** 这样 LLM 调用从"每步每目标"降到"少数岔路口",成本时延降一两个数量级,内核照样并行几百目标。

## 4. 映射到 Nest:已有的骨架 + 缺的三块

**已有(骨架是对的):**
- Coordinator + **同质 worker**(不是角色专员)+ assignment/directive。
- 结构化证据:侦察已产出 `valueScore / surface / apis / opportunities / hypotheses / incrementalDecision / verificationPlan` 等。
- 投递管道:实时事件(A)+ 门控反应(B)+ `task_binding` 的 directive/supplement/execution-control(pause/cancel)——**本会话早先已搭好**。
- 预算体系:每层(quick/standard/deep)token/请求上限。

**缺的三块:**
1. **智能分配**:Coordinator 的分配决策现在是死启发式(典型:`record_agent_routing_skip` —— 静态 `valueScore=0` 就跳过模型调查)。应改成**看证据、带预算的分配**(见 §5 控制面、§8 阶段二)。
2. **持续验证层**:当前基本没有"确认可利用"的闭环。这是 XBOW 的核心竞争力,也是降误报的关键(见 §6)。
3. **确定性地板**:现代 SPA 的浏览器运行时采集未执行(`browser_network_broker_unavailable` / `captureStatus: not_executable`),导致 SPA 一律 0 分、被跳过。**地板不通,上面再聪明也空转**(见 §7)。

## 5. 控制面：Coordinator 能掌舵的旋钮

这些 Nest 已有,只是现在由内核死规则定、没交给智能分配:

| 旋钮 | 现状 | 智能分配后 |
|---|---|---|
| `automationTier`(recon_only/standard/deep) | 按 valueScore 死定 | 按资产价值+证据动态定 |
| `scanMode`(skip/manual_review/investigate) | 低分即 skip | **改成"升级阶梯",见 §8.1** |
| 升级开关:运行时采集 / 认证采集 / 深爬 | 内核默认 | 对低静态信号的高价值目标主动开 |
| hypotheses / opportunities 队列 | 内核生成 | 追加定向假设 |
| `verificationPlan` / `requiredBaselineFamilies` | 固定 | 按目标裁剪要测的族 |
| 每层预算 | 配置固定 | 总预算内按价值重分配 |
| execution-control(pause/cancel/supplement) | 已有 | 叫停/补投/定向纠正 |

## 6. 持续验证层（Nest 最缺、价值最高）

借鉴 XBOW:**结论不是模型说有就有,要用生产安全的方式确认可利用,且边跑边验。**
- 疑似结论一出现,就派一个**验证执行体**做**非破坏性**确认(例:IDOR → 找端点→验端点→找对象引用→验越权访问,分步确认)。
- 验证是**确定性为主**(重放、对比、状态校验),模型只用来设计验证步骤和判读。
- 未通过验证的,降级为"线索/覆盖缺口",**不冒充漏洞**(Nest 已有 `manualDeepDive`/coverage-gap 结构,正好承接)。
- 收益:大幅降误报——这是"产出有用东西"的关键,比多测更重要。

## 7. 确定性地板：当前的具体故障（必须先修）

现象:创建任务→~2 秒侦察→模型调查被跳过→无结果;历史任务删不掉(`scan_quiescence_original_target_exit_missing`)。**同一根因。**

证据(扫描自产文件):SPA 目标 `valueScore:0`、`surface:static_frontend`、`modelGate:false`、`runtimeBrowserAvailable:false`、`runtimeCaptureStatus:not_executable`、`runtimeStopReason:browser_network_broker_unavailable`、`nodeVersion:unknown`。

已排除:Node v26(/opt/homebrew/bin)、Chrome+Edge(内置清单位置)、探测脚本与 `browser-locations.json` 均在安装包内、位置正确。**材料齐全却报 not_executable。** 代码里侦察路径默认 `browser_broker: None`(该 broker 目前只在 agent 交互式浏览器工具路径才开),需进一步定性:是启动/握手 bug,还是该采集需要某能力/授权开关。**这是落地阶段一的第一件事。**

修复方向还包括把 `record_agent_routing_skip` 的"低分即跳过"改成**升级阶梯**:
`静态 → 运行时采集 → 带认证采集 → (分配决策裁决) → 才可能 recon_only`,且永远"保留并标注待复核",不是"消失"。

## 8. 落地阶段（先修地板,再盖楼）

### 阶段一:修好确定性地板
1. 定性并修复浏览器运行时采集(§7)。
2. 跳过门禁 → 升级阶梯。
→ 光这步,你那种 BI/SPA 目标就能真正进调查、出证据。

### 阶段二:智能分配(把死启发式换成看证据的 Coordinator 决策)
侦察完成后,把一批目标的证据喂模型做**批量 triage**(低频、便宜),产出每个目标的「层级+策略+初始假设」,写回 §5 控制面。这是"灵活分配"的第一步落地。

### 阶段三:监督回路(事件驱动的动态纠正)
接 A/B 管道:新阶段事件 → 门控(B1 账本:每 attempt 次数上限+爆发熔断,已实现)→ 一回合判断 → 有界指令 → 内核消费。**默认只读建议,人确认有价值后再放开有界自动写。**

### 阶段四:持续验证层(§6)
把"确认可利用"做成边跑边验的确定性闭环 + 模型判读。

### 阶段五:合成
按证据合成发现+报告+人工复核清单。

## 9. 护栏（无人值守也不失控）

- **预算**:每层硬上限;只能重分配,不能凭空扩。
- **写作用域**:智能分配只限一组预授权安全动作(升级层级/加假设/调验证/pause/supplement),**不含**扩范围/碰新目标/越权。
- **幂等+在途感知**:同 `(目标,意图)` 不重复下达;读 `pendingCommands` 不与在途指令打架。
- **熔断**(B1 已实现):断掉"agent↔内核互相刺激"的回授环。
- **人工门**:高风险或超预算的升级默认转人工确认。
- **默认只读起步**:监督/分配先"只建议不自动写",验证通过再逐步放开。

## 10. 和原设想 / 角色卡的对比

| | 原设想(LLM 盯每一步) | 角色卡(固定岗位 agent) | 本文(Coordinator 灵活分配) |
|---|---|---|---|
| LLM 位置 | 热路径,每步 | 每角色固定介入 | 只在分配/判断的岔路口 |
| 成本/时延 | 极高 | 高 | 低(降 1–2 个数量级) |
| 并行 | 没了 | 受限 | 内核照样几百目标并行 |
| 灵活性 | 伪灵活(其实死循环) | 低(岗位钉死) | 高(按证据动态派发) |
| 确定性/可复现 | 差 | 中 | 高(干活确定,判断才上模型) |
| 对标 | 无 | Strix / 每族一 agent | XBOW(SOTA) |

## 10b. 能力扩展模型（新增检查怎么加：模板 vs skill）

新增扫描能力不是"全写死规则"也不是"全做成 LLM skill",而是**按能力性质分三层归位**,统一由 Coordinator 按适用性分配。

**第 1 层 · 确定性高信号检查**（git 泄漏、`.env`、备份文件、`.svn`、`.DS_Store`、`/.well-known/security.txt`、已知 CVE 固定路径、框架特有敏感路径）
- **形式:声明式主动检查模板**(数据,不是硬编码 Rust,也不是 LLM skill)。
- **归位:** 新增一个"主动检查"注册表 `resources/rules/active_exposures.json`(现有 `resources/rules/*.json` 只做被动内容匹配,这是第一个"主动探测"模板集)。
- **模板字段:** `{id, title, severity, appliesTo(指纹前置), path, method, match{status/bodyContains/bodyRegex}, negate(软 404 排除)}`。
- **软 404 处理(关键):** 很多站对任意路径都返回 200。执行器**先探一个随机不存在路径建立"未命中基线"**,任何与基线同构的响应判为未命中,避免误报。
- **为什么不硬编码/不上 LLM:** 这类检查要频繁增删 → 数据驱动免重编译;它是确定性机械活 → 上模型只会又慢又飘。git 泄漏探测近乎免费,**对所有 HTTP 目标都跑**。

**第 2 层 · 适用性("有的站适用有的不")**
- 每个模板声明 `appliesTo` 前置,拿 recon 指纹匹配(git 泄漏→任意 http;nginx 状态页→`server=nginx`;WordPress 路径→`framework=WordPress`)。
- **Coordinator 按指纹选择跑哪些模板** —— 既不是每站全跑,也不是每站写死,而是数据驱动的适用性选择(§3 的灵活分配)。

**第 3 层 · 判断 + 联网(开源二开)**
- **形式:agent 能力/skill + 工具**,不是模板(识别"是哪个开源项目的二开、哪个版本、改了什么"是判断+检索)。
- **流程:** recon 指纹 → agent 猜候选上游 → 联网搜代码托管平台 → 拉上游仓库 → **喂进已有的 SAST 引擎(semgrep/codeql rule_packs)** → 与目标做差异/映射 → 推出针对性测试点。
- **归位:复用已有源码分析链路**。缺口只有:①给 agent 加"代码托管搜索 + 拉仓库"工具;②"指纹→识别上游→拉取"前置。分析引擎已存在。

**统一原则:** 每个能力 = `{探测器, 适用判据, 成本/层级}`;机械的→声明式模板,要判断/联网的→agent skill+工具;Coordinator 按适用性分配(便宜的广撒,贵的按证据才上)。

**落地顺序:** 先做第 1 层(主动检查模板 + git/.env/备份),确定性、安全、立刻出发现,经 `insert_finding` 直接进结果(不受 API/valueScore 门禁约束);第 3 层(二开 OSINT)后续,需加联网代码搜索工具。

## 11. 需要你拍板的决策

1. 采用"Coordinator 灵活分配 + 共享证据黑板 + 持续验证",不走角色卡——确认?
2. 落地从**阶段一(修地板)**开始——确认?(没证据,调度空转)
3. 智能分配/监督验证通过前**默认只读**(只建议不自动写)——确认?
4. 持续验证层的优先级:是否提到阶段二之前?(它最降误报,但也最重工程)

## 来源
- XBOW 平台页(Coordinator+并行短命 agent):https://xbow.com/platform
- XBOW 博客(短命 agent + 协调 agent 动态定下一步,确定性复盘):https://xbow.com/blog/ai-pentesting-strengths-weaknesses-xbow-fills-gaps
- Wavestone(全代理 vs AI 辅助,多数商业产品是后者):https://www.riskinsight-wavestone.com/en/?p=29693
- Agentic Security 系统化综述(Inspectra:持久工作流 + 模型前确定性分诊):https://arxiv.org/pdf/2608.21423
- Strix how-it-works(agent 图 / BaseAgent):https://www.mintlify.com/usestrix/strix/concepts/how-it-works
