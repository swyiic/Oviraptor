# Root 模型预算及共享额度审计（2026-10-02）

Master 未完成，继续框架开发。只用隔离库和真实 localhost 模型请求；没有真实业务库、资产/CAS、安装包、提交或授权 URL 操作。本审计是框架增量，不能代替全量及整体功能验收。

## 实际问题、作用域与修复

1. 注册 Native Single 模型原走带隐式重试的通用 Gateway，没有自己的原费用控制。实际 SDK 500 红测发出2次，现原 Root 控制UUID+独立dispatch在I/O前提交，one-shot发1次；未知留原债、无child/Coordinator/worker伪造。未知policy原也发2次；现在仅明确single/multi策略可进入对应路径，其余在写账及传输前拒绝。
2. Root私有预算事务的触发器可以写projects并发1次模型请求；单效果红测后用连接级authorizer仅允许主事务向五张预算表INSERT，拒绝trigger写入、其他业务/资产/其他Root、DDL/attach；最后移除authorizer，失败整笔rollback。首次重复projects名称的夹具意外通过，修正单效果后才得到真实red，不能把首次当证据。
3. 活Root行仍可在paused scan初始化费用控制；实际red后，首建和执行都检查原scan/attempt。原控制冻结Native原plan文本/hash、target、origin、硬限额、policy和原Coordinator epoch/fence，C续期不重写证据；历史用量/Root模型或工具历史、部分10维限额不回填权限。首建HTTP/Native checkpoint完整边界留下一批继续证明。
4. 剩余额度1仍将预留缩成1并实际发原请求1次。另实际SDK body没有输出上限。两个red后，完整输入字节+输出估算须能放下，云默认输出8192也真实写入max_tokens，绑定原request hash，保留schema/tools、单次传输和原deadline。第三方忽略请求上限的实际账单仍保留并阻断，不保证外部服务绝不超费。
5. Root已付200+child900能越过原1000；反向child600又因input/out两份envelope被算1200而拒绝合法Root100。已有coarse总额损坏2000还能放大原Root1000。三处真实red后，共用gross函数取原冻结硬限额，有限child逐worker账本与coarse spent+held核验一致；不将两类token预留重复相加。Root按自身每call已付+持有计数，child按整份grant/prefix/terminal语义核算；unlimited已报告扩额有明确不同生命周期，原费用不塞进child projection。
6. 已报告unlimited费用超过估算原转unknown；实际red后仅真实、完整、非负、一致usage允许原费用扩额。另一原owner已unknown时，普通reserve门禁又丢失已知late invoice：Root→child及child→Root双向真实生产财务red后，共用persist_known_cost只接受三token维、Reserve、原limit=NULL，由既有原dispatch/receipt proof调用。普通新执行/槽位/目标/请求预留仍拒绝，其他owner未知债不退、不清、不恢复权限。
7. schema REPLACE原只防call+phase，轮次唯一键和单terminal部分唯一索引两种碰撞均可删除原发票；实际2/2失败后新增完整unique碰撞保护。additive guard不改原Native行或回执、不依赖recursive_triggers，不重写现有guard。

## 数据与执行证明

Root控制表、模型journal及追加账本保留原UUID、round/request hash和费用来源；只保存response hash及reported usage，不复制prompt、完整响应或凭据。已知响应先保存账单，再复核原执行权和全Root未知债，才能使用输出。原Root关闭、same/different Root Coordinator接管后的typed费用仍归原控制，不授权后续动作或新控制接管；此部分是生产财务函数合同，不是已实现Multi Root SDK。

真实在途Single SDK收到请求后暂停scan，在provider响应释放前返回停止；原1次请求保持未知，不重发。已知SDK回执验证dispatch确已提交及实际body max_tokens，精确四维consume/release。缺usage返回停止，token留未知、实际请求已知为1。重复round/回执、错数据库、旧用量、坏部分限额、所有INSERT阶段IGNORE/ABORT、业务与owner触发器、update/delete/replace均验证原费用、业务及权限作用域不变。Root/child未知费用交错证明财务保存与新工作准入分离，不是并行多智能体SDK全场景验收。

## 验证记录

最后代码快照的受影响门禁已收取，不借前序快照或局部测试宣称整体完成。

- 主库1733→1755，新增22项Root合同；最后受影响255/255、exit0（141.42秒），`/tmp/oviraptor-root-budget-final-affected-original-topup.log`，包括22新合同和真实SDK/Source/worker/预算消费者。此前251/251、144.92秒及unique保护后的253/253、143.28秒是前序快照，集合重叠不相加；不代替最后全量验收。
- 最后严格全目标/特性Clippy exit0（15.64秒），`/tmp/oviraptor-root-budget-final-clippy-original-topup.log`；导入器39/39（2.36秒）、严格退役字面量1/1（0.59秒）、作用域fmt/差异及890路径摘要复核通过。Clippy首轮cmp_owned和第二轮bool_assert_comparison均失败，保留原JSON文本比较语义及断言含义修复；这些失败不当通过。
- 错误API/缺测试import等编译失败不是生产red。一次本增量格式命令错cwd报路径不存在，未产生格式修改；后续绝对路径明确格式成功，不把失败格式当通过。此前父批也有格式错误，历史证据各审计独立。

## 文件保护与结构

逐文件before、差异和scope保存在 `/tmp/oviraptor-root-budget-*`。13已有+14新增、无删除；已有共享entries仅改原root参数并加严格财务用途入口，limits抽原Root初始化复用，旧child权限验证不删除。新模块分开Root控制、clock、model、receipt、shared gross、私有事务和SQL；新增手写文件均不超过400行。现有大文件只含include及薄接线，没有复制Coordinator或child权限算法。

基线876路径摘要396f59edd479a8d23e2eb635247469f8b250f57f62186644d77559857e84fcbd；最后890路径摘要1210136b5ad12e14bb8945d0e80fea5833cfaaf32a1093d2d522ad2131f793e5，最后门禁后已复核一致。HEAD仍59be3d86d25adda5b1f975759bf256ef3b94f32b。基线中只有13已声明路径变化，无丢失或作用域外修改；未自动提交。

## 未完成与风险

- 普通Web生产入口仍无条件Multi prepare；本增量注册Single的真实SDK函数合同不证明UI已可选择完整Single模式。Multi Root SDK调用尚未接线；child SDK原路径在受影响集合中验证。
- Single target HTTP、工具新工作/费用/原deadline、browser逐请求Broker及晚到业务输出还未接Root预算；Root最终/idle elapsed费用取样、完整历史/Native checkpoint前置及精确manual reconciliation/dynamic allocation仍未完成。10维限额初建不等于10维全部实际消费。
- finite gross核验覆盖当前prefix/terminal/零I/O替换；unlimited separated coarse spent/held完整来源核验仍需补充，不因读到max(fine,coarse)称全部投影已核实。
- Source/Reviewer安全重派及canonical action、跨Coordinator执行恢复和持久原worker/parent完整矩阵、剩余角色、聊天四态动作、逐路实时日志、最终完整门禁/安装态和两个授权URL继续开发。SRC race adapter default8/max64及无worker/Broker活路径已静态定位，先真实red收紧再完成独立Concurrency闭环。

继续Master，不在局部绿色结束，也不标Goal或Master complete。
