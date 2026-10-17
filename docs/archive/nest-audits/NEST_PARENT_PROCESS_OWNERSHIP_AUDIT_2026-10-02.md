# 父服务 OS 独占审计（2026-10-02）

Master 未完成，继续开发。只使用隔离库与真实测试子进程；无真实业务库、资产/CAS、安装包、提交或授权 URL 操作。

## 问题和最小修改

同一 Root 可以同时创建两份 WorkerSupervisor，双方都有续租和撤权职责。新增生产入口负测实际0/1，0.14秒：`/tmp/oviraptor-parent-ownership-red.log`；修复后1/1，0.14秒。

把现有 NativeInvocationOwner 原实现完整抽到共享 runtime/execution_owner.rs；保留原文件命名、JSON framed hash、规范DB路径、0600、NOFOLLOW、常规文件检查、错误码和不删除锁 inode 的合同。原 commands/native_invocation.rs 仅使用共享实现并保留结果持锁类型；前台调用和后台父确有两个相同语义使用方，没有复制锁策略。

父先只读核验原 Native/multi Root、当前 Coordinator 和原 clock，再领取 scan/attempt/Root 的 parent-supervisor OS 锁，之后才允许首次续租/撤权。锁不随 Coordinator epoch/fence 改名，新控制代不能与尚未 join 完的旧服务重叠。失败构造自然释放；Drop 先撤活、stop/join，再释放锁。进程强杀由 OS 释放，不产生费用回执、任务重派或权限恢复许可。

## 实际合同和门禁

新增5项（包括普通全量中零工作的子进程probe，不能单独算强杀验收），主库1728→1733。独占同进程、规范DB别名与新 fence、失败启动回滚并释放、真实进程竞争/SIGKILL释放均覆盖。实际子进程确认ready后才强杀，严格要求signal=9，无Rust Drop；强杀前后完整表数据快照保持不变。

- 新旧 invocation 合同14/14，1.90秒；包含原前台 branch barrier、隔离路径、重复调用不触发失败清理和跨进程合同。
- 最后扩大207/207，127.96秒：`/tmp/oviraptor-parent-ownership-final-affected.log`，包含原父存活/续租、worker/预算/Source、scan quiescence/deletion和branch调用者。集合重叠，不相加。
- 最后严格全目标/特性Clippy exit0，25.63秒；导入39/39，2.39秒。7文件scoped fmt、git diff --check、严格字面量1/1（0.65秒）和876路径最后摘要一致，不冒充整体功能或全量验收。
- 一次格式命令使用了错误cwd，输出文件不存在；没有产生格式修改，随后的明确仓库cwd格式命令成功。失败命令不算门禁通过。

## 文件保护

4已有+3新增，无删除；已有runtime/mod22、supervisor218、测试入口59、commands/native_invocation9；新增共享OS实现77、parent_ownership73、真实进程合同111行。原74行实现搬迁保持语义，只提升共享可见性并加imports；不改历史JSON、数据库schema或费用。

基线873，当前876路径摘要`396f59edd479a8d23e2eb635247469f8b250f57f62186644d77559857e84fcbd`，`/tmp/oviraptor-parent-ownership-code-snapshot.json`；HEAD仍`59be3d86d25adda5b1f975759bf256ef3b94f32b`。逐文件修改前文本、差异和作用域保存在`/tmp/oviraptor-parent-ownership-*`，未覆盖其他未提交改动。

## 未完成

这是父服务跨进程独占及强杀锁释放证明，不是安装App强杀、远端独立worker通知或全任务恢复矩阵。旧捕获Weak凭证仍失效；新父/new ticket能否借用原worker的完整持久化归属仍需实际生产路径证明。状态页仅持久租约仍不能说明父服务健康。

继续Root/Single十维预算及精确对账、Source/Reviewer安全重派与canonical请求、跨Coordinator财务恢复、剩余角色、聊天动作和逐路实时日志；最后完整门禁、安装态和两个授权URL。局部绿色不标Master或Goal完成。
