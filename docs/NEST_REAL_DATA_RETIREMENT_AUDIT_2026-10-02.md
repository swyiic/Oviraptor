# 真实 Nest 旧数据精确清理审计（2026-10-02）

## 授权、问题与边界

用户后续明确允许真实业务记录读写、Nest清理及CAS接触，资产不得删除，覆盖旧“不能碰真实DB”限制。未重复索要已授予许可。盘点发现16个明确旧checkpoint分属3个当前Nest扫描，当前普通删除/结果准备保护会拒绝旧格式，不能以清理旧格式为由删整任务或当前Native JSON。真实库128张业务表另有sqlite_sequence/sqlite_stat1；Native runs8、扫描7、资产及关联各107,558。没有旧process记录，4份已有backend plan为对象schemaVersion1，保留当前/未经证明的源内容，不改写成新权限。

先SQLite online backup保存一致性恢复副本。在线副本继承WAL头，首次只读打开失败；只对备份checkpoint及转DELETE日志模式，quick_check及独立只读验证通过。备份：`/Users/swyiic/oviraptor/database-backups/master-retirement-before-20261002-908ccc6e-b95d-4897-8848-e86c5c20f2a6.sqlite3`，434,225,152字节、0600、SHA-256 `6e18ce4d33687568e530614da75d1d5b2ad6584789f74a38a9f444608abe0af3`。旧启动备份未覆盖。真实库没有做这项journal模式修改。

## 最小工具与证明

`tools/retire_nest_data.py`（258行）只提供显式plan/apply命令，没有启动调用、schema初始化、Legacy reader、格式提升或CAS删除。盘点与应用要求现有普通文件、独立且不为同一硬链接的备份、SHA及完整schema/130表带SQLite类型和rowid的指纹。仅允许七类已知候选表；当前实际仅五表变更。顶层退役配置移除保留所有其他字段值与嵌套用户文档，不进行normalize或补造Native配置。重复JSON键拒绝而不修复。

IMMEDIATE内重新核验源快照和精确actions；SQLite authorizer禁止其他表写入及数据库附加/结构修改，最终全库指纹/行数/foreign_key_check与预期精确匹配后commit。触发器越界、IGNORE、源或备份变化全部拒绝/回滚。提交响应丢失后重试，只有完整最终数据和原备份再次通过才能0写入只读返回，不能把任意变动说成已清理。

`tools/test_retire_nest_data.py`（107行）共10项合同。备份硬链接漏洞红测0/1，版本/同inode拒绝补齐；已提交重放红测0/1，补原backup-derived计划与最终只读证明。其余覆盖资产/Native/nested字段保留、计划篡改、源变化、备份损坏、资产触发器、当前checkpoint破坏、静默少删和重复JSON。最后10/10、0.162秒。最终真实备份的隔离副本通过实际CLI plan→apply59行→同计划只读apply0行，计划文件0600。严格退役登记1/1、0.59秒，只逐项新增两条migration/fixture登记、总49文件，无活路径豁免。

## 真实写入和提交后核验

最终CLI应用 `/tmp/oviraptor-real-retirement-exact-plan-20261002.json` 到真实库，59行精确变更：

- 删除37条旧app_settings。
- 删除16条旧checkpoint。
- 删除4条strix_knowledge_entries及1条strix_skills（旧learning候选0行）。
- 配置1行去除24个顶层strix字段和退役agentBackendPolicy别名；其他列及当前字段值保留。

提交后新只读连接完整重读130表与schema，精确匹配原计划预期；123张非清理表和候选表内所有当前行均保留。资产/关联各107,558、Native runs8，当前候选0、quick_check=ok。所有git未提交工作保护，HEAD仍59be3d86d25adda5b1f975759bf256ef3b94f32b；没有git提交、安装包更新或授权URL访问。

## 证据与限制

源码及本次显式登记JSON共865路径，摘要 `50b8b0c6eb889ca8c835a05dbb1ec93f083ef79145433d6c78c4dc5c492fb425`。前序862路径runtime快照未含登记JSON，本批显式补入该元数据；只有1个既有登记文件和2个新Python文件变化，Rust源码未变。前序373项Rust结果不是本Python工具或最终全量验收；本工具以10项合同、实际副本CLI、严格登记及真实提交后核验为证据。

私有作用域、backup、code snapshot、源码前文本/差异、红/绿日志及真实结果保存在 `/tmp/oviraptor-real-retirement-*`；提交结果及核验为 `...live-apply-20261002.json` / `...live-verification-20261002.json`。公开文档不输出业务正文、凭据或逐行私有资产。

这是已证明的明确旧记录清理，不是所有历史字节清零。CAS59对象和37bundle未证明独占旧格式，全部保留，不碰CAS文件；当前Native报告、backend plan、任务、资产、共享配置和未知源内容都保持。空旧schema表保留作拒绝检测/迁移边界，不赋予运行或格式兼容。

Master尚未完成。继续父监督存活与在途即时阻断、完整预算/恢复/各角色/聊天/逐路实时日志，最后才做完整门禁、当前安装态和授权URL；不将真实数据清理或局部验证当作整体功能交付。
