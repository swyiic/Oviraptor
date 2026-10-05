# Source 已知工具耗尽费用结清审计（2026-10-05）

本批为Master A01/A02/A07的一项生产修复，框架未完成，持续Goal active。

## 问题与作用域

签名Source发布、实际Source launcher和原角色/工具入口调用SDK7次。初评2、Mapper工具2、Analyst工具3，均由受控localhost提供原已知usage；Analyst三次实际读取但没有assignment.finish。原Root暂停及扫描/分支partial准确，粗账本却仅spent80/4并reserved266640/3；原细费用为请求7、输入70、输出70。新增原入口断言0/1证明两账本与执行义务没有收口。未借用真实数据库、历史手填Root或财务凭据。

不清理历史数据、不修改当前Native JSON、不增加schema/migration/执行授权，不把已知费用清成0。未知、在途、错归属、失效授权或损坏材料继续保留原义务。旧Root终态/退出不被本函数重写。

## 最小实现

复用原完成审计的完整轮次循环，保留其原字节；完成审计仍要求真实finish。新增独立已知耗尽审计，要求精确3条原轮次、已知原回执和用量、原转录/工具输出/事件/费用/快照一致，每轮都没有finish及拒绝字段。它只返回原Usage，不返回工具权限或成功结果。

实际执行尾部进入同一Immediate事务：核验原当前Source/C/worker授权、原SDK inode闲置并持有到提交；拒绝该子任务的异种执行历史；捕获整个粗账本和本子预留，结清原费用并调用现有失败结束器。最后重验原审计、账本守恒、失败状态、原用量、无成功邮箱、无lane/有效capability。静默忽略或后置副作用失败即回滚。精确已知耗尽错误只在该事务成功后返回，避免通用未知用量清理再次处理已结束子任务。

实际launcher最终为spent140/7、reserved0/0，原细费用不丢失；失败子任务terminal/failed，Root仍terminal/paused，扫描/分支partial。未完成Source私有删除审计精确deleted_audit_original_source_completion_required；公开接口返回原native_paid_audit_retention_required，两路径均全typed rows/rowid零写。

## 验证原始边界

- 新launcher断言修改前0/1：32.10秒编译、2.81秒测试；修复后1/1：44.53秒编译、2.97秒测试。
- 两新增具名测试2/2：39.77秒编译、4.14秒测试。两夹具各SDK3次；初始仅在临时库注入最终settlement静默失败，保留实际三轮费用/工具与原运行义务，不伪造执行事实。
- 8类原闭合事务故障、原SDK忙锁、5类材料/未知状态损坏与撤权拒绝，失败全库typed rows/rowid保持；正确失败闭合后SDK/工具/ModelCost/事件/邮箱原行保持，已知费用30/30/3、粗账本60/3且预留0；重复闭合拒绝并零追加SDK/写入。场景不计作独立测试。
- 扩大首轮33/34（326.18秒测试/355.94秒阶段）。新断言误要求公开错误桥泄露内部原因；不改原接口、不放宽生产审核，增加私有精确原因验证及公开保留提示验证后exact1/1（18.12秒编译/3.12秒测试）。失败日志保留。
- 最终同源码关联34/34：324.93秒测试、325.43秒阶段，选择/报告/通过集合相同。含原入口、Source已知/未知付费删除/冷审计、Source轮次及在途、实际暂停/财务退出、保存finish/截止时间恢复。不是全项目2329项门禁。
- 最终严格all-targets/all-features Clippy exit0，8.92秒；退役过滤50/50，4.97秒，精确集合相同，含literal登记及当前Native JSON原字节回环；四执行/测试叶局部rustfmt--check和范围git diff--check exit0。

证据前缀：/tmp/oviraptor-source-exhausted-*。red、green、negatives、first-final、assertion-check、final、clippy、retirement日志和选择/结果JSON保留。一次补丁check失败未修改源码，误接续的旧测试再次失败也不作修复证据；其后依赖操作分开，正确补丁检查及应用均0。

## 差异与数据保护

| 路径 | 本批变化 | 行数 |
|---|---|---|
| agent_runtime/multi_agent/source_rounds/completion_audit.rs | 共享原轮次核验，新增只读耗尽Usage证明 | 159 |
| agent_runtime/multi_agent/source_rounds.rs | 单条export | 320 |
| commands/native_source_tool_execution.rs | 实际尾部原已知失败结清，避免重复未知清理 | 214 |
| commands/native_source_exhausted.rs | 新增原授权/退出/财务守恒失败事务 | 72 |
| commands/tests_source_branch_terminal_original.rs | 仅新增一个已知账本/删除拒绝断言调用 | 355 |
| commands/tests_source_exhausted_original.rs | 新增两真实SDK生产者负向及共用断言 | 281 |
| commands/tests.rs | 仅新增include，旧清单保持 | 123 |

7路径5已有/2新，前像、原Git差异与最终增量逐文件保存并审查。1341原范围外源码SHA及HEAD59be3d86不变；1348集合SHA f1ba5daf2629addde84f28f1600452ee14b5a85c60a0b38f4443a85a49397169。原完成核验循环和原launcher测试/helper字节保持（只多一断言调用），rounds模块仅export、测试清单仅include。没有删除测试、ignore、真实业务/资产/CAS写入、自动提交或安装/外测；UI本批不改。三个进度文档前像及SHA另存，全部历史字节保留。

## 继续与限制

本批只证明新鲜实际执行的已知三轮耗尽结清；它不等于Source整体失败/取消/恢复完成，失败/pausedSource冷删除仍拒绝。ToolPending恢复错误分支当前保留活Root，须补真实耗尽恢复生产者后收口，不能以当前测试代替。其他取消、原事实缺失、历史attempt、Code/Greybox、人工及其余角色仍未完成。

十维动态分配/全通道对账及真实供应商美元、六类Root监督、15角色/General ReAct/真实推理、真正并行、用户聊天、逐路实时日志、整体UI和完整门禁/安装App打开/授权URL仍按Master剩余推进。脚本模型和分析器回调不证明真实模型质量或真实Docker分析器。真实清理继续先盘点备份，不删除资产。
