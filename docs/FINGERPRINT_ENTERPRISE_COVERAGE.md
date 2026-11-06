# 企业框架与运行组件覆盖补充（1.1.120）

原始 798 条不是完整产品目录。本次针对具体漏项与转换链路检查后，新增 **23 条独立复核规则，默认合计 821 个唯一 ID**。原 798 包逐字节保留，同名不同条件的旧规则没有为了凑数合并或重写。

## 本次补充

| 类别 | 产品或组件 |
|---|---|
| 企业平台与控制台 | 普元 Primeton EOS / AFCenter、PowerJob 控制台 |
| Java 应用服务器 | 东方通 TongWeb、IBM WebSphere、Caucho Resin、JBoss AS7、WildFly8 |
| 框架与语言组件 | PHP、ASP.NET、Werkzeug、Restlet、Express、Beego、Sails |
| HTTP 服务、代理、网关与数据库服务 | OpenResty、Squid、Gunicorn、Tengine、WEBrick、MiniServ、Python wsgiref WSGIServer、Apache APISIX、Apache CouchDB |

资源文件为 `src-tauri/resources/fingerprint-packs/enterprise.fingerprints.json`，由原有资源 manifest 登记，正常构建自动校验并包含。ID 和规范化产品名与原包检查无直接重复。

逐条依据见 [enterprise-evidence.json](fingerprint-coverage/enterprise-evidence.json)，规则的范围、来源类型、原文对象 SHA、源码 URL/位置及未核验项分开记录。pack manifest.sources.revision 是该证据 JSON 经 UTF-8、键排序、无多余空格序列化后的 SHA-256，不是证据文件原始排版字节的 SHA。

## 普元 EOS 与 /.remote

本地归档包含 `/default/.remote`，原始截图中也可直接观察到 `com.eos.access.http`、`com.primeton.access` 的过滤器/处理器堆栈。本次逐一核对图片和归档哈希；这里是历史截图转录，不伪造为原始 HTTP 字节或本次目标验证。

新规则只接受以下组合之一：

- HTTP 500 错误页标题与两组对应框架堆栈同时出现。
- HTTP 500 错误页标题、`.remote` 路由文本和 EOS 的真实堆栈形式同时出现。
- 独立 JSON 对象同时包含 AFCenter 标题、`/afc` API 前缀与 Primeton 版权信息。字段依据来自[普元 EOS 官方前端配置说明](https://help.primeton.com/eos/8.3.3/eos_dev_reference/web-config.html)。

单独 `/.remote`、`/default/.remote`、`.biz.ext`、200/500 状态、普元品牌、单个类名、普通公告/帮助或外链文档都不能命中。最初候选的外链分支经复核会误认普通帮助页，已删除，并补充阴性样本。公开 AFCenter 配置只用于组件识别，不能据此报告未授权或 IDOR。

## 其他规则依据与范围

Header 规则严格匹配指定的响应头和值；例如 [Express 上游输出代码](https://github.com/expressjs/express/blob/master/lib/application.js)直接设置 X-Powered-By，[APISIX 上游代码](https://github.com/apache/apisix/blob/master/apisix/init.lua)设置 Server，另有已校验的本地原始协议文本。它们识别框架、组件、网关或服务，不推断其后业务应用、受影响版本或漏洞。

JBoss AS7、WildFly8 使用上游默认欢迎页的产品标题 AND 对应的专属 logo，不能用共享的 jboss_community 图片代替，已知覆盖范围限定在所核验模板。

TongWeb 来自人工重新归属的原文发现查询，按明确 Server 头编写独立保守规则，不将原查询不支持的 banner/app 分支删掉后冒充等价转换。WebSphere/Resin 的原厂索引文本与历史观察已记录，直接页面读取失败也记录在来源中，不声称拿到了本次目标响应。

## 为什么原来会漏

- 历史提取把 EOS 的产品名截成 `Platform`，把一些中间件误归到业务应用标签。
- 旧流程只转换明确的产品 DSL；响应头观察、产品提示、端点及无标签查询没有进入运行库。
- 查询中混入不支持的字段或弱 OR 分支时，整体被拒绝；本次通过额外原文/上游证据重写独立规则，保留原拒绝记录。

Apusic、FastAdmin、DataEase、Kettle、WebSphere CE 等仍在证据文件的 pending 列表中，当前依据不足的候选没有激活。该列表是本轮复核范围，不是所有剩余产品的穷举清单。

## 当前识别边界

本轮增加的 23 条均为被动条件，不新增路径 GET、RPC 或 POST 探测。当前生产入口主要将根响应的 body/header/cookie 送入识别器，尚不收集外部 JS 内容、已观察的接口 URL 或请求流量做跨响应组合。EOS 的异常响应或配置必须实际进入识别器才可能命中；把规则包补齐不能单独解决采集缺口。

URL/JS/favicon 等线索后续需要独立的采集与条件语义，不能把路径存在当成产品、把接口相似当成漏洞。现有 10 条主动路径规则及原始 23 条规则的行为保持不变。

[固定响应样本](fingerprint-coverage/enterprise-response-fixtures.json)包含正例、字段错位、产品前缀碰撞、普通城市/公告/帮助/配置、文档引用、框架单词、单独路径及两个产品共享 logo 的阴性对照，使用实际 Rust 匹配函数离线验证。样本是最小合成/上游片段，不能解释为线上覆盖率或零误报承诺。没有向真实目标发请求，也不写真实数据库/CAS或用户规则文件。

## 本轮验证与成品

原生回归28项、资源校验14项、查询界面回归21项通过。108个响应样本（27正例、81阴性对照）由实际Rust匹配函数验证，意外命中0、缺失匹配0。前端类型检查及正常Tauri发布构建通过。新/旧配置均得到821条，原699+76文件未改写；全量重复导入不创建用户副本。

App版本1.1.120，macOS arm64，包内两份资源与源码字节及哈希一致，共821个唯一ID；深层严格签名和ZIP完整性检查通过，未发现随包数据库文件。原798包、原规则和两份用户配置共5项保护文件SHA均不变。未安装或启动新版App，界面交互及真实目标覆盖未实测。机器结果见 [enterprise-verification.json](fingerprint-coverage/enterprise-verification.json)。

ZIP：`src-tauri/target/release/bundle/Oviraptor-1.1.120-macos-arm64.zip`（31,117,520字节）；SHA-256：`c8982c0fb0487dcc7e82c2df673fba39d12c7669bfe7e9ff22cd23362949598b`。
