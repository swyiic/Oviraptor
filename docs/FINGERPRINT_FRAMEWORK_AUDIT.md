# 框架覆盖复核与技能更新（1.1.121）

本轮保留原有 821 条，新增 11 条已核对输出代码的具名响应头规则，默认共 832 个唯一 ID。新增 Next.js、Nuxt、Symfony Web Profiler、Uvicorn、Hypercorn、Waitress、Tornado、aiohttp、ASP.NET Core Kestrel、Rocket、Caddy。它们分别识别框架、Profiler 或服务器组件，不由服务器品牌推断 FastAPI、Flask、业务产品、受影响版本或漏洞。

[运行规则](../src-tauri/resources/fingerprint-packs/frameworks.fingerprints.json)与原有两包由同一资源 manifest 校验/打包；本轮未新增主动 GET、RPC、POST 或目录枚举。

[75类框架/组件对照清单](fingerprint-coverage/framework-family-inventory.json)按明确规则ID和已复核候选映射维护；其中也明确列出尚未独立核验的项目，不把“列入清单”算作已读取其全部源码或覆盖所有版本。

## 查漏结果如何处理

| 状态 | 本轮处理 |
|---|---|
| 有独立、可表达的运行输出依据 | 11 条具名响应头规则加入默认库，精确字段匹配；Symfony 要求双头组合 |
| 有模板/源码依据，当前 matcher 无法可靠表达活跃 HTML 上下文 | 13 个 HTML 分支保留在候选清单与技能方法中，没有作为默认规则启用 |
| 仅支持渲染后 DOM 或外部 JS 的线索 | 按通道记录，例如 DataEase 默认根 title 为空，浏览器代码再设置品牌 |
| 产品/来源或运行输出尚未核对 | Apusic、Dorado 等保持待核验，不从产品概览或查询标签编造指纹 |
| 已有规则 | FineReport、Smartbi、旧 Jeecg、Pentaho BI 等保留，避免按品牌重复新增 |
| 原有准确性问题 | Struts2 旧规则只靠 .action/.do 通用引用，单列审查；按此前保留旧行为要求，未静默改写原包 |

[逐项来源及限制](fingerprint-coverage/frameworks-evidence.json)包含实际读取的上游 URL、行与文件 SHA，部分还记录不可变 Git blob 标识。Pack sources 第一条 revision 是证据 JSON 的排序紧凑 UTF-8 序列化 SHA；其他来源 revision 是相应仓库所引用文件 SHA 排序组合后的摘要，不冒充部署 commit。来源代码仅用于只读研究，不执行安装/构建钩子或 PoC。

## 没有为了数量启用的 HTML 分支

[候选规则与核验要求](fingerprint-coverage/frameworks-held-candidates.json)保留 Next/Nuxt 的 HTML 分支，以及 Angular、Django 技术404、Rails 欢迎页、Wicket、Tapestry、Vaadin Flow、DWR 引用、Carte/Kettle、JFinal、FastAdmin 模板和 JeecgBoot Vue3 模板的提案。它们不在自动运行库中。

独立复核发现，裸正文正则会把 `data-src/data-id/data-type/data-ng-version`、另一个属性值中的字符串、HTML 注释、惰性模板和教程代码当成真实元素/运行标记。只把单词边界改成空白分隔也不能解决带引号属性值问题。Vaadin 的真实 JS 还可能包含 `<` 运算符，不能靠 `[^<]*` 同时实现脚本边界和代码读取。

DWR 的相对引用还会受 document.baseURI 影响；有外部 base 时，根相对和相对资源都可能指向外域。JFinal/Carte 的品牌链接和页面标题最多支持模板关联，不能由此断言本地后端正在运行。需要具备正确元素/属性和上下文解析、资源最终 URL/来源及部署关联后，才决定是否转为自动规则。

原有 raw-body 条件没有被改成新的 HTML 语义。新增规则和对照样本不会使旧规则自动获得这些能力。

## 写入技能的方法

两个通用技能包分别增加框架识别与任务内按需调用专题，知识包另有按观察通道组织的具体框架目录。保留完整目录后可由原生 skills、文件读取或用户提供上下文的宿主使用，不固定模型、客户端或安装目录。

方法先区分前端引用/模板、运行组件、后端框架、业务产品、版本与补丁，再按根响应、渲染 DOM、静态资源、HAR、源码及部署材料建立证据。正文路径不冒充已观察请求，公开配置不升级成未授权，接口相似不升级成目标漏洞；正负样本与真实目标准确率分开。

技能文件更新不会自动覆盖 Ovi 已导入的技能快照，也不会自动安装到宿主。主 Agent 和扫描执行器的存储与可用工具需要分别核对；当前任务调用链见 [Agent 交互说明](FINGERPRINT_AGENT_INTEGRATION.md)。

## 验证口径

[新规则响应样本](fingerprint-coverage/frameworks-response-fixtures.json)覆盖合法具名头、字段大小写、错位头、前缀碰撞、普通帮助文本、data-属性/注释/模板等不应激活默认规则的输入，以及服务器与框架头同时存在的多层情况。候选提案测试保留在单独文件中，不把它们混算为已启用规则的准确性验收。

构建和本地合成回归不能替代 installed App/真实目标验收。真实数据库、CAS、用户规则文件与已有目标均不作为写入/探测对象；新功能仅使用隔离样本和只读公共规则进行验证。

本次本地执行：新规则127个响应样本全部符合预期；指纹相关 Cargo 22 项与已选证据 Cargo 5 项通过（其中一个旧指纹投影用例重叠），原有暴露检查11项通过；Host工具/任务/明文链路26项、界面21项、资源预检14项通过。空数据目录导出832条完整独立包，再向新临时目录导入/重复导入通过，未生成用户规则文件。上述数据不代表真实目标准确率。

知识技能2.5.1、SRC技能1.3.1已更新到指定Wiki实际目录；两个入口校验通过、105处本地链接有效，无新审计发现。原有8个medium审计提示保持，未把“无新增发现”写成所有历史问题已清零。当前工具结果/已选任务证据改用明文有界传递，具体范围与仍保留的处理见 [交互说明](FINGERPRINT_AGENT_INTEGRATION.md)。

正常桌面发布构建生成1.1.121 arm64 App，签名深度/严格验证通过，3个指纹包与源文件逐字节一致，v1 scanning App为1.4.3，两项查询工具和最新Host源码均在成品中。ZIP通过完整CRC检查，并逐字节核对125个关键文件；未包含数据库文件。归档大小、SHA256、测试及权限边界见 [验收记录](fingerprint-coverage/frameworks-verification.json)。本轮没有安装或启动新版App，也未通过实际模型/真实扫描任务验收。
