# 内置 Python Worker → Rust 迁移记录

更新：2026-09-03。当前版本号维持 1.1.59；本记录不是新安装包的发布说明。

## 当前结论

内置 Python 执行入口及历史 CSV 导入工具已替换为 Rust，旧 Python 文件不再参与运行或打包。但“已经没有内置 Python Worker”不等于“旧版本全部高级行为已逐项证明等价”。下面列出的覆盖缺口仍属于发布前待办，不能隐藏，也不能把单元测试通过当成全面等价验收。

第三方 Strix CLI 仍需要它自己的 Python 环境；CDP 浏览器控制与 Babel AST 解析仍使用 Node.js。这两部分不是本次移除的内部 Python Worker，不应卸载其运行时。

## 模块边界

| 职责 | 当前实现 |
| --- | --- |
| CSV、URL 归并、资产优化、公共数据工具 | `src-tauri/src/jobs/native_workers.rs` |
| 候选分层、证书归并、重试计划 | `src-tauri/src/jobs/native_refinement.rs` |
| FOFA 查询、账号档位字段、缓存与重试 | `src-tauri/src/jobs/native_fofa.rs` |
| HTTP/TCP 探测、阶段结果与断点日志 | `src-tauri/src/jobs/native_probe.rs` |
| 总速率、单主机间隔与取消 | `src-tauri/src/jobs/native_probe_control.rs` |
| 内容分类与自定义规则 | `src-tauri/src/jobs/native_content.rs` |
| 配置兼容读取，不执行旧 Worker | `src-tauri/src/jobs/native_config.rs` |
| 前端侦察与独立身份采集编排 | `src-tauri/src/commands/native_frontend_recon.rs` |
| 子进程并行管道读取、输出上限、超时和取消 | `src-tauri/src/commands/native_helpers.rs` |
| 身份汇总、请求证据、静态 URL 解析 | `src-tauri/src/commands/native_recon_contract.rs` |
| 敏感规则匹配和排除/语义复核 | `src-tauri/src/commands/native_sensitive.rs` |
| 原始 TCP/TLS 报文、有限并发契约执行 | `src-tauri/src/commands/native_src_transport.rs` |
| 离线历史 CSV 导入 | `tools/import_existing_results.rs` |

## 已落实的行为约束

- 探测按单个结果完成顺序更新进度，不等待一批任务中最慢的第一个主机。
- 每个已完成探测结果立即写入并刷新 JSONL 日志。输入或有效配置变化会生成新的日志指纹；取消结果不充当成功缓存。损坏的最后一行不阻止其余有效记录恢复。
- 恢复已完成结果和重新探测的行为有本地 HTTP 测试；不会因为旧配置指纹相同与否不明而直接复用。
- 子进程 stdin/stdout/stderr 同时处理，防止较大 JSON 堵住管道；输出超过上限或不是合法 JSON 时返回明确错误。
- 前端结果先写本次唯一暂存文件，只有当前任务仍接受本轮结果时才发布。失败后只保留本轮已发布证据，不回读历史文件冒充新结果。
- 多账号使用独立会话 ID；显示名称相同也不会合并。匿名访问使用 `anonymous`，不伪装成账号 A；缺少会话有效性证据不会默认显示有效。
- 汇总保留各身份的接口、脚本、路由、功能、请求和响应；接口附带 `identityObservations`，用于加载该身份自己的重放基线。
- 同接口归并包含来源 origin、路径、方法与查询参数名；不会把不同域名相同路径合并。未知方法不自动伪造为 GET。
- 静态 AST 候选不直接当正式接口；显式 clientBaseUrl 参与拼接，动态占位符不拼成可执行 URL。相对模块 import 在同一脚本预算内处理，预算耗尽的脚本标明 deferred。
- 路径包含 login/admin 等只是调查线索，不等于已有风险证据。A/B 当前结构对比明确标注基于状态码、内容类型和字段名，不宣称已经完成业务数据授权判定。
- 对照请求只复制来源请求的内容协商头；认证头从目标身份取得。真实浏览器本地测试覆盖自定义认证头不串号。
- 原始 HTTP 使用 TCP/TLS 直接发送原字节，保留重复头及二进制请求体。TLS 保持证书校验，不降级明文。结构化请求不跟随重定向，不自动解压改变响应摘要。
- 并发契约遵守 attempts/concurrency 上限，返回状态分布、错误数与响应摘要数；差异不是自动漏洞结论。取消后不再派发新轮次，已执行写契约仍保留清理机会。
- 验证请求不阻塞回连监听；最多同时接受两个适配器任务，其余明确返回 busy。接收器退出时等待已经开始的有限时长请求结束，不继续派发新任务。
- 历史 CSV 导入按文件事务提交，失败回滚当前文件并写入失败状态；不修改源 CSV，不创建拼错路径的空数据库。
- 应用打包只收录必要 CJS 与许可证，不再用通配目录把 Python 字节码或测试缓存打进安装包。

## 验证方式

本轮实测：后端全量 164 项通过；导入工具及其复用数据库结构测试 11 项通过；本机 Chrome/CDP 匿名与身份隔离回归通过；前端生产构建、桌面应用开发构建、导入工具帮助命令和 Git 差异检查通过。尚未生成或发布新安装包。前端构建仍有单包超过 500 kB 的体积警告，不是构建失败。

在仓库根目录执行：

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1
cargo test --manifest-path src-tauri/Cargo.toml --features import-tools --bin import-existing-results
node tools/test_native_runtime.cjs
npm run build
cargo build --manifest-path src-tauri/Cargo.toml --bin oviraptor
git diff --check
```

浏览器回归只启动 127.0.0.1 临时页面和合成身份，覆盖自动点击触发 API、响应采集、写动作不发送、匿名身份和跨账号认证头隔离；不访问业务目标，不使用真实 Cookie/API Key。

导入工具的测试直接初始化应用现有数据库结构，验证中文/BOM CSV、重复导入、项目关联去重、排除记录、事务回滚和最终状态。原始 HTTP 测试验证接收到的请求字节、重复头、二进制正文和完整响应摘要；并发/回连测试验证调度计数、取消与回连接收互不阻塞。

## 尚不能宣称等价的部分

- [ ] 旧前端分析的 source map 全流程，以及部分业务代码片段、加密线索、注册流程、实时通信和功能矩阵输出，尚未完成逐项迁移验收。当前部分字段仍为空，不能解读为目标不存在这些行为。
- [ ] FOFA 的已确认资产派生扩展、证书/指纹扩展及原工作簿输出，仍需逐项核对；现有基础查询/字段降级测试不覆盖全部高级配置。
- [ ] 探测的虚拟主机纠正、跨运行缓存过期等高级分支尚需旧/新输入输出对照。
- [ ] 敏感规则当前已覆盖现有 JSON 规则包与二次排除；旧 Python 内置额外规则种类仍需完整差异清单，不能假设文件迁移就代表规则覆盖完全一致。
- [ ] TLS 正向/异常证书、Windows 和 Intel Mac 实际运行、安装包启动、UI 事件延迟及长时间负载仍需单独验收。当前本机编译和回环测试不替代跨平台验收。
- [ ] 未测量同一工作负载下的旧/新峰值内存、耗时和 UI 延迟，因此没有提供未经测量的提升百分比。

完成以上检查前，不把当前工作树标为“全部功能等价迁移完成”，不把前端采集成功标为完整扫描成功。

## 历史文件与恢复

旧 Python 源文件作为已跟踪文件的删除保留在 Git 差异中；本次没有清理业务数据库、历史扫描记录、用户模型配置或第三方 Strix 环境。

生成的 Python 字节码缓存已从仓库移到临时隔离目录 `/tmp/oviraptor-python-cache.RwKNaF`（workers、tests、tools 三组）。需要时可找回；临时目录可能被系统清理，不是长期备份。
