# 2026-10-04 Single 原请求与财务来源的删除保留

本批只收口 Single 删除保护和拒绝原因，**未实现完整付费任务物理删除，REM-A01 与 Master 仍未完成**。不把归档、安全拒绝、局部回归或 readonly 盘点当功能验收。

## 原问题与精确作用域

沿真正普通 Single creator、SDK、runner finally 和 owned outcome 消费执行。十维 reserved/indeterminate 均为0、原 run 真 terminal；初始删除正向0/1被 quiescence 拦住，因为测试仍持有真实 target owner。该检查是正确的，修夹具释放原 owner 后真正0/1（3.38秒）返回旧“未结算”诊断。故意旧 completed 标签另跑真正0/1（3.13秒）仍是“未结算”，**本批没有证明它当时实际删除了付费任务**；其他 Native 执行义务也不能从十维余额推断为关闭。

源码与 schema 证明另一合同缺口：Single 的 immutable model journal、financial exit、projection 没有指向原 run 的 RESTRICT，但它们的原核验需要 agent_runs、事件、快照及 native source 行。仅放行 terminal 或仅保留凭证表，再级联掉这些依据，不构成完整审计留存。当前 Multi 的 Tick/时间线保留检查不足以说明 Single 同类记录为什么不能删除。

最小修改原 IMMEDIATE/FULL 删除事务：沿原 retired-data、状态和全部 worker/cleanup 核验之后，在 tombstone、解除子链接和级联前，增加原 model journal、Single financial exit、projection 各自与本 scan 的 run 关联 EXISTS。保留原错误前缀 native_paid_audit_retention_required，中文改为“原始请求或财务审计凭证”，能准确覆盖0 SDK的财务退出。查询失败保留任务；不写新审计表、不改原状态、费用、限额、不可变 trigger、外键或 schema，不删除原文件。

## 实际回归与检查结果

新增6项均为临时 SQLite/localhost。脚本 provider 验证实际 SDK 运输/计费和生产接口，**不证明真实模型推理质量或安装 App 功能**：

- 原 SDK/原 finally/原 outcome 消费后的原 terminal，以及测试故意旧标签：原全应用物理行、Native JSON证据文件和账单保持；重复删除仍准确拒绝。原脚本实际调用12次，模型请求消耗与实际 calls 相等，不误记为1；十维已结清不等于全部执行义务关闭。
- 真 target owner 仍存活时，财务退出不能代替 quiescence；真实释放后仍保留原审计来源，全行保持。
- 已付 Single 并不误挡另一 draft 的正常删除；除指定 draft/墓碑外全表保持，重复成功不刷新墓碑，原 owner 仍可只读核验。
- 实际普通 creator 与单次 native_model_transport：原 fee60、SDK1，尚无 financial exit 或 projection。仅原 journal 已足够要求保留，全行保持，不重发。
- 真普通 Single 原模型配置提前失败：SDK0、原 financial exit1、projection0，保留原财务来源。该结果只证明财务退出入口，不冒充成功模型执行。

最初4项4/4（11.90秒）。扩大80首79/80（62.99秒）；唯一新夹具误以为 run_agent_target 返回时尚无financial exit，实际finally已经保存原退出。修为实际单次SDK入口检查尚未退出/未发布的阶段，没有弱化生产条件。另有前置夹具误记请求1而真实为12、Cargo多selector位置和编译名单脚本错误，均不能算产品功能Red。

最终同一源码逐名80/80（60.34秒）、strict all-target/all-feature Clippy0（15.76秒）、编译2249项、导入39/39（3.06秒）、exact退役1/1（0.77秒）。选择名单与实际passed集合完全相等，无忽略/空选择；运行时只一个 Cargo -j1。日志、名单和命令见 /tmp/oviraptor-single-paid-retention-final-r2-*。不是完整Rust/UI/Native JSON黄金门禁；四个具名旧失败和其他E2E债继续保留。

## 未提交保护及下一项

本批修改前逐文件保存当前原文、已有Git差异和代码集合；3代码路径、2已有/1新，1285集合SHA d8bf63eb177fa005014332229b2db80859629b4e2d6e191952e0122751405280，1282原范围外保持，HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b不变，diff--check0。三份最终增量差异逐文件阅读。

下一项仍是独立可核验的原财务/执行来源留存与任务删除合同：精确列出原run/物理rowid/请求/费用/退出/事件/快照/source依赖，证明完全退出/确定费用后才删除任务关联；原凭证不退款、不重发、不补造，不从归档重新授予执行权限。要有原子性、崩溃、commit未知、幂等、越权/IGNORE故障与Native JSON黄金回归。现有paid Single/Multi仍须保留原任务，不能发布“已支持完整删除”。

无真实DB/CAS/资产/安装/URL操作；不提交。Master顶部与progress/交接已同步，14项保持未完成，Goal active。InputParser原自动审批拒绝保留，完整原因不可见，不重试绕过。
