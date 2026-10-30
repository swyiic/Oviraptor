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
