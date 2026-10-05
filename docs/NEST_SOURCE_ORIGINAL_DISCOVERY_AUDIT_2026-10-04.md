# Source不可变原归属发现审计（2026-10-04）

本批进一步修复Source原终态消费中的原Root丢失识别问题：实际生产入口完成SDK7次、原财务控制/费用与退出存在，仅临时损坏Root目标/计划surface及当前C目标字段，原查找返回空，错误回调误走无Root的failed消费，修改前业务负向0/1。现在查找同时读取不可变原财务控制中的Source scan/attempt/target/policy；只定位原执行义务，随后仍验证原Root/C、冻结材料和原退出，不发执行权、不补Root/租约/账、不恢复旧格式正向兼容。

新增一个具名回归含两场景：已知SDK7次/请求7/输入输出70；无usage SDK1次/请求及输入输出未决1/4406/4406。归属字段损坏时仍找到原Root，精确budget_root_original_owner_conflict，消费与guard退出全部typed rows/rowid不变。临时库恢复原字段后全库原行回到精确前像，正常消费分别completed_with_gaps/partial；原费用保持，闭合重放零写。未把两个场景算成两个测试。

首次修复后exact1/1（编译39.02秒/测试10.64秒）；最终同源码关联18/18（测试31.51秒/阶段32.01秒，选择=报告=通过集合）、严格all-targets Clippy0（15.36秒阶段）、退役46/46（5.43秒阶段，含exact及当前Native JSON原字节回环）、两执行叶局部fmt/diff--check0。关联为Source实际消费3/本批隐藏原Root1/Source quiescence2/工作台admission9/retry2/spawn1；前批50/50和Source结果15全过保留历史，本批未重跑其余集合，不拼接为全量门禁。

3代码路径2已有/1新逐文件前像保护与三个最终增量全文审查。生产只改原Root查找SQL，其他两个消费函数和全部既有测试/helper原字节保持；文件小于400行。1340原范围外源码SHA及HEAD59be3d86保持，1343源码集合SHA 3925a37ee58a58ab1db79f2dc380fb0cd1e36ad9143abd4bb582b9747a09ec5b。Native JSON/allowlist、权限/财务写入规则均不改。仅临时Git/SQLite/CAS/localhost脚本SDK，没有写真实DB/CAS/资产、不自动提交、未安装/外测或修改UI；未配置固定镜像的分析器能力缺口保留，脚本SDK不能证明真实模型脑力或供应商美元对账。

Source付费删除审计尚未完成。代码检查确认deleted_scan_audit/multi.rs要求原Web Multi mode，并显式拒绝agent_source_model_rounds；Source材料/所有权/回执/退出需独立受控审计，不能造Web mode来通过。当前仅证明该实现边界，尚未完成实际Source删除/冷审计验收；保护业务/资产/CAS与原财务来源，真正清理仍先精确盘点备份。其余Source原失败/保护/取消/恢复终态、缺原事实pending恢复及其他14类范围仍保留。

继续先证明真实Source已知付费删除的材料/归属图与拒绝作用域，再实现原Source审计合同和负向；其余删除/残余活路径收口后按Master推进十维动态预算/对账/恢复、六监督、全15角色及真实推理/并行、聊天/逐路日志/整体UI、非Web与知识资产生命周期。框架后才全量门禁、同源安装App实际打开、两授权URL匿名只读及真实模型质量；登录凭据待提供。不删除asset、不绕过InputParser原拒绝。Master未完成，Goal active。

详见[NEST_SOURCE_ORIGINAL_DISCOVERY_AUDIT_2026-10-04.md](NEST_SOURCE_ORIGINAL_DISCOVERY_AUDIT_2026-10-04.md)。

证据：/tmp/oviraptor-source-branch-original-discovery-before.log为实际业务负向0/1（编译38.32秒/测试7.39秒）；after.log为两场景exact1/1；related.log/clippy.log/retirement.log/checks.json/selected.json为最终同源码18/18及检查集合。baseline.json/before.json/prior-diffs、scope-final.json/code-snapshot.json/reviewed-merge-diffs/semantic-review.json保存逐文件范围与前像；docs-before.json及文档preimage/proof保护全部历史字节。仅修改原Root查找，不改两个后续验证/事务消费者。

| 文件 | 新文件 | 行数 |
|---|---|---|
| `src-tauri/src/commands/tests_source_branch_terminal_original.rs` | False | 354 |
| `src-tauri/src/commands/tests_source_branch_original_discovery.rs` | True | 140 |
| `src-tauri/src/commands/native_source_branch_completion.rs` | False | 176 |
