# 当前扫描任务内按需使用指纹和技能

本轮在实际打包的 v1 scanning App/Host/desktop owner 链路新增两项只读工具。这里区分主 Agent 的任务交互、扫描执行器和技能导入快照，不把文件存在或本地回归通过当作全链路实测。

## 主 Agent 可使用的读取顺序

1. 从当前项目的任务上下文或 `scan_list/scan_status` 取得真实 taskId、当前状态与已有证据；不采纳其他项目的任务，不重置已用预算或重复扫描。
2. 用 `scan_results` 读取该任务的 `tech_identified` 记录及必要的证据详情。本轮补齐该类型的安全投影，返回产品 ID/名称与允许来源；识别记录仍不是漏洞证明。
3. 用 `scan_fingerprint_search` 搜索框架名、规则 ID 或特征文本。默认10条、最多20条，返回数量和信号摘要；需要时再 `scan_fingerprint_read` 读取某条完整条件树。
4. 用主 Agent 已有的 `skills_list/skills_read` 找到本会话已选技能，读取入口；再 `skills_list_resources/skills_read_resource` 读取当前问题相关的引用页。正常续问也可保留已有有效授权的参考读取工具，不要求每句话都重复“skill”。
5. 把任务证据与规则条件逐项对照，区分候选、条件匹配、已验证框架/部署关联和漏洞假设。记录 task/project、证据 ID、catalogRevision、规则全文 SHA 及技能版本/摘要，缺哪类响应就写未覆盖。

工具实际前缀以当前工具目录为准。以下是参数关系示意，taskId 和 revision 必须取实际返回值：

```text
scan_fingerprint_search({taskId, query: "Next.js", limit: 5})
scan_fingerprint_read({taskId, fingerprintId: 命中项.id,
                       catalogRevision: 上一条.catalogRevision,
                       offset: 0, limit: 4096})
```

读返回的 content 是 UTF-8 JSON 字节页。hasMore 为 true 时按 nextOffset 和同一 catalogRevision 续读，不能把一页当完整规则。search 续页也须保留 query 和 revision；版本改变会拒绝继续，避免拼接不同库的内容。

## 为什么分成查询与技能

- 指纹包存放可实际执行的识别条件；模型查询时只拿少量摘要或指定规则，不把整个库反复塞进上下文。
- 技能说明如何判断来源、层级、通道、反证、二开和权限边界。相关页包括 `references/framework-identification.md`，知识包另有 `references/framework-observation-catalogue.md`。
- 新线索先复用本任务已有 JS/HAR/正常响应。需要额外验证时沿用原有任务范围、控制协议和预算，读取实际回执；accepted 不等于执行器已应用或验证成功。
- 框架/规则与技能内容都不授予执行、扩目标或提交报告的权限。技能/规则读取不发目标 HTTP，但返回的上下文仍会消耗正常模型输入额度。

例如 `Server: uvicorn` 先指向服务器组件，不能自动选择 FastAPI 漏洞。JS 中的 `/.remote` 只能触发相关识别资料的按需阅读；没有实际请求响应或部署关联时，不确认普元，更不运行反序列化 PoC。

## 两项新工具的边界

| 项目 | 行为 |
|---|---|
| 项目/任务归属 | 必须带真实 taskId，沿用已有项目及任务所有权验证；不允许调用方自行指定另一个 projectId |
| 数据来源 | 仅编译时校验并嵌入的公共指纹，scope 为 bundled-defaults，userOverridesIncluded=false |
| 不返回的内容 | 用户规则文件路径、用户覆盖、配置秘密、完整主动暴露规则库 |
| search | 1–160字符的有界字面量查询；英文大小写/全角形式归一；固定ID顺序分页，不执行用户正则 |
| read | 指定 fingerprintId，最多4096 UTF-8字节/页；完整规则 SHA、总字节数及续读标记；错误字节边界拒绝 |
| 结果体积 | 每个结果最多24KiB，超限返回明确错误，不静默截断后声称完整 |
| 状态与执行 | 不写任务/发现，不访问目标，不添加 HTTP 额度，不调用探测器；结果标为不可信参考与非独立目标验证 |

查询公共默认库不等于拿到了任务当时的实际匹配快照。历史 `tech_identified` 记录未必有规则 revision，用户覆盖也可能不同；必须分别标注本次 catalogRevision 和历史证据，不能回推它们相同。

## 工具结果和任务证据使用明文

按用户明确选择，当前 Agent 工具结果在序列化、进入下一轮模型消息及当前工具正文的上下文裁剪时保留原文，不再自动把 token、64位哈希、本地路径或 URL 参数替换成脱敏标签。`catalogRevision`、规则 SHA 和完整分页内容因此可以逐字节核验。超出工具结果大小或上下文限制时仍会明确截断，不能把截断内容当成完整材料。

`scan_results` 的任务证据仍只投影已有允许字段，但这些字段中的文本保留原值；有效 HTTP(S) URL 保留其原始写法、userinfo、query 和 fragment，拒绝非法协议及超长 URL，不通过解析器重新编码。投影元数据使用 `selected_saved_fields_plaintext`。字段白名单、项目/任务归属、工具授权及技能 lease 继续生效；明文传递本身不增加读取任意配置或数据库字段的权限。

这次调整针对当前工具/任务证据通道，没有更改助手自然文本/推理、OAuth/全局配置日志或长期记忆摘要的既有处理。旧记录或旧摘要中已经保存为脱敏标签的内容无法自动还原；本轮不改写历史数据。

## 技能版本、选择与导入

本次更新的是用户指定目录中的普通技能包：知识包2.5.1、SRC包1.3.1。原生 skills、文件读取及聊天/API 提供内容均可按其入口使用。任务内明文偏好沿用用户明确选择，保留已授权证据原值；对外共享材料按用户选择和项目规则另行生成。

Ovi 中已导入的技能是有版本/摘要的快照，编辑 Wiki 目录不会自动更新应用库。本轮没有对真实数据库执行导入。使用前在当前主 Agent 的 Skills 入口导入/更新这两个实际目录，并核对当前会话列出的版本；保留 references 与 scripts 结构。

主 Agent 的 Host SkillLibrary 与旧原生技能存储是分开的，数字 skillId 不能跨库照搬。工具只保留本会话已选、有效授权/lease 的技能；撤销或版本变化后沿宿主既有规则重新核验，不通过读取文件绕过它。

## 仍未完成的链路

- Native 扫描执行器目前主要注入所选技能入口文本，没有同等的通用引用资源读取工具。新增主 Agent 查询不能被写成所有执行器都已自动读取 references。
- 需要扩展 Native 工具面时，要继续绑定任务选择/版本、限定资源与分页，并把参考读取和目标 HTTP 额度分开；本轮没有放宽现有 HTTP/执行预算门禁。
- 根响应识别不会自动获得渲染 DOM、外部 JS 或已观察 URL 条件。13个 HTML 分支保留为待核验，不由本工具伪装成自动支持。
- 本轮验收以源码、隔离样本、Host/mock、Cargo及成品资源检查为限。未安装或启动新版App，未验证实际模型主动选用新工具或真实任务 Host↔执行器的全部往返。
