# 2026-10-01 App 启动崩溃修复

用户报告编译成功但仓库 App 与 `/Applications/Oviraptor.app` 均无法打开。提供的 incident 为 `ADB61560-411A-4794-B5ED-729831C26C6D`：macOS 主线程在 Tao 启动回调中 Rust panic 后 SIGABRT。两个既有包的二进制 SHA-256 均为 `98f580878d6f23e129be0af50855a1b56c29109a05485fb07ceb0d7a462101fc`。

## 作用域与授权

用户本轮明确允许读取业务记录、写入、清理 Nest 和访问 CAS；资产数据不能删除。这覆盖此前禁止访问真实库的限制。本修复不清理任何记录或 CAS。诊断先只读，真实库先用 SQLite online backup 保存一致性备份，然后在隔离副本验证。

- 原库：`/Users/swyiic/oviraptor/oviraptor.sqlite3`；quick_check 为 `ok`。
- 资产和项目关联各 107,558 行，项目 10 行。
- 可恢复备份：`/Users/swyiic/oviraptor/database-backups/startup-before-20261001-d5a5fb42-14d7-4cb0-9dd0-c97843105515.sqlite3`。
- 备份大小 433,897,472 bytes，SHA-256 `1e63cbf36ddca506af1c39385455290ee765c88f3e5326995f6012997f4ac52d`。
- 原安装包备份：`/Users/swyiic/oviraptor/application-backups/Oviraptor-before-startup-20261001-d68ddd4c-9b81-4111-843c-a75439a9da13.app`。

## 证明与最小修改

真实库 `agent_messages` 尚无 `to_run_id` 等六个 orchestration 字段，且不存在 `idx_agent_messages_recipient`。初始化先执行基础 SCHEMA，其中该索引使用缺失列；补列迁移尚未执行，初始化即失败。Tauri 2.11.5 在 Ready 回调遇到 setup 返回错误时 panic，macOS 外部回调不能 unwind，形成 abort。

新临时库回归复现 `no such column: to_run_id`。从基础 SCHEMA 移除此索引的提前创建，继续由已有 orchestration 迁移在确保列存在后创建。同一字段/索引仍存在，原消息内容与时间、二次初始化幂等性均保持。没有删除或重建消息表，没有修改当前 Native JSON。

## 开发门禁与后续验证

- 红测：`/tmp/oviraptor-startup-schema-red.log`，1 项确实失败于缺列索引。
- 绿测：`/tmp/oviraptor-startup-schema-green.log`，数据库 16/16。
- `/tmp/oviraptor-startup-clippy.log` 严格全目标全特性 Clippy 退出 0；`startup-fmt.log` 格式检查通过。
- 发布构建 `/tmp/oviraptor-startup-fixed-build.log` 退出 0，修复后的 App 签名核验通过。安装版更新使用 staging 与可回滚的原包移动，仓库包与安装版修复后 SHA-256 均为 `2d6800d4165f2838a406ac335440cc081ee8733f1fb2272c98c27a4ee3da653b`。
- 隔离真实副本已启动并实际显示主窗口和 Asset 仪表盘。macOS 弹出上次异常退出的窗口恢复提示，选择不恢复旧窗口后进入界面；仅进程存活不能作为可打开证明。副本 `/tmp/oviraptor-startup-copy-preservation.json` 校验 18 张资产相关表全部内容摘要不变，补列和索引均已生效，quick_check 为 `ok`。
- `/Applications/Oviraptor.app` 使用真实普通目录已启动并实际显示 Asset 列表（当前 Web 人工队列 19,669 条，是筛选数，不是数据库总数）。`/tmp/oviraptor-startup-live-preservation.json` 校验 18 张表全部摘要不变、107,558 条资产及项目关联完整，quick_check 为 `ok`，新 recipient 字段/索引已生效。App 留在真实资产界面供用户使用。
- 旧 DMG 已另外备份，`/tmp/oviraptor-startup-fixed-dmg.log` 对已有发布二进制重新 bundle 退出 0。新镜像只读挂载核验通过，签名有效；镜像、仓库 App、安装版三者二进制完全相同。DMG SHA-256 为 `208f05de9bf7ea16a27f89cfdca28c089c251917b77741760f4e2c54c94dbd4a`，回执 `/tmp/oviraptor-startup-fixed-dmg-receipt.json`。只打 DMG 时 bundler 会清理中间 App，已从验证后的安装版恢复仓库 App 路径并重核三者一致。

这是用户当前启动故障的修复，不是 Master 全部完成，也不是全角色、聊天、实时日志或授权 URL 验收。
