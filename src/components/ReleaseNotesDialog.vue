<script setup lang="ts">
import { useI18n } from "../i18n";
import ModalShell from "./ModalShell.vue";

defineProps<{ version: string }>();
defineEmits<{ close: [] }>();
const { tr } = useI18n();
</script>

<template>
  <ModalShell :title="tr(`Oviraptor v${version} 更新说明`, `Oviraptor v${version} release notes`)" @close="$emit('close')">
    <template #eyebrow><span class="eyebrow">RELEASE · 2026-08-24</span></template>
    <div class="release-notes">
      <article>
        <strong>{{ tr("重新执行与继续未完成阶段彻底分开", "Fresh reruns and continuations are now truly separate") }}</strong>
        <p>{{ tr("全新执行会清理当前机器结果面后重建；继续未完成阶段只处理未完成目标，并保留正式接口、前端证据、人工确认和任务内登录会话。预检失败会完整回滚，不留下半轮状态。", "A fresh rerun rebuilds the current machine result surface. A continuation processes only unfinished targets while retaining formal APIs, frontend evidence, human confirmations, and task-scoped login sessions. Failed preflight rolls back completely.") }}</p>
      </article>
      <article>
        <strong>{{ tr("当前页面只保留一个最终状态", "The current view keeps one final status") }}</strong>
        <p>{{ tr("任务卡和结果页默认只显示最新一次状态，旧轮次主动展开后才可见。实时执行链与学习输入也只读取最新 attempt，历史错误、旧 Token 和旧工具轨迹不会再混入当前展示或 AI 判断。", "Task cards and result pages show only the latest status by default, with older attempts available on demand. Live traces and learning inputs also read only the latest attempt, keeping historical errors, tokens, and tool traces out of the current view and AI judgment.") }}</p>
      </article>
      <article>
        <strong>{{ tr("SRC 规则从最低基线扩展为现代覆盖矩阵", "SRC rules now expand into a modern coverage matrix") }}</strong>
        <p>{{ tr("公司现有规则继续保留，同时内置 ASVS 5、WSTG 和 API Security Top 10 2023 风险族，补充对象属性权限、敏感业务流、资源消耗、影子 API、第三方信任、实时协议、客户端信任和缓存代理差异。", "Existing company rules remain the minimum baseline, augmented with ASVS 5, WSTG, and API Security Top 10 2023 families covering property authorization, sensitive flows, resource consumption, shadow APIs, third-party trust, realtime protocols, client trust, and proxy/cache differences.") }}</p>
      </article>
      <article>
        <strong>{{ tr("每个目标都有具体的人工深挖路线", "Every target gets concrete human deep-dive routes") }}</strong>
        <p>{{ tr("系统根据真实接口、动作、参数、身份和缺失前置条件生成按投入产出排序的建议，写清现有证据、还缺什么、人工步骤和停止条件；历史任务也能直接补算，不需要重新扫描。", "Target-specific recommendations are ranked by expected value from real APIs, actions, parameters, identities, and missing prerequisites, with evidence, missing inputs, manual steps, and stop conditions. Historical tasks are computed without rescanning.") }}</p>
      </article>
      <article>
        <strong>{{ tr("覆盖缺口永远不会伪装成漏洞", "Coverage gaps can never masquerade as findings") }}</strong>
        <p>{{ tr("人工建议不会进入漏洞数量、AI 验证队列或学习成功样本。模型只接收前三条压缩建议，不再花 Token 重复生成泛化清单；界面明确区分能力就绪、已测试、未发现和未测试。", "Human follow-ups never enter finding counts, the AI validation queue, or successful learning samples. The model receives only three compact leads, while the UI distinguishes available capability, tested, no finding, and not tested.") }}</p>
      </article>
      <article>
        <strong>{{ tr("标准与深度调查不再丢失工具", "Standard and deep investigations retain their tools") }}</strong>
        <p>{{ tr("正常系统提示词不会再误触生命周期恢复。模型保留完整调查工具集，只有宿主发出的精确收口补救消息才会压缩历史并限制为生命周期工具。", "Normal system prompts no longer trigger lifecycle recovery. The model retains the complete investigation toolset; only an exact host-generated recovery message compacts history and limits tools to lifecycle actions.") }}</p>
      </article>
      <article>
        <strong>{{ tr("没有请求响应证据就不会伪完成", "Missing request/response evidence cannot become a false completion") }}</strong>
        <p>{{ tr("真实 repeat_request 与浏览器响应才计入自动验证；空响应、错误和仅侦察结果统一标记为待补充验证。历史错误状态会在启动后精确迁移，证据和执行历史保持不变。", "Only real repeat_request or browser responses count as automatic validation. Empty responses, errors, and recon-only results are marked Needs validation. Historical false states are precisely migrated at startup without changing evidence or attempt history.") }}</p>
      </article>
      <article>
        <strong>{{ tr("模型首轮直接获得可验证核心证据", "The first model turn receives verification-ready evidence") }}</strong>
        <p>{{ tr("正式接口的方法、URL、清洗请求、响应摘要和身份引用直接内联到任务，不再浪费一轮列目录和读取巨大 JSON；旧版侦察缓存会自动失效并按新规则采集。", "Formal API methods, URLs, sanitized requests, response summaries, and identity references are embedded directly in the task, avoiding a wasted directory-listing and giant-JSON turn. Legacy reconnaissance caches are invalidated and recollected under the new rules.") }}</p>
      </article>
      <article>
        <strong>{{ tr("CDP 请求可以直接重放", "CDP requests can now be replayed directly") }}</strong>
        <p>{{ tr("HTTP/2 伪请求头和大小写重复请求头会在进入编辑器前归一化，历史报文也能兼容解析。像 /check_login 这样的真实 POST 在完成一次性授权后不再错误禁用发送按钮。", "HTTP/2 pseudo headers and case-duplicated headers are normalized before entering the editor, while historical messages remain parseable. Real POST requests such as /check_login are no longer disabled after one-time authorization.") }}</p>
      </article>
      <article>
        <strong>{{ tr("匿名、单账号与双账号展示彻底分开", "Anonymous, single-account, and multi-account views are separated") }}</strong>
        <p>{{ tr("匿名任务不显示账号 A/B 或身份选择；一个登录账号显示账号 A 与单账号证据；只有两个以上真实登录身份才展示 A/B 权限差异、分栏和跨账号提示。", "Anonymous tasks show no Account A/B or identity selector. One signed-in account shows Account A and single-account evidence, while A/B authorization comparisons and cross-account prompts require at least two real identities.") }}</p>
      </article>
      <article>
        <strong>{{ tr("本地 Strix 不再在收口续轮越过上下文", "Local Strix no longer overruns context during lifecycle recovery") }}</strong>
        <p>{{ tr("当根 Agent 已完成分析却忘记调用 finish_scan 时，宿主会确定性压缩旧工具历史并只保留收口工具，不消耗额外模型请求。9B 与 27B/35B 也会自动限制单轮输出，为证据和生命周期收口保留稳定余量。", "When the root agent finishes analysis but misses finish_scan, the host deterministically compacts old tool history and retains only lifecycle tools without another model call. Output is also bounded for 9B and 27B/35B models to preserve evidence and completion headroom.") }}</p>
      </article>
      <article>
        <strong>{{ tr("部分完成会保留真正原因", "Partial results retain their real cause") }}</strong>
        <p>{{ tr("后台结果聚合不再用通用计数覆盖目标错误。任务卡会保留模型上下文、服务或目标级中断原因；历史任务已经丢失的说明会从目标记录自动恢复。", "Background result aggregation no longer overwrites target errors with generic counts. Task cards retain model-context, provider, or target-level interruption causes, and recover missing historical explanations from target records.") }}</p>
      </article>
      <article>
        <strong>{{ tr("Asset 查询区不再浪费横向空间", "Asset filters now use the available width") }}</strong>
        <p>{{ tr("清除了旧 Flex 换行和控件最大宽度的残留影响，搜索、项目与两级探测筛选会铺满查询卡片；Asset 导航也与 Strix 保持相同工作流顺序，新建查询固定为末尾高亮入口。", "Legacy flex wrapping and control width caps no longer squeeze the toolbar into the center. Search, workspace, and both probe filters span the card, and New Query is now the final highlighted navigation action, matching Strix.") }}</p>
      </article>
      <article>
        <strong>{{ tr("未完成阶段重试不再继承旧错误", "Incomplete-stage retry no longer inherits stale errors") }}</strong>
        <p>{{ tr("失败、部分完成、熔断和取消任务只重试未完成 URL，并在同一后台操作中完成准备与启动；启动前检查失败会恢复上一轮终态。旧轮次错误只保留在执行历史，不再污染当前任务卡和调查结果。", "Failed, partial, limited, and cancelled tasks retry only unfinished URLs in one prepare-and-start operation. Preflight failures restore the previous terminal state, while earlier errors remain only in attempt history.") }}</p>
      </article>
      <article>
        <strong>{{ tr("探测结果支持精确二次查询", "Probe results support exact secondary filtering") }}</strong>
        <p>{{ tr("Asset 查询可以在一级队列下继续按 Web 可访问、受限、需渲染、需域名、异常、TCP 非 Web、隔离和不可达等真实探测结果筛选；界面会自动匹配兼容队列，避免组合出永远为空的条件。", "Asset search can refine a primary queue by exact observed outcomes such as accessible, restricted, render-required, vhost-required, abnormal, TCP non-Web, blocked, or unreachable, and automatically selects a compatible queue.") }}</p>
      </article>
      <article>
        <strong>{{ tr("不同调查命令不再被误判为重复工具", "Distinct investigation commands no longer look like a repeated tool") }}</strong>
        <p>{{ tr("重复熔断现在按工具名与参数共同判断。列目录、读取证据和发起验证即使都通过同一个命令工具执行，也会被视为不同步骤；本地模型也不再读取完整适配器源码。", "Repeat detection now combines the tool name with its arguments. Directory listing, evidence reads, and validation commands remain distinct steps even through one command tool, and local models no longer read the complete adapter source.") }}</p>
      </article>
      <article>
        <strong>{{ tr("完成任务不再显示“继续当前任务”", "Completed tasks no longer say Continue current task") }}</strong>
        <p>{{ tr("已完成任务显示“再次扫描”，异常终态显示“重试未完成阶段”；仅本地收口改为“仅完成确定性侦察”，明确表示没有进入模型验证。", "Completed tasks show Scan again and abnormal terminal states show Retry incomplete stages. Local-only closure is labeled Deterministic reconnaissance only, explicitly indicating that model verification did not run.") }}</p>
      </article>
      <article>
        <strong>{{ tr("本地 Strix 收口不再误报 exit status 1", "Local Strix shutdown no longer becomes exit status 1") }}</strong>
        <p>{{ tr("进程退出后会等待并核对最终 run.json，中断但已产生工具证据的有界运行不再被旧错误抢先覆盖；Strix 可选远程遥测也已关闭，Oviraptor 本地运行轨迹与 Token 账本保持不变。", "The final run.json is reconciled after process exit, so bounded runs with tool evidence are no longer overwritten by a premature exit-status error. Optional Strix remote telemetry is disabled while Oviraptor's local trace and token ledger remain intact.") }}</p>
      </article>
      <article>
        <strong>{{ tr("匿名访问不再伪装成失效账号", "Anonymous access no longer appears as an expired account") }}</strong>
        <p>{{ tr("未登录任务不会创建“账号 A”身份节点，也不展示身份与权限差异模块；正式接口中的匿名观察会明确标记为“匿名访问”。旧任务读取时自动修复，无需重新扫描。", "Unauthenticated tasks no longer create an Account A identity node or show the identity-difference panel. Anonymous API observations are labeled explicitly and historical tasks are repaired when read without rescanning.") }}</p>
      </article>
      <article>
        <strong>{{ tr("任务详情自动跟随最新尝试", "Task details follow the latest attempt") }}</strong>
        <p>{{ tr("详情页即使已经显示失败或部分完成，也会继续轻量读取最新任务状态；后台完成后会从旧尝试错误自动更新为最终完成结果。", "Even after showing a failed or partial state, the detail view continues a lightweight refresh of the current task row and automatically replaces stale attempt errors when background reconciliation completes.") }}</p>
      </article>
      <article>
        <strong>{{ tr("本地扫描不再继承云端 LLM 凭据", "Local scans no longer inherit cloud LLM credentials") }}</strong>
        <p>{{ tr("Strix 1.5.3 使用的 LLM_API_* 与 OPENAI_* 现在同时按活动 Profile 隔离并覆盖，真实扫描会强制经过当前任务 Hook。需要鉴权的自建服务可以填写独立本地 Key，无鉴权服务继续留空。", "Both LLM_API_* and OPENAI_* variables used by Strix 1.5.3 are now isolated and overwritten from the active profile, forcing real scans through the task hook. Authenticated self-hosted services can use a separate local key while unauthenticated services remain blank.") }}</p>
      </article>
      <article>
        <strong>{{ tr("修复旧数据库升级闪退", "Fixed legacy database upgrade crash") }}</strong>
        <p>{{ tr("任务级登录 Session 新字段会先迁移、确认存在后再创建索引。1.1.35 启动时的 owner_scan_id 缺列崩溃已修复，不需要删除数据库或任何历史数据。", "Task-scoped login-session columns are now migrated and verified before their index is created. The 1.1.35 owner_scan_id startup crash is fixed without deleting the database or historical data.") }}</p>
      </article>
      <article>
        <strong>{{ tr("本地大模型不再留下孤儿推理", "Local models no longer leave orphan inference") }}</strong>
        <p>{{ tr("任务停止、暂停或失败会立即断开仍在执行的本地模型请求；20B 以上自动串行并限制单次输出，同时为 27B/60B 提供更合理的首轮预填充窗口。前端/CDP、沙箱、warm-up 和真实推理现在分别显示。", "Stopping, pausing, or failing a task immediately disconnects active local-model requests. Models above 20B are serialized with bounded output and receive size-aware first-prefill windows. Frontend/CDP, sandbox, warm-up, and actual inference are now displayed as separate stages.") }}</p>
      </article>
      <article>
        <strong>{{ tr("登录 Session 只属于一个任务", "Login sessions belong to one task only") }}</strong>
        <p>{{ tr("登录身份先绑定当前任务草稿，提交后固化到该任务；同一工作空间中的 Asset 新任务、Strix 新任务与重新添加的任务不再显示或复用历史 Session，原任务仍可重新登录后续扫。", "Login identities are scoped to the current draft and then owned by the submitted task. New Asset, Strix, and re-added tasks no longer list or reuse historical sessions, while the owning task can still re-authenticate and resume.") }}</p>
      </article>
      <article>
        <strong>{{ tr("本地模型首轮推理不再被 90 秒误杀", "Slow local first inference is no longer killed at 90 seconds") }}</strong>
        <p>{{ tr("模型请求到达时立即显示“推理中”和估算输入 Token，不再等响应结束才算作进展；本地模型获得适配慢预填充的独立启动窗口，真实无请求、上下文错误和接口错误仍会明确终止。", "An arriving request immediately shows active inference and estimated input tokens instead of waiting for the response to finish. Local models receive a first-prefill startup window, while genuine no-request, context, and provider failures still terminate explicitly.") }}</p>
      </article>
      <article>
        <strong>{{ tr("Asset 任务模式不再隐藏", "Asset scan mode is now explicit") }}</strong>
        <p>{{ tr("Asset 批量送扫可直接选择快速、标准或深度；任务中心和结果页同时显示任务模式与每个 URL 的实际分流，便于判断深度预算是否真正生效。", "Asset bulk dispatch now explicitly selects Quick, Standard, or Deep. The task center and result view show both requested mode and actual per-URL routing.") }}</p>
      </article>
      <article>
        <strong>{{ tr("匿名页面也能使用源码映射做有界调查", "Anonymous pages can use bounded source-map investigation") }}</strong>
        <p>{{ tr("没有自然触发 XHR/Fetch 时，高置信度 source map/AST 中具有准确调用位置的 GET/HEAD 可进入目标验证；普通字符串、动态占位符、遥测和写接口仍被拒绝。调查图谱即使没有模型调用也会显示节点数量。", "When no XHR/fetch is naturally triggered, exact high-confidence GET/HEAD calls recovered from source maps or AST can enter bounded validation. Strings, placeholders, telemetry, and writes remain blocked, and graph node counts stay visible without model use.") }}</p>
      </article>
      <article>
        <strong>{{ tr("Web 调查改为统一策略入口", "Web investigation now uses one policy entry point") }}</strong>
        <p>{{ tr("Asset 待确认任务与 Strix 页面直接任务使用同一后台策略；默认业务前端流程、附加专项 Skills、补充要求、身份、预算和模式不会再因入口不同而失效或分叉。", "Asset drafts and direct Strix tasks now share one backend policy. The default workflow, specialist skills, extra requirements, identities, budget, and mode no longer diverge by entry point.") }}</p>
      </article>
      <article>
        <strong>{{ tr("Standard 与 Deep 真正扩大验证覆盖", "Standard and Deep now expand real validation coverage") }}</strong>
        <p>{{ tr("Quick / Standard / Deep 统一为 4 / 12 / 24 条契约，并同步扩大证据包、验证器分工和定向发现次数；单个无发现分支不会再提前结束整站。", "Quick, Standard, and Deep now process 4, 12, and 24 contracts with matching evidence, verifier partitioning, and targeted discovery. One no-finding branch no longer ends the whole target early.") }}</p>
      </article>
      <article>
        <strong>{{ tr("SRC 专项适配器直接内置", "SRC specialist adapters are built in") }}</strong>
        <p>{{ tr("原始 HTTP、有界竞争、按契约受控写入和攻击链不再需要人工开关；每个目标自动启动唯一 HTTP OAST 回连与查询地址。只有目标网络不能回连当前工作站时，该目标的 OAST 类别才显示网络不可达。", "Raw HTTP, bounded races, contract-gated writes, and attack chains no longer require manual switches. Every target gets a unique HTTP OAST callback and polling endpoint; only targets unable to route back to this workstation show OAST as unreachable.") }}</p>
      </article>
      <article>
        <strong>{{ tr("A/B 重放现在明确绑定账户", "A/B replay is now explicitly identity-bound") }}</strong>
        <p>{{ tr("重放器会分别载入账号 A 或 B 实际采集的 URL、动态签名、Cookie、请求头和请求体；发送按钮、响应与验证历史始终显示使用的账户，缺少完整请求的一侧不会被错误放行。", "The repeater loads the URL, dynamic signature, cookies, headers, and body actually captured for account A or B. The send action, response, and validation history always identify the account, while an incomplete side cannot be selected accidentally.") }}</p>
      </article>
      <article>
        <strong>{{ tr("被降级的监控服务不再消失", "De-prioritized monitoring services no longer disappear") }}</strong>
        <p>{{ tr("Sentry、监控遥测、设备指纹和页面初始化请求继续与正式 API、权限差异及 AI 队列隔离，但会在关联服务清单中保留域名、路径、方法、A/B 身份、观察次数与 CDP 来源。旧任务可直接从检查点恢复。", "Sentry, monitoring telemetry, device fingerprinting, and page bootstrap traffic remain isolated from formal APIs, authorization differences, and AI queues, but now retain host, path, method, A/B identity, observation count, and CDP provenance in a related-services inventory restored from existing checkpoints.") }}</p>
      </article>
      <article>
        <strong>{{ tr("页面文字不再成为权限差异", "Page text no longer becomes an authorization difference") }}</strong>
        <p>{{ tr("身份矩阵只比较同一真实 HTTP 接口的采集、状态和响应结构；FEATURE 路由键、备案号、用户名、新闻标题和其他渲染文本不会进入权限结论。旧任务中的这类记录也会立即从界面隐藏，原始证据保持不变。", "The identity matrix now compares capture, status, and response shape only for the same real HTTP endpoint. FEATURE route keys, registration footers, usernames, headlines, and other rendered text no longer enter authorization conclusions. Historical rows are hidden immediately while raw evidence remains intact.") }}</p>
      </article>
      <article>
        <strong>{{ tr("前端侦察有心跳，Token 0 也可见", "Frontend reconnaissance now has a heartbeat and visible zero-token state") }}</strong>
        <p>{{ tr("双账号 CDP 探测每 5 秒更新已运行时间和总时限，并明确说明该阶段新增 Token 0；即使模型尚未调用或认证在首个请求前失败，任务卡和成本页也会显示完整的 0 请求、0 Token。", "Dual-identity CDP reconnaissance updates elapsed and maximum time every five seconds and explicitly reports zero new model tokens. Task and cost views remain visible with zero requests and zero tokens even before the first model call or when authentication fails first.") }}</p>
      </article>
      <article>
        <strong>{{ tr("扫描不再继承旧 Strix 凭据", "Scans no longer inherit stale Strix credentials") }}</strong>
        <p>{{ tr("每个扫描进程从当前活动模型配置生成独立的 Strix 1.5.3 配置，显式绑定本次 Key 和 Hook 地址，结束后自动销毁；设置页测试与实际扫描现在使用同一份配置来源。", "Each scan process now receives an isolated Strix 1.5.3 config built from the active model profile, explicitly binding the current key and Hook endpoint before removing it on exit. Settings tests and real scans now use the same configuration source.") }}</p>
      </article>
      <article>
        <strong>{{ tr("同一接口不再按账号和 nonce 复制机会", "One endpoint no longer creates per-account and per-nonce opportunities") }}</strong>
        <p>{{ tr("行动中心按稳定 HTTP 契约归并 A/B 观测并保留两侧证据；nonce、hkey、时间戳和身份只属于运行实例。device_id、client_id、request_id 等传输标识也不再被误判为可替换业务对象，历史误报会自动退出活跃队列。", "The Action Center merges A/B observations by stable HTTP contract while preserving evidence from both sides. Nonces, signatures, timestamps, and identities remain run metadata. Transport identifiers such as device_id, client_id, and request_id no longer masquerade as replaceable business objects, and historical false positives leave the active queue automatically.") }}</p>
      </article>
      <article>
        <strong>{{ tr("关闭登录窗口不再破坏原会话", "Closing the login window no longer breaks the saved session") }}</strong>
        <p>{{ tr("登录框关闭现在只是取消本次捕获，会自动恢复打开前的会话状态；校验优先使用登录后成功业务请求，404、网络失败或单个 401/403 不再把有效会话降级。", "Closing the login window now cancels only the current capture and restores the previous session state. Validation prefers a successful post-login business request, and a 404, network failure, or isolated 401/403 no longer downgrades a valid session.") }}</p>
      </article>
      <article>
        <strong>{{ tr("响应字段不再显示整段 JSON", "Response fields no longer contain full JSON bodies") }}</strong>
        <p>{{ tr("运行时只提取 JSON 顶层字段，Python、Rust 和前端同时过滤正文形态和超长字段；旧任务中的“仅 B：整段 JSON”也会在读取时自动净化。", "Runtime capture now extracts only top-level JSON fields, while Python, Rust, and the UI reject body-shaped or oversized keys. Historical full-body only-B rows are sanitized when read.") }}</p>
      </article>
      <article>
        <strong>{{ tr("静态资源、遥测和页面初始化退出权限矩阵", "Static, telemetry, and bootstrap traffic leaves the permission matrix") }}</strong>
        <p>{{ tr("图片、字体、deviceprofile、Sentry、埋点以及 categories/banner/feeds 等页面初始化请求仍保留原始网络审计，但不再作为正式 API 或 A/B 权限差异；跨身份重放只接纳计划中的同一请求。", "Images, fonts, device profiling, Sentry, telemetry, and categories/banner/feeds bootstrap calls remain in raw network evidence but no longer appear as formal APIs or A/B authorization differences. Cross-identity replay accepts only the planned request contract.") }}</p>
      </article>
      <article>
        <strong>{{ tr("修复历史尝试反向覆盖最新 A/B 结果", "Older attempts no longer overwrite the latest A/B result") }}</strong>
        <p>{{ tr("同一任务的多个 attempt 现在按时间顺序同步，并为每个前端结果独立记录签名。最新双账户 CDP 矩阵会稳定保留，不再因文件系统遍历顺序随机退回旧结果。", "Multiple attempts are now synchronized chronologically with an independent signature per recon file. The latest dual-account CDP matrix remains stable instead of randomly rolling back to an older result.") }}</p>
      </article>
      <article>
        <strong>{{ tr("当前小黑盒账户数据自动恢复", "The current Xiaoheihe identity matrix is restored automatically") }}</strong>
        <p>{{ tr("原始第 6 次结果实际包含账号 A 391 条请求/44 个 API、账号 B 430 条请求/78 个 API。启动本版本后会自动替换数据库中被旧第 4 次结果覆盖的 A=0/B=7 展示，无需重新登录或重新扫描。", "Attempt 6 actually captured 391 requests and 44 APIs for account A, plus 430 requests and 78 APIs for account B. This version automatically replaces the stale A=0/B=7 database view without another login or scan.") }}</p>
      </article>
      <article>
        <strong>{{ tr("真实运行时接口会完成一次标准调查", "Runtime APIs now receive one standard investigation") }}</strong>
        <p>{{ tr("风险证据深挖与标准调查不再共用同一个硬门禁。纯静态字符串仍然结束在本地侦察；CDP 已采集正式 API、页面动作或完整身份对照时，会在固定预算内完成只读控制响应和接口契约检查。", "Evidence-guided deep validation and standard investigation no longer share one hard gate. Static strings remain recon-only, while CDP-observed APIs, actions, or complete identity comparisons receive a bounded read-only investigation.") }}</p>
      </article>
      <article>
        <strong>{{ tr("有界计划结束就是本轮完成", "Finishing the bounded plan completes the run") }}</strong>
        <p>{{ tr("达到时间、Token、调用次数或无进展停止规则后，已有证据和覆盖摘要会保存并把目标标记为本轮完成，不再反复提示继续当前任务。只有模型配置错误、没有任何工具结果、会话失效或明确 WAF/验证码/持续限流才保留重试或熔断状态。", "Reaching time, token, request, or no-progress limits now preserves evidence and coverage as a completed bounded run instead of repeatedly asking to continue. Configuration errors, zero tool evidence, expired sessions, and confirmed WAF, CAPTCHA, or sustained rate limits remain retry or fuse conditions.") }}</p>
      </article>
      <article>
        <strong>{{ tr("暂停任务可以按原类型正确续跑", "Paused tasks now resume through their original pipeline") }}</strong>
        <p>{{ tr("代码审计、灰盒和 CI/CD 任务不再错误进入 Web URL 流水线；继续操作会在原任务中创建下一次隔离尝试，已有发现、证据和累计成本保持不变。", "Code, gray-box, and CI/CD tasks no longer resume through the Web URL pipeline. Continuing creates the next isolated attempt on the same task while preserving findings, evidence, and cumulative cost.") }}</p>
      </article>
      <article>
        <strong>{{ tr("只有人工停止才显示已暂停", "Only an explicit operator stop is shown as paused") }}</strong>
        <p>{{ tr("Strix 的 interrupted、stopped 或 cancelled 可能来自预算和生命周期结束，现在统一保存为可继续的部分完成；只有 Oviraptor 明确收到停止命令才进入暂停。停止按钮增加确认，避免误触终止长任务。", "Strix interrupted, stopped, or cancelled artifacts may result from budget or lifecycle termination and are now retained as resumable partial results. Only an explicit Oviraptor stop request becomes paused, and the stop action now requires confirmation.") }}</p>
      </article>
      <article>
        <strong>{{ tr("正式接口不再等于漏洞机会", "A formal API is no longer treated as a vulnerability opportunity") }}</strong>
        <p>{{ tr("CDP 或 AST 发现的请求继续完整保留在 API 清单；只有对象 ID、归属引用、权限/租户字段、敏感响应结构或关键状态变更等可解释安全信号，才进入 AI 自动验证队列并消耗 Token。", "Requests found by CDP or AST remain in the complete API inventory. Only explainable signals such as object IDs, ownership references, privilege or tenant fields, sensitive response shapes, or critical state changes enter AI validation and consume tokens.") }}</p>
      </article>
      <article>
        <strong>{{ tr("普通登录与会话接口退出模型队列", "Ordinary login and session APIs leave the model queue") }}</strong>
        <p>{{ tr("登录、会话恢复、搜索和页面初始化不再仅凭路径关键词获得高分。历史 ready 项会保留证据并归档为 API 清单；每个真正可验证的机会会直接展示 riskEvidence 触发原因。", "Login, session restore, search, and page bootstrap calls no longer receive high scores from path keywords alone. Historical ready items retain their evidence as API inventory, while every truly verifiable opportunity records the exact riskEvidence trigger.") }}</p>
      </article>
      <article>
        <strong>{{ tr("敏感信息使用两阶段规则包", "Sensitive data now uses a two-pass rule pack") }}</strong>
        <p>{{ tr("第一遍匹配独立敏感规则，第二遍统一执行排除信号、排除正则、上下文要求和语义有效性校验。已合并 MobileE 的云密钥、OAuth、Firebase、数据库连接串与已验证误报排除；API 路径和弱加密不会混入敏感信息。", "The first pass matches packaged sensitive rules; the second applies exclusion signals, exclusion regexes, context requirements, and semantic validity checks. MobileE cloud-key, OAuth, Firebase, database-connection, and verified false-positive rules are included without mixing API paths or weak crypto into sensitive findings.") }}</p>
      </article>
      <article>
        <strong>{{ tr("UNKNOWN 静态路径不再伪装成正式接口", "UNKNOWN static paths no longer look like formal APIs") }}</strong>
        <p>{{ tr("没有 HTTP 方法、运行时请求或成功探测的 JS 字符串只保留为本地静态线索，不进入行动中心、目标评分和 Strix 输入。成功的安全 GET 探测会把它明确晋升为 GET 并保留探测来源。", "JS strings without an HTTP method, runtime request, or successful probe remain local static clues and no longer enter the Action Center, target score, or Strix input. A successful safe GET probe promotes the clue to an explicit GET contract with provenance.") }}</p>
      </article>
      <article>
        <strong>{{ tr("历史 UNKNOWN 噪音自动退出活跃队列", "Historical UNKNOWN noise leaves the active queue automatically") }}</strong>
        <p>{{ tr("旧任务中仍为 queued/ready 的 UNKNOWN 静态机会会自动标记为已排除并保留审计记录；真实 CDP 请求、验证响应和人工结论不会被修改。", "Queued or ready UNKNOWN static opportunities from older tasks are automatically dismissed while preserving audit history. Real CDP traffic, verified responses, and manual verdicts are unchanged.") }}</p>
      </article>
      <article>
        <strong>{{ tr("请求重放器改为完整 HTTP 报文", "Repeater now edits complete HTTP messages") }}</strong>
        <p>{{ tr("请求行、Host、请求头和正文现在像 Burp Repeater 一样在同一个编辑器中修改，发送时自动转换回内部结构化契约；响应可切换 Pretty、Raw HTTP 与响应头。", "The request line, Host, headers, and body now share one Burp-style editor and are converted back to the internal structured contract on send. Responses provide Pretty, Raw HTTP, and headers views.") }}</p>
      </article>
      <article>
        <strong>{{ tr("A/B 响应改为直观逐项对照", "A/B responses now use a direct field-by-field comparison") }}</strong>
        <p>{{ tr("账号 A 与账号 B 不再混入同一个 JSON。界面逐项标出采集结果、状态码、内容类型、大小和字段变化，并固定左右展示各自的 Pretty 正文和完整 Raw HTTP。", "Accounts A and B are no longer mixed into one JSON blob. Capture, status, content type, size, and schema changes are compared directly, with separate Pretty bodies and Raw HTTP in fixed columns.") }}</p>
      </article>
      <article>
        <strong>{{ tr("任务、API 与验证队列更紧凑", "Task, API, and validation layouts are more compact") }}</strong>
        <p>{{ tr("任务详情取消重复操作和大块空白，任务文件降级为技术信息；API 筛选和 AI 自动验证卡改为全宽紧凑布局，把空间留给真正的证据。", "Task details remove duplicate actions and wasted space, task files move to technical details, and API filters plus AI validation cards use a compact full-width layout that prioritizes evidence.") }}</p>
      </article>
      <article>
        <strong>{{ tr("A/B 身份证据改为同接口对位", "A/B identity evidence is aligned by endpoint") }}</strong>
        <p>{{ tr("账号 A 与账号 B 固定左右展示，同一接口只占一行，直接给出会话状态、采集完整性、HTTP 状态、响应字段差异和解释结论，并区分待自动复核与预期正常差异；执行过程 JSON 仅保留在调试折叠区。", "Accounts A and B now stay in fixed columns, with one row per endpoint and human-readable session, capture, status, schema, and conclusions that distinguish pending review from expected-normal differences. Execution JSON remains available only in the debug disclosure.") }}</p>
      </article>
      <article>
        <strong>{{ tr("只读接口自动进行跨身份重放", "Read-only endpoints replay automatically across identities") }}</strong>
        <p>{{ tr("当接口只被一侧页面自然触发时，另一侧浏览器会使用自己的 Cookie、Storage 和认证头自动重放同一只读请求，然后再判断正常差异或可疑权限边界；写入动作仍需要一次性授权。", "When only one page naturally triggers an endpoint, the other browser replays the same read-only request with its own cookies, storage, and authentication headers before classifying the difference. Mutations still require one-time approval.") }}</p>
      </article>
      <article>
        <strong>{{ tr("AI 契约队列稳定去重并自动判断", "AI contracts are stably deduplicated and judged automatically") }}</strong>
        <p>{{ tr("动态 nonce、时间戳和签名不再产生几十条重复契约。每次扫描由同一验证器按价值自动处理最多四条契约，普通只读结果、无差异和 401/403 边界无需人工逐条放行。", "Dynamic nonces, timestamps, and signatures no longer create dozens of duplicate contracts. One verifier automatically handles up to four ranked contracts per run, without manual approval for ordinary read-only, no-difference, or 401/403 boundary results.") }}</p>
      </article>
      <article>
        <strong>{{ tr("修复 release App 的 CDP 端口发现", "Fix CDP endpoint discovery in the release App") }}</strong>
        <p>{{ tr("Chrome 已监听本机 DevTools 端口但流水线仍报 endpoint unavailable，是因为 localhost 发现请求可能被代理变量接管。现在使用 Node 原生 http.get 直连 127.0.0.1，目标站点请求仍按代理配置执行。", "The release App could see Chrome listening on a DevTools port but still report endpoint unavailable when proxy variables intercepted localhost discovery. The probe now uses native Node http.get directly to 127.0.0.1 while target requests still follow proxy settings.") }}</p>
      </article>
      <article>
        <strong>{{ tr("修复 TCP CDP 备用通道自身退出", "Fix the TCP CDP fallback startup path") }}</strong>
        <p>{{ tr("备用 DevTools 端口现在通过随机 localhost 端口和 /json/version 发现 WebSocket，并处理浏览器提前退出的 Promise；之前版本可能在备用通道刚启动时就被 Node 未处理异常终止。", "The fallback now discovers its WebSocket through a random localhost port and /json/version, while handling early browser exits. Earlier builds could terminate the fallback with an unhandled Node Promise rejection.") }}</p>
      </article>
      <article>
        <strong>{{ tr("复杂系统探索预算扩大，模型预算保持受控", "Larger complex-system exploration with bounded model spend") }}</strong>
        <p>{{ tr("云端确定性浏览器探索提升到最多 36 个页面状态、60 个动作、2400 个请求和 10 分钟探索窗口；这只增加本地证据采集，不放开 Strix 的 Token、请求和无进展门禁。", "Cloud deterministic browser exploration now supports up to 36 states, 60 actions, 2,400 requests, and a 10-minute window. This expands local evidence collection without removing Strix token, request, or no-progress gates.") }}</p>
      </article>
      <article>
        <strong>{{ tr("修复 Chrome 子进程残留导致的 allocator/CDP 失败", "Clean up Chrome process groups to prevent allocator/CDP failures") }}</strong>
        <p>{{ tr("每次运行时探针现在使用独立 Chrome 进程组，退出时回收整个 Renderer/GPU/Utility 子进程树；连续冷启动和双账号实测均可建立完整 CDP，allocator 警告只作为非致命诊断保留。", "Each runtime probe now uses an isolated Chrome process group and reclaims the full Renderer/GPU/Utility tree on exit. Repeated cold starts and a two-identity run now establish complete CDP; allocator warnings remain diagnostic only when capture is complete.") }}</p>
      </article>
      <article>
        <strong>{{ tr("CDP 运行时改为强制门禁", "CDP runtime is now a hard gate") }}</strong>
        <p>{{ tr("Chrome DevTools Protocol 启动增加握手延迟、三次冷启动重试和进程回收；任何账号的 CDP 未完整成功都会阻止 Strix 启动，不再拿登录会话降级结果继续烧 Token。", "Chrome DevTools Protocol startup now uses a readiness handshake, three cold-start retries, and deterministic process cleanup. If CDP is incomplete for any identity, Strix is blocked instead of spending tokens on a login-capture fallback.") }}</p>
      </article>
      <article>
        <strong>{{ tr("修复健康检查误触发无进展熔断", "Health checks no longer trigger the no-progress fuse") }}</strong>
        <p>{{ tr("模型启动时的 OK 检查继续计入 Token 成本，但不再算作正式扫描轮次或失败。Strix 第一轮真实工具调用可以正常执行。", "The startup OK check remains in token accounting but no longer counts as a scan turn or failure, allowing Strix's first real tool call to run.") }}</p>
      </article>
      <article>
        <strong>{{ tr("调查图谱跟随内容区自适应", "The investigation graph follows its actual container") }}</strong>
        <p>{{ tr("因果链按实际结果区宽度切换四列、两列或单列，长路径不再裁掉右侧假设；验证契约默认只展示最高价值的 12 条。", "The causal chain switches between four, two, and one columns based on result-area width. Long paths no longer clip hypotheses, and only the top 12 contracts render by default.") }}</p>
      </article>
      <article>
        <strong>{{ tr("任务与成本取消常驻双栏", "Task and cost no longer reserves two permanent columns") }}</strong>
        <p>{{ tr("任务列表使用完整内容宽度，选中任务后在下方展开紧凑详情；成本卡、目标和操作按钮都减少了无效占位。", "The task list uses the full content width and expands a compact detail panel below the selection, reducing wasted space across cost cards, targets, and actions.") }}</p>
      </article>
      <article>
        <strong>{{ tr("维护请求与扫描请求可解释", "Maintenance and scan requests are now distinguishable") }}</strong>
        <p>{{ tr("运行轨迹分别标注模型健康检查、上下文压缩和正式 Agent 请求，累计 Token 不丢失，扫描轮次也不会再虚增。", "Runtime traces label health checks, context compaction, and real agent calls separately without losing token totals or inflating scan turns.") }}</p>
      </article>
      <article>
        <strong>{{ tr("现有数据库无需迁移", "No database migration is required") }}</strong>
        <p>{{ tr("旧任务会从前端检查点补全身份状态，下一次重扫自动刷新旧版 A/B 采集结果；原任务、授权记录、证据和累计成本保持不变。", "Existing tasks recover identity status from frontend checkpoints, while the next retry refreshes legacy A/B captures. Tasks, approvals, evidence, and accumulated cost remain intact.") }}</p>
      </article>
    </div>
  </ModalShell>
</template>
