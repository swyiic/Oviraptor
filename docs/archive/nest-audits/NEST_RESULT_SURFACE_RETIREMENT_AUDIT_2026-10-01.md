# 结果面准备：停止隐式清理退役数据（2026-10-01）

状态：本切口代码门禁通过；Strix 完整退役及 Master Plan 未完成。

## 问题与作用域

完整阅读交接、progress 顶部及 Master Plan 后，核验 HEAD 和工作树：598 项 status 记录、656 个未跟踪文件（目录展开计数），没有在途 Cargo 回归。仅逐文件审查本次拟改文件，不把已有增量视作本轮成果；没有 reset、批量覆盖、commit/push、访问真实数据库/CAS 或授权 URL。

`prepare_current_attempt_surface` 唯一生产调用方是 `persist_web_startup_in`，在 Web 启动 IMMEDIATE 事务内调用。旧实现即使 Native marker 已为当前 attempt 也会删除旧 marker；其他路径删除旧 finding/checkpoint/signature，fresh 整面删除同样覆盖旧行。signature 的 LIKE 拼接会把 scan ID 内 `%`/`_` 当通配符。以上不是用户确认的数据清理入口。

## 最小修改

- 结果准备前只读判断本 scan 的明确退役 finding stage、checkpoint stage 和精确 marker/signature 命名空间。有任一记录就返回 `retired_result_data_requires_confirmed_cleanup`，先于幂等快捷返回和任何结果面写入。不读取或解释旧报告/状态内容，不自动迁移、不删除旧行、不赋予 Native 权限。
- 移除全部旧 finding/checkpoint/signature/marker 的隐式删除。纯 Native fresh 保留现有整面重建及人工作业保留合同；resume 保留当前 recon/learning outcome 和证据，重建本轮 pending opportunities。当前 Native JSON、导入合同及 schema 未改，没有新增 migration。
- 签名命名空间通过字符串等值／substr 前缀比较检测，scan ID 不是 LIKE/GLOB pattern。checkpoint 的 GLOB 使用固定字面量，不能由 scan ID 扩大作用域。
- 旧测试中三个清理正向合同改成拒绝且原行不变；fresh 正向夹具改用 Native stage/validation key，保留原有重建、人工作业断言。没有删除独有安全测试。

## 失败与验证

- 修改实现前：resume 负向红测失败，原路径未拒绝且删除 checkpoint。
- 修改实现前：preparation 组合 14 通过／2 失败，含旧数据拒绝与 `%`/`_` scan 跨任务签名误删两项明确红测。
- 初次 green attempt 组合 73/73 通过。
- 单记录矩阵首次失败为新增夹具漏填 NOT NULL kind，修正夹具；随后矩阵通过。矩阵覆盖 8 种记录 × initial/fresh/resume，`total_changes` 无增加。
- 新增真实生产准备/发布函数回归，复用已有 Web startup 夹具与快照；拒绝时整个 publication 事务与本次 owned startup 文件回滚。不是 Tauri 桌面/目标 URL 验收。
- 最终完整 `nice -n 15 cargo test -j 1 --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1` 退出 0：主库 **1516/1516**（1582.41 秒），导入工具目标 **38/38**，main 目标 0 测试。日志 `/tmp/oviraptor-surface-all-targets-all-features.log`。
- 严格全目标/全特性 Clippy `-D warnings` 退出 0，日志 `/tmp/oviraptor-surface-clippy.log`；Rust fmt check 与最终 `git diff --check` 退出 0。
- UI **769/769**，生产构建退出 0，日志 `/tmp/oviraptor-surface-ui.log`、`/tmp/oviraptor-surface-build.log`。
- Native 本机浏览器回环退出 0：匿名 capture complete、8 个 observed requests、comparison complete、身份隔离为 true，日志 `/tmp/oviraptor-surface-native-runtime.log`。仅本地夹具，不是安装态或授权 URL 验收。
- 本轮完整测试中，6 项结果面专项、生产 startup 回滚和残留 hash 登记均通过；各集合不能相加冒充全量。无在途 Cargo 回归进程。

## 体量与残留

`result_ingestion_runs.rs` 123 行；结果面专项测试 202 行；既存 `tests_scan_lifecycle.rs` 675 行仅替换本次 Native 正向夹具的三个字面量，没有堆新职责。新测试复用生产启动夹具，未复制 fixture。

三个残留清单条目逐文件核验后更新 hash/计数/原因。结果准备中的旧字面量仅用于拒绝检测；不是最终清零或支持旧兼容。仍有旧配置启动自动清理、UI stage/source 分支、历史发布文案等待审路径。

## 未完成与风险

- 存在旧记录的同一 scan 将拒绝准备新 attempt，保留旧数据；定向清理工具、精确真实对象盘点和备份/恢复方案尚未交付，需要用户确认后才能执行清理。
- 拒绝检查限定本切口列举的来源，不代表全库旧数据识别、所有恢复入口或通用终态消费均已完整验收。
- 设置初始化仍会持久去掉旧配置字段；没有在本轮顺带改动该独立合同。
- 十维 append-only 账本、全部专家真实执行/监督、聊天闭环、逐路日志、完整安装态和授权 URL 验收仍未完成。
- 本切口不产生外部目标/模型请求，本切口准备函数的目标请求、模型请求与 Token 为 0（其他全量夹具会产生本地测试请求，不计作真实目标验收）；不将其记作产品整体请求/预算守恒验收。
