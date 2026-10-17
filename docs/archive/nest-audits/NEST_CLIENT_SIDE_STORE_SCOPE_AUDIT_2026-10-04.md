# 2026-10-04 ClientSide 业务历史快照作用域审计

仅仓库/临时SQLite/localhost真实HTTP和SDK，未操作真实数据库、CAS、资产、安装或授权URL；NativeJSON和原已有改动保持，无自动提交。使用前批真实新creator/Root/C/UUIDworker/原供应者GET/真实provider到达边界，不用界面模拟或只读诊断代替执行。

问题：两个私有writer全应用capture假定每张表有rowid，并物化所有旧业务数据。真实生产finish之前加入无关WITHOUT ROWID表，SDK原claim失败；在实际provider收到原请求之后创建该表，已付交付失败。首3项1pass/2fail、runtime5.31秒/compile32.56秒，两真正红均报各自physical_schema:no such column:rowid。第三项六个业务INSERT/UPDATE/DELETE跨SDK/已付mailbox边界本来就被原hook拒绝，不声称新修出的红。

最小只改capture集合：SDK允许插入的agent_specialist_calls/agent_budget_entries与sqlite_sequence；交付允许写入的agent_budget_entries/messages/collaboration_events/budget_ledger/runs/assignments/assignment_attempts/capability_leases/lane_leases与sqlite_sequence。完整主/临时schema仍独立hash核验；私有READ_WRITE/noCREATE连接的authorizer在BEGIN前安装，到commit/drop不清空；原IMMEDIATE排斥其他写入，任意其他业务/Source/trigger写入仍被拒绝。可写Native表继续逐个保留全部Root/worker的原rowid/原rawJSON/列/allocator，而非仅当前Root；原费用、原source proof、canonical事件及其他写入后置核验不改。

修后同三项3/3（5.00秒/compile32.67秒）。两业务形态正向现在真的各完成独立SDK/ACK/供应者收尾且表原值保持；原六越权负向仍全部拒绝，检查全应用未触及范围、目标费用、原无发送grant或已付received费用/run/lane/caps保持，不进入补偿重试、不调用第二SDK。先SDK和后provider两时序不互相冒充。

规模用独立临时表107558行、每行512随机bytes，共55069696bytes；逐行流式SHA256含id、长度、原blob，在交付及实际整段finish重入后都一致。SDK和目标各1、只读Clientcompleted且ACK1、finding0；原所有budget_entries在重入保持。原版本该功能1/1（12.02秒），delivery1302ms/replay869ms、RSS283410432bytes；缩小capture后1/1（9.61秒），delivery242ms/replay137ms、RSS50266112bytes。/usr/bin/time -l针对同样test单独运行，runtime还含数据库生成与三次完整digest；未以总runtime冒充实际交付时间。仅单次受控比较，没有设置机器相关脆弱时限，也不外推完整实库/原生历史/安装态性能。

最终当前逐名34个Client合同+原局部Native补证1项=35/35 runtime50.98秒/wall51.58秒，严格all-target/all-featureClippy exit0/wall15.87秒，import-existing-results39/39 runtime2.92秒，exact退役1/1 runtime0.73秒。serial nice15/offline/locked/j1，Cargo运行中不改Rust。其他角色writer未改；前批238/238为前批源码证据，本批不冒充重新跑过238或完整主库。六旧失败名单 /tmp/oviraptor-specialist-replay-proof-unresolved-prior-names.json保持，未ignore/删除。

5路径4已有/1新，1273集合SHA ca016be529e83d91918dd43680c6c6bba3f95c0d72db5e51b672369f9de45b35，1268原范围外保持，HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b，diff--check0。完整原文/原差异/逐路径merge/patchSHA/红绿/单独测量/35名称保存在 /tmp/oviraptor-client-store-scope-*。test patch e5c3d1439db73edb74ee2e8dfcbc912e81941365462ad5a93c5adb3131ae786c，runtime patch f7bf488a8a65cb05aff951a4d7105838ab53331d62e9438d5c38b6bfc222a410。

待完成：更多Native历史规模、实际Browser/DOM影响/候选/独立Reviewer、原Root回放退出凭证的真实证明和余下六触发/通用ReAct、其他角色SDK写权/全工具进程join、动态十维grant/provider精确对账/显式续跑、15角色/Broker/并发、聊天闭环/逐路日志/整体UI、六旧失败/paid删除；最后整套门禁、安装App打开/源码一致、授权匿名URL/模型质量。InputParser原自动审批拒绝保持、不重试或换名绕过，完整原因不可见；Goal工具旧blocked，按用户继续授权开发，不标complete。
