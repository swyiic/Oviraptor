# 七类终态实际原入口审计（2026-10-04）

旧`every_target_ends_in_exactly_one_terminal_state`直接构造七种结果、播种旧Root并传裸回调，未提供当前原财务合同及实际退出；最近原14快照13/1。当前以真实creator/startup/HMAC、冻结原Root、实际SDK/HTTP或调用前拒绝、`run_agent_target`及owned消费者替换该区域。生产代码、预算准入、费用、Native JSON、allowlist均未改变；原非失败词汇断言保留。取消仍不计完成，现实际写出cancelled目标/checkpoint而不跳过验证。

| 场景 | 实际生产者与作用域 | SDK/HTTP | 原消费结果 |
|---|---|---|---|
| 完成 | 捕获合成session-a，真实登录/匿名HTTP对照；早finish拒绝后两个无进展窗口 | 13/2 | completed，覆盖authorization，无确认漏洞/命名缺口 |
| 带缺口完成 | 实际匿名HTTP，匿名材料不足以关闭authorization；两个无进展窗口 | 13/1 | completed_with_gaps，保留命名缺口 |
| 暂停 | 原先绑定的B在实际SDK响应前从临时库删除；实际授权拒绝阻断两侧HTTP | 1/0 | paused，原已付账单/退出保留，不恢复B或补造覆盖 |
| 保护停止 | 实际/api/gated challenge响应 | 1/1 | protected_stop/WAF；关联既有e2e还核验fuse |
| 取消 | 占住真正全局cloud准入3槽；实际请求排队后临时scan状态撤权；原gateway产dispatch/unsent | 0/0 | 原Single退出有效，cancelled，计数0，不清理已付账单或虚构取消费用 |
| 恢复不兼容 | 真Multi Root/C1监督/SDK之后加入缺权限的临时旧target历史，实际bootstrap拒绝 | 1/0 | resume_incompatible，需要新尝试，原七张权威表和已付费用保持 |
| 失败 | 真实creator后使用空模型名形成冻结计划；实际model_setup拒绝 | 0/0 | failed，原Single财务退出有效，不造Native checkpoint或SDK用量 |

另一个具名测试：同样真实队列请求，在临时scan状态pausing时，原gateway保存unsent和原Single财务边界；owned消费者返回false，Root paused且terminal_state空，目标paused，agent_terminal不存在，tally0。重复回调所有typed rows/rowid保持。因此原始Cancelled返回值不能直接等同用户取消终态。

每种主场景核验原Root捕获身份、真实Single退出或Multi FinalClock原退出、十维无未结预留/不确定费用；SDK/HTTP计数与原账本消费相同，模型输入/输出只在实际调用时有已付消费。owned首次消费不改任何原费用；目标只有一行，checkpoint与Root canonical状态相符。第二次owned消费不改全应用typed rows/rowid，不追加调用、不重计。错目标拒绝，孤立attempt_count=2后的迟到回调拒绝，负向前后全部原行保持。该单字段轮次负向不是完整恢复/重启验收。

恢复场景复用已审核bootstrap helper，仅播种不完整历史拒绝输入，不播种正向Root/费用/子授权/SDK/退出；另有相关bootstrap三场景及错报负向继续通过。七终态为1个具名测试，暂停区分为另1个，不能把内部循环报为7个测试。

## 验证与保护

首轮编译成功，1/2：完成场景实际SDK13次，测试误写2次而失败。源码中stall窗口先检查再停止，拒绝早finish也计一次无进展；修正为1+2*实际冻结window，当前13；未松动生产门限/账本或删业务断言。第二轮2/2，编译21.91秒、测试13.33秒；首次失败日志保留。

最终关联22/22（53.67秒）、原14独立14/14（57.12秒），两选择集合与报告/通过集合精确相同，无ignored。严格offline/locked/all-features/all-targets Clippy -D warnings 0（11.30秒），退役46/46（5.62秒，含literal exact/当前Native JSON原字节回环）。两新叶局部rustfmt及diff--check通过；不声称全库fmt、UI/build/安装或整体验收。

范围3路径：已有identity_e2e仅替换七终态区域为include（491→316行，其余字节保持）；新original验证叶273行、producer叶192行，均不超过400行。已有全Git diff与三最终增量全文审查，保护本批前未提交改动；1327原范围外SHA、HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b不变；1330源码集合SHA35fb78d5a16d776405e2f944a8ee66275d55906d747a61b59db504e89bb6e657。Master/progress/handoff在备份后追加，原全文后缀字节保持。不reset、不批量覆盖、不提交。

证据：`/tmp/oviraptor-seven-terminal-original-{baseline.json,before.json,scope-final.json,code-snapshot.json,prior-diffs/,reviewed-merge-diffs/,selected.json,checks.py,checks.json,first.log,second.log,related.log,original14.log,clippy.log,retirement.log}`；初始文档前像`/tmp/oviraptor-seven-terminal-docs-before/`、最终前像/增量`/tmp/oviraptor-seven-terminal-original-docs-final/`。仅临时SQLite/localhost/合成会话，未写真实DB/CAS/资产，没有安装App或外测。

## 未完成与风险

Master仍有14类剩余，最新完整范围见Master顶部。6项Source Broker既有无原始子权限夹具失败仍在；本轮仅只读核验其source_broker_context创建Coordinator/旧计划，没有实际角色授权，不计修复或通过。Source其他调用同helper，不能假定债仅六项。还须删除/残余路径、动态预算/全通道对账/恢复、六类监督/全角色真实推理与并行、聊天/逐路日志/整体UI、非Web及知识资产生命周期。完整门禁、安装App打开、授权URL与真实模型质量最后。

队列cancelled/pausing均为临时状态负向，证明实际队列/调用前归属消费，不是活动取消IPC/GUI、已发远端取消、清理join全部通道或完整重启验收。SDK脚本不能证明真实模型理解/推理/纠错与用户聊天体验。没有以原14全绿标Goal完成；Goal保持active。
