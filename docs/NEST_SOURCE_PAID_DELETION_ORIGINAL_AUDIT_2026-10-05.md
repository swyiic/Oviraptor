## 2026-10-05 Source原付费删除/冷审计收口与完整剩余任务

本条为最新进度，所有下方旧文保留历史原字节。已按用户要求更新完整十四类剩余范围，Master未完成，持续Goal active。

本批以签名Source发布、实际分支领取和Source SDK/工具/独立覆盖/原退出证明重现已知付费任务被deleted_audit_original_web_mode_required拒绝，业务负向0/1（之前两次盘点查询列名错误仅属测试编写失败，日志保留）。新增专属只读Source审计：核验原C/十维结清/原退出、冻结材料和接受修订、角色/工具/邮箱/独立候选与覆盖审核、原分支报告和Source闭合事件；CI使用既有历史只读政策读取器，执行读取器不放宽。按精确表名纳入Source材料，导入修订只取冻结结果接受的原revision，受保护业务/asset/CAS不扩入删除权限。

原Source父调用锁由只含实际原inode所有权的私有类型，从原quiescence顺序移交审计；按数据库规范路径/scan/attempt取用，随所有SDK实锁持有到COMMIT，不创建缺失原锁、不把布尔声明作为退出证明。Web/Single原退出拒绝顺序及精确断言保留。独立准备入口仅test构建使用，不新增执行权、续租、账本写权或历史reader。

新增5个具名开发回归：真实SDK7次的原已知费用任务删除后保留原财务/rowid，重开库仍冷审计且删除重放零写；实际SARIF回调/原导入器、Mapper/Analyst和两类独立审核SDK8次，原接受修订与临时CAS全部文件字节保持；SDK503无usage一次，未决请求/输入/输出1/4406/4406保持且拒绝删除；七材料/回执/CI/闭合事件损坏及三类原锁占用零写拒绝；受保护Source业务FK、审计/anchor/delete静默忽略及恶意业务/asset触发器均拒绝并全库typed rows/rowid保持。没有把多个场景算成多个测试。

验证阶段精确区分：新增首轮5/5（223.39秒测试），旧关联扩大首轮45/48（164.19秒，三个精确退出锁错误被后续财务拒绝盖住）；不改原测试，恢复原检查顺序和实锁移交后53/53（336.35秒测试/384.07秒阶段，选择=报告=通过）。其后只将独立prepare标为cfg(test)及三个执行叶格式化；最终关键5/5复验（96.89秒，前述三个原精确拒绝+Source有/无接受候选两冷审计，集合与53重叠）、严格all-targets Clippy0（18.79秒阶段）、退役过滤50/50（5.33秒测试/30.31秒阶段，含exact登记及当前Native JSON原字节回环）、八叶局部fmt/diff--check0。Clippy首次unused prepare失败已按test专用职责修复，不抑制警告。前批18/50/Source结果15等仅作历史，不拼接为当前全量。

12代码路径9已有/3新逐文件前像与最终增量保护，既有Source完成/执行CI gate函数以及其他pause/deletion helper原字节保持，所有修改文件小于400行。1334原范围外源码SHA及HEAD59be3d86保持，1346源码集合SHA 0d435f59f5aa8954df65e00b8ed08eabb8c78f9c487938fc988034997f6ee0eb。Native JSON/allowlist、UI零修改。只用临时Git/SQLite/CAS/localhost和受控分析器SARIF回调，没有写真实DB/CAS/资产、不自动提交、未安装或外测；脚本SDK不证明真实模型脑力/供应商美元对账，回调不等于真实Docker分析器。

剩余风险：本批实际覆盖CICD/保留覆盖缺口的成功原终态，不代表Source整体完成；原失败/保护/取消/恢复、历史attempt/其他角色/人工合同、Code/Greybox及真实沙箱、业务关联规模/未来schema和只读审计UI继续待完。冷审计仍依赖保留冻结view文件，缺文件拒绝，不自动重跑或补凭据。继续收口这些删除/残余活路径，再推进十维动态预算/对账恢复、六监督、全15角色/真实推理与并行、用户聊天、逐路日志、整体UI及数据知识。框架后才完整门禁、同源安装App打开、两授权URL匿名只读及真实模型质量；登录凭据待用户提供。真实Nest/CAS读写已授权，真实清理先精确盘点备份，不删除asset，不绕过InputParser原拒绝。


本批代码路径：

- `src-tauri/src/commands/tests.rs`
- `src-tauri/src/commands/tests_source_paid_deletion_original.rs`
- `src-tauri/src/agent_runtime/deleted_scan_audit.rs`
- `src-tauri/src/agent_runtime/deleted_scan_audit/multi.rs`
- `src-tauri/src/agent_runtime/deleted_scan_audit/scope.rs`
- `src-tauri/src/agent_runtime/deleted_scan_audit/writer.rs`
- `src-tauri/src/agent_runtime/deleted_scan_audit/rows.rs`
- `src-tauri/src/commands/native_source_completion.rs`
- `src-tauri/src/agent_runtime/deleted_scan_audit/source.rs`
- `src-tauri/src/commands/scan_quiescence.rs`
- `src-tauri/src/commands/scan_deletion.rs`
- `src-tauri/src/commands/tests_source_paid_deletion_original_negative.rs`

原始证据位于 `/tmp/oviraptor-source-paid-original-*`：baseline/before/prior-diffs、initial/production/lifetime/candidate-negative/original-order patch及各阶段日志/结果、final-selected、semantic-review、scope-final/code-snapshot、reviewed-merge-diffs与文档前像。各阶段失败保留。全项目Rust/UI/build、安装包/IPC与授权URL本批未执行。
