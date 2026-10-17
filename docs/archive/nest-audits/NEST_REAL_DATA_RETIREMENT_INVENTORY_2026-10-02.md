# 真实库旧数据盘点与恢复备份（2026-10-02，已按精确计划清理）

用户已允许业务记录读取/写入、Nest 清理及 CAS 接触，资产不得删除。先只读盘点和保存独立备份，之后按已授权精确计划清理Nest旧记录；未运行schema初始化、更新安装包或操作CAS，资产不得删除。

数据库：`/Users/swyiic/oviraptor/oviraptor.sqlite3`，当前128张表。资产与项目关联各107,558条；8条 agent_runs 全部 backend=native，Nest扫描7条。不能以旧标记存在删除这些当前记录。

已识别并完成精确清理：16条旧 `strix_run:*` / `strix_events:*` checkpoint；37个旧app_settings键；当前一个配置中24个顶层strix字段及1个退役agentBackendPolicy别名；旧知识4条、旧技能1条，旧学习候选0条。这些对应58条删除及1条配置25字段移除，详见正式清理审计。当前Native JSON字段、Native checkpoint/结果、资产关联、嵌套用户文档继续保护。CAS有59条对象、37个导入bundle，尚未证明对象独占旧格式，不删除对象或历史导入。

SQLite online backup：`/Users/swyiic/oviraptor/database-backups/master-retirement-before-20261002-908ccc6e-b95d-4897-8848-e86c5c20f2a6.sqlite3`，434,225,152字节，权限0600。原online副本继承WAL头，首次只读验证无法打开；只对备份做checkpoint及journal_mode=DELETE，形成独立文件后quick_check=ok。最终SHA-256：`6e18ce4d33687568e530614da75d1d5b2ad6584789f74a38a9f444608abe0af3`。真实库没有进行这项journal变更，既有启动备份也未覆盖。

私有盘点与备份元数据在 `/tmp/oviraptor-real-data-schema-inventory-20261002.json`、`/tmp/oviraptor-real-data-inventory-20261002.json`、`/tmp/oviraptor-real-retirement-backup-20261002.json`。内容仅结构、键名、计数和关联，不在文档输出业务正文或凭据。

前期临时实现的8项合同和真实备份副本59行试验之后，工具已固化到 `tools/retire_nest_data.py`（258行）及10项合同测试，补齐硬链接备份拒绝与已提交计划只读重放。最终CLI副本plan→apply59→只读重放0均通过；严格退役登记通过。

真实库已完成相同59行变更：58条明确旧记录删除，以及1条配置去除25个退役顶层字段。提交后重新完整验证130张表（含sqlite_sequence/sqlite_stat1）和schema，精确匹配计划；123张非清理表及清理表内当前行保留，资产/关联各107,558、Native runs8、quick_check=ok，未操作CAS或安装包。恢复备份保留。具体日志、精确范围和未清理未知历史内容见 `NEST_REAL_DATA_RETIREMENT_AUDIT_2026-10-02.md`。这仍不是Master或整体验收完成。
