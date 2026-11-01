# 独立指纹包

1.1.121 的应用资源包含 832 条去重默认指纹（原 798 条、23 条企业补充、11 条具名框架/运行组件头）；全新安装无需另行导入。构建期校验与更新方式见 [资源说明](../src-tauri/resources/fingerprint-packs/README.md)。已有用户配置继续按 ID 覆盖，不会因内置相同规则而重复计数。

额外独立指纹包仍可在设置 → 指纹库 → 选择指纹包 → 查看校验与冲突预览 → 确认导入。按规则类型使用 `fingerprints.user.json` 和 `fingerprints.boolean.json`，不操作 SQLite、历史知识库或 CAS；下一轮扫描读取，无需重启。

分享给同事时，选择“导出全部生效指纹”（内置 + 用户覆盖后的去重结果），将生成的 JSON 发给使用新版 Oviraptor 的同事。只分享增量时选“导出自定义指纹”。不需要复制整个用户数据目录、数据库或 poc-kb。原生保存对话框会选择导出位置；为避免覆盖应用数据，目的地必须是应用数据目录之外的普通 `.json` 文件。

仓库附带 [内置规则独立包示例](examples/oviraptor-builtin.fingerprints.json)，由生产导出函数在空临时目录生成，包含最初的 23 条识别规则（该示例保留不变），没有个人数据或 CVE 库。

## 搜索、统计和查看特征

设置 → 指纹库 → 指纹检索可按中文/英文框架名称或 ID 搜索，忽略大小写、首尾空格及全半角差异。搜索和来源筛选均在本地执行，不发起探测或重复加载。

列表以实际生效规则为准，按精确产品 ID 去重：同 ID 自定义规则覆盖内置规则，只显示一次；不同 ID 不因同名而合并。显示去重总数、内置生效数、自定义生效数以及筛选命中数，主动暴露检查不计入指纹总数。读取或校验失败时显示统计不可用，避免把旧数据当作当前结果。

点击结果可展开 Header、Cookie、正文正则、特征路径、状态码和匹配方式；同一路径的状态码与正文条件始终为 AND。每页显示 25 条，搜索、筛选、导入或重新加载后会更新结果与分页，避免数千条规则同时渲染。

## 格式与兼容性

独立包包含 `schemaVersion: 1`、`kind: "oviraptor-fingerprints"`、`manifest`、`fingerprints`、`sha256`。manifest 包含 name、version，以及可选 sources 数组；每个来源可填写 name、repository（无凭证/查询参数的 HTTPS 仓库地址）、revision、license。请按实际来源填写，不把“Wappalyzer-style”当作上游来源证明。

指纹只接受以下字段：

| 字段 | 意义 |
| --- | --- |
| id、name | 稳定的产品标识与显示名称 |
| headerRegex | Header 名称到 Rust 正则的映射 |
| setCookieRegex | Set-Cookie 正则数组 |
| bodyRegex | 首页正文正则数组 |
| pathSignals | 含 path、status、bodyRegex 的站内路径识别条件 |
| matchMode | `any`（默认）或 `all`；按每个原子条件执行 OR 或 AND |

当前 matchMode 表达平面的 any/all 条件，不支持嵌套布尔树；无法无损表达的上游条件应拒绝转换，不能摊平成 OR。同一路径信号内，status 与 bodyRegex 始终是 AND。路径信号必须有正文正则，不允许只凭 200/401 等状态码判断。HTML 登录页和文档页可以作为产品信号，路径跳转与满足同样条件的通用缺失页不会作为路径证据。

兼容原有 `{"schemaVersion":1,"fingerprints":[...]}` 或省略 schemaVersion 的旧识别 JSON；旧 note 不会导出。直接导入完整 Wappalyzer/EHole/Goby 规则、FOFA 查询表达式或漏洞模板不受支持，须先转换识别条件并保留原有布尔语义。未知字段会报错，不会静默吞掉 CVE、PoC、请求模板或检测逻辑。

规则内容 SHA-256 针对规范化后的 fingerprints 数组计算：按 ID 排序；Header 名转小写；模式数组、状态码与路径条件排序去重；省略空信号和默认 matchMode=any；使用生产序列化器的紧凑 JSON 字节。名称/版本/来源属于元数据，不在该校验值内。校验值用于发现规则内容损坏，不代表发布者身份；修改规则后应由应用重新导出，不能沿用旧值。

## 去重、保护与限制

- 同一包内相同 ID、相同规范化内容合为一条；相同 ID、不同内容拒绝整个输入。不同 ID 不会因模式相似而合并，避免把不同产品误并。
- 与现有规则相同则不重复加入；冲突默认保留现有版本，明确选择替换后才覆盖。用户规则覆盖同 ID 内置规则，扫描时每个 ID 只出现一次。
- 预览校验格式版本、规则数、正则、状态码、路径、校验值以及最终合并体积；上限 5 MiB、5000 条规则。禁止 URL、路径穿越、查询参数、空匹配正则和无识别条件规则。
- 预览绑定现有用户文件的修订值；文件在确认前改变时必须重新预览。损坏或无法读取的现有文件不会被导入覆盖，错误会显示在页面。
- 写入前创建原文件备份 `fingerprints.user.<随机值>.bak.json`，同目录临时文件完整写入并同步后替换。备份只用于恢复，不会被指纹导出收集。
- 接受规则的来源信息会合并保留；全部因冲突跳过的包不会被误标为现有规则来源。
- 识别规则每轮扫描加载一次；每个目标的重复路径请求复用结果，最多探测 64 个不同特征路径，另有一次缺失页基线。达到上限会记录日志，未执行路径不能视为未命中。响应读取限制为 64 KiB。

## 与漏洞检测的边界

指纹包只携带产品识别信息；现有主动暴露检查仍是独立内置模块，并且现有扫描流程仍会执行这些检查。导入/预览/导出不会连接目标或启动漏洞验证。产品识别记录是信息级结果，不是 CVE 或有效漏洞的证明。

打包保留原有四个规则文件的明确清单；另包含 `resources/fingerprint-packs/manifest.json` 和经构建期严格登记与校验的 `*.fingerprints.json`。其他 CVE 文件不会因规则目录通配符被自动收进安装包。公开规则的来源、版本、许可证应随独立指纹包维护；应用没有自动克隆混合漏洞仓库的指纹同步能力。

## 回归验证

```sh
node --test tools/test_fingerprint_pack_ui.mjs
npm run build
cd src-tauri
CARGO_TERM_COLOR=never cargo check --tests --offline
CARGO_TERM_COLOR=never cargo test --lib scanning::fingerprint_pack --offline
CARGO_TERM_COLOR=never cargo test --lib scanning::exposure_probe::tests --offline
```

Rust 测试仅用随机临时目录、合成文件和回环 HTTP 服务。类型检查、构建与隔离回归通过不等于安装版 App 已验收；本次没有启动应用或读取真实数据库。

早期独立包阶段验证（2026-10-30）：前端类型检查和生产构建通过；完整 `cargo check --tests --offline` 通过；真实 Cargo 测试二进制中指纹包/导出保护 11 项、识别链路 7 项全部通过；独立界面回归 16 项通过。示例包由生产导出函数生成，23 条规则、6440 字节，规则校验值为 `acf2e366833919cee957ed591d345a36d83b6ea3364f75ed2ec044851361dae3`，未包含 CVE 文本；该阶段没有重新打包、安装或启动桌面 App。

1.1.121 的框架增补、Agent 按需查询及工具结果明文通道见 [框架复核](FINGERPRINT_FRAMEWORK_AUDIT.md)与[任务交互](FINGERPRINT_AGENT_INTEGRATION.md)，其验收与早期示例包分别记录。
