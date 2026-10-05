# 原终态与 elapsed 事实私有连接审计 · 2026-10-04

Master仍开发中，仅临时SQLite/localhost相关SDK及原实际共享结束入口，无真实DB/CAS/资产、安装/授权URL或提交。

实际Root结束调用抹掉调用方既有projects UPDATE拒绝钩子，结束后业务写从拒绝变允许。首负向0/1（0.46秒）。最初仅隔离closure writer仍0/1（0.40秒）：直接前置elapsed_fact writer即使无新fact也会替换并清空原hook；读实际代码后将该直接前置producer一并隔离，1/1（0.44秒）。

两独立原writer各自从调用方现有main数据库path以RW/no-CREATE打开私有连接，先拒绝开放调用方事务和无文件数据库，FK ON/synchronous FULL，然后原authorizer贯穿BEGIN、原scope验证/事实或费用发布、COMMIT/ROLLBACK直至私有连接Drop。调用方连接、hook、未提交事务保持，原terminal writer和仅INSERT elapsed fact writer权限范围均未扩展；无新schema/identity/grant/ACK/SDK或Native JSON变化。

新增先5/5（2.72秒），实际正常Root结束/原未发出child释放、失败回滚全行保持且caller hook仍拒绝；自然原hard deadline过期的原fact及Root使用同一cutoff；actual入口拒绝caller事务且原未提交row仍在；两writer拒绝memory DB且旧行不变。

首次扩大127有125过/2失败（206.43秒）：caller的TEMP冒名emitter未被私有连接看见，旧拒绝合同实际退化；旧锁竞争探针绑定caller busy callback，而私有连接不会调用它，250ms也不足以等待原held writer。保持原合同：closure初始与COMMIT前只读核caller exact五emitter/无TEMP shadow，私有事务内仍锁下前后核main exact emitter；两writer等待改应用原db::open既有10秒，不继承caller callback。旧fencing测试改实际ready/done回执、持SQLite写锁350ms证明未提前返回，释放后原stale C拒绝且Root/directive仍running/accepted，不以caller callback作私有连接探针。未删除或弱化fencing/schema结果断言。

修后相关7/7（3.91秒）；补回调期间在caller新增TEMP冒名的实际负向1/1（0.46秒），提交前核验拒绝、全部原main应用行回滚，TEMP故障仅临时fixture。

最终128逐名单次全部通过（199.91秒），为原82+新6+40原金融/elapsed/Source退出相关合同。不以首次125/127或分批结果当最终全通过；不是旧九E2E/整个Source角色质量/完整Master门禁。前端未变，不重复UI/build。

- clippy：exit0，wall15.67秒；`/tmp/oviraptor-terminal-writer-isolation-final-2-clippy.log`。
- affected：exit0，wall200.53秒；`/tmp/oviraptor-terminal-writer-isolation-final-2-affected.log`。
- importer：exit0，wall31.36秒；`/tmp/oviraptor-terminal-writer-isolation-final-2-importer.log`。
- literal：exit0，wall1.24秒；`/tmp/oviraptor-terminal-writer-isolation-final-2-literal.log`。

5路径4已有/1新，1214集合SHA `036b4caaef20cc49de8d2a91a673e27c9b3d0201ca3caf34367849ab697d6965`，1209原范围外字节保持；HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`、diff --check0。逐文件before/prior diff、先extend范围、SHA/check/apply及最后合并差异保存`/tmp/oviraptor-terminal-writer-isolation`前缀。

未完成/风险：此处保护原调用方连接权限和原结束/elapsed事务，不证明在途SDK/进程全部join，不提供跨Coordinator账本采用、精确provider对账或续跑。继续在途/晚费用与唯一静止终态、原paused/过期/换C/重启清理、动态十维grant/精确对账/显式续跑、六触发/15角色/General ReAct/Broker/实际并发、整体UI/旧九E2E/正常paid删除，再完整门禁/安装app打开/已授权URL。InputParser既有自动审批拒绝未重试或绕过，完整理由未提供不猜测。Goal active。
