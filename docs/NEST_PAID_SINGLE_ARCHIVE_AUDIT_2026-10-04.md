# 普通 Web Single 付费物理删除与原来源审计留存 — 2026-10-04

## 结论与边界

REM-A01 本批实现普通Web Single在原实际执行退出、全部已知执行义务/十维费用确定后，原子留存任务原来源并物理删除scan/run。只证明受限普通Single范围；Master14项均未完成，Goal active，不暂停/不标complete。本批未执行真实数据清理、CAS/资产操作、安装App、外部URL或真实模型质量验收，不自动提交。

原来源保存在新的不可变 `native_deleted_scan_audits` 中；原财务、模式、SDK日志表继续原行保留，不删除防篡改trigger。当前仅内部只读核验，没有归档公共原文API或UI；保留数据不用于续跑/补造C/预算/SDK/目标许可。Native JSON当前黄金导入路径保留，旧格式与Strix活运行时未恢复。

## 原问题与真实范围证明

初始正向夹具通过实际普通新URL创建/启动/HMAC、原冻结Root、真正SDK/费用、runner finally、owned target结果消费、释放原target owner、实际 `finish_native_branch` partial闭合；十维reserved/indeterminate均0，但原删除仍拒绝，实际0/1（4.67秒）。没有将旧completed标签当执行证明，也没有用只读诊断或mock UI代替执行。

第一版资格检查误以为run ID必须UUID，实际creator出生ID为 `plan-{scan}-{attempt}-{target_hash}`，错误0/1（2.97秒）；核对原producer后去掉错误ID格式要求，保留原Root/财务/退出/发布核验，首次真实物理删除1/1（3.21秒）。随后真故障1/3（9.45秒）：原SDK72条日志未通过owner FK入留存，且audit INSERT触发器能改业务。加关联行闭包及私有writer边界后3/3（12.73秒）。

进一步8/10（50.78秒）：隐式 `REFERENCES sentinel_scans` 未解析主键，真实能级联删业务；已按原PK解析并拒绝受保护记录。另一失败是新夹具用了不存在的token维度名、读到0；改用原DIMENSIONS前三维，未改财务生产逻辑。修后10/10（50.32秒）。原冻结财务控制但SDK/exit尚无，仅腐坏旧run标签且实际branch闭合的负向真0/1（0.30秒）暴露漏查；删除保留判据现包括本scan的原Root budget control，不能靠旧标签清除原来源。

当前正向是一个真实普通Web Single target/attempt的已付partial退出，不宣称成功漏洞发现、全15角色、真实目标HTTP闭环、全部多attempt或非Web分支删除。原SDK/HTTP退出验证复用真实原inode的probe，不CREATE，保持到事务结束；原执行/费用未知、未发布、in-flight、源损坏、pending branch/指令/worker均保留任务。已有Single/Multi拒绝回归仍过。

## 最小实现与数据边界

- 原删除的私有connection/IMMEDIATE事务、quiescence、所有权、清理、退休数据、人工结案、后继任务完整行检查继续生效。只有不可构造/不可clone的Prepared经过原财务/原publication核验，才使原Single `terminal`成为可删除状态；不是自由bool或旧标签授权。
- 无损保留列序、物理rowid、Null/Integer/Real位模式/Text/Blob原SQLite值。Native表按本scan/原root及声明FK闭包捕获；包括原scan/run/attempt/target、plan/mode/request/fee/exit/events/snapshot/checkpoint与原SDK owner/rows/gaps。非Native业务/asset/CAS不进入留存；关联会被级联或SET NULL且仍有受保护记录时拒绝。
- 原15类财务/模式/SDK来源，原行/rowid与JSON字节前后相同。未归属Native行另存事务内比较范围，防止跨任务改写。business WITHOUT ROWID且BLOB含00/FF的正向原样保留，不作为可读rowid表假设。无PID信号、没有删除证据文件/目录/真实资产/CAS。
- Writer仅在删除命令原私有connection中装一次并持有到COMMIT/rollback/close；直接操作限定audit/tombstone INSERT、原task前继链接更新、声明FK清理/对话选择SET NULL。触发器业务/跨任务写、schema写与原账单写均拒绝。非Native受保护关联先按原FK/PK精确检查，不复制业务内容到归档。
- 留存与墓碑、删除同事务，exact INSERT/readback/原财务/未归属行及旧删除后置检查全部在提交前。audit自身UPDATE/DELETE/REPLACE禁止；重试只核对原持久结果，不刷新墓碑、不重复审计/SDK/费用。
- 删除后在私有in-memory还原列和原值，设query_only，复用原财务exit/十维守恒/原publication的run、snapshot、事件、collaboration与source证明；不执行存储schema、trigger或SQL，不返回Connection/RootOwner/活能力。留存hash重算也不能隐藏原run损坏；原活库原财务/模式/SDK锚点继续精确比对。

## 新增14项与故障证据

四个模块测试文件总计14项，均入148项逐名相关门禁：正常task/run消失且9类原财务行及文件/BLOB保持；SDK72条原日志与rowid保留；audit触发器业务/他任务写；IGNORE三位置、删除后业务写、跨任务目标DELETE；原SDK owner在用/原inode缺失且不补造；Native外来scan别名及保护业务FK；显式/隐式FK保护；原审计不可变与改run并重算hash拒绝；已删审计缺失重试拒绝；真实SDK去掉usage仍消费1次model_requests并保留三token维度未知；原控制尚无退出时旧标签拒绝；删除后实际启动、冻结、open_run及SDK producer拒绝，SDK0/全行保持。

两项使用真正第二进程/同一编译测试binary/同一临时SQLite：在写audit/墓碑/删除并完成所有后置核验、COMMIT之前进入仅cfg(test)的精确绑定屏障，父进程确实kill子进程并wait，全部原行与Native JSON恢复，之后正常删除；另在实际commit成功后故意让子进程丢成功回复，父进程先验证持久删除，再幂等重试，全部剩余行/费用/文件不变。不是用SQL改标签模拟崩溃，也不是对真实运行PID发信号。子进程入口复用两个实测用例，不增加默认空过或ignored测试。

## 门禁与工作树保护

最终统一执行 `nice -n 15`、offline/locked、单Cargo/单编译job、all-features、test-threads=1；所有源码修改完成后统一复跑。首两次严格Clippy分别只报新pragma match冗余guard、测试屏障比较中临时owned String；按原语义修正，不allow/ignore，后续门禁完整通过。

| 检查 | 结果 |
| --- | --- |
| strict Clippy all-targets -D warnings | exit0，17.46秒 |
| lib no-run / 精确名册 | exit0，2263已编译项；名册真实包含本批148 |
| affected exact集合 | 148/148，147.42秒；实际通过集合等于所选集合 |
| import-existing-results | 39/39，2.96秒（含构建wall50.45秒） |
| exact退役literal | 1/1，0.78秒 |
| 删除Vue组件检查 | 10/10，合成IPC/组件检查，原夹具readyOpportunityCount警告保留；不当安装/完整UI验收 |
| Git diff --check | exit0；没有提交/重置/整批覆盖 |

原范围17路径：7已有/10新；每个已有文件先存完整原文/既有Git diff，所有新范围变更前extend；逐文件读增量差异。1295源码集合SHA `11b8682327c14e6285bcf34faf0d2f7baf40c4c6ba266c00ea3ef603cb68b2f5`，1278原范围外SHA不变，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 不变。新Rust模块/测试文件36—253行，不以批量格式化已有文件覆盖用户内容。

门禁原日志/命令/名册：`/tmp/oviraptor-paid-single-archive-final-r3-{gates.json,affected-names.json,compiled-test-list.txt,clippy.log,compile.log,affected.log,importer.log,literal.log}`；源码原文/Gitdiff/增量/集合：`/tmp/oviraptor-paid-single-archive-{before.json,baseline.json,prior-diffs,reviewed-merge-diffs,scope-final.json,code-snapshot.json}`。本记录与Master/progress/handoff另存原字节/预期SHA/差异校验，历史正文保留。

## 尚未完成与下批优先级

1. Multi paid任务物理删除仍不允许；两个原Tick/时间线表的run RESTRICT未改，原worker/SDK/费用/elapsed/final projection的全部可独立核验来源要先证明。不能删除trigger/账单或保留可活用run假造删除。
2. 受保护业务/Source等关联仍拒绝，尚无完整保存/解绑合同。多个原attempt的source checkpoint被后来同URL写覆盖时不能补造原源；缺原证明继续拒绝。并未证明全部多target、多attempt、Code/Greybox/CI生产路径。
3. 当前v1全列原值核验可能在未来schema扩列后拒绝，未解决历史版本迁移；真实大库性能/内存、超大Native row、非标准Native表/模式、全部FK图边界待验证。事务内未归属Native行只读比较会随库规模增长，未做真实资产库性能宣称。原auth材料仍留私有原DB，未新增原文公开接口；归档只读展示与脱敏UI待接。
4. 本批只核验原已关闭SingleSDK/HTTP能力与重复/崩溃边界；不是全进程/全部后代join、全量Rust/UI/build/fmt/Native JSON黄金、安装App打开、授权URL或真实模型质量验收。Master剩余动态budget/grant/精确对账/显式续跑、六监督触发、15角色/GeneralReAct/Broker/真正并行、聊天/逐路实时日志/整体UI和已知旧失败继续执行。
5. InputParser原自动审批拒绝保持，完整原因不可见，不改名、重试或绕过。无真实DB/CAS/资产/安装/URL操作，真实清理仍先盘点/备份/按已有用户授权边界执行；Goal active。
