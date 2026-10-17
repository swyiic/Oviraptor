# 模型源码工具实际范围与身份审计

状态：源码文件工具已接入真实分析视图和运行身份。完整流水线 session 93712 退出 0：主库 1075 项（493.17 秒）、历史导入器 30 项（2.15 秒）、严格 Clippy、fmt、空白检查全部通过。这不是源码多智能体或整个 Master Plan 的完成声明。

## 实际修复

- 生产入口恢复冻结合同、完整来源和实际分析视图，不因缺少视图回执而回退整仓。
- inventory/search/read/dependency/候选路径只接触选中文件；保留空格、中文、换行与前导点的精确路径，不接受路径别名。
- 必须存在同库、同 scan/attempt/target 的有效 run 与真实 coordinator root；停止、取消、旧合同及身份变化拒绝。不得伪造 root 或把数据库错误当默认 revision。
- 逐工具 IMMEDIATE 事务复核原有权限、合同、身份以及视图完整性；候选写入后再次检查，失败全部回滚。
- 候选必须引用真实文件及有效行号，绑定内容与分析视图摘要；同名不同位置不会误合并，相同输入保持幂等。

## 回归与证据

新增 `commands/tests_source_broker_scope.rs` 9 项真实生产入口测试，覆盖 Diff/Full/Auto、特殊路径、缺回执、身份/状态/版本变化、写入触发器撤权和候选位置绑定。

初版红测日志 `/tmp/oviraptor-source-broker-scope-red.log` 中 inventory 真失败，第二项因 trigger 名拼错而失败，不能算第二项缺陷证据。修正 fixture 后 `/tmp/oviraptor-source-broker-scope-red-v2.log` 两项均真实失败；修复后 green 日志两项通过。最终 focused-v3 为 9 项通过，pipeline 为 61 项，routing 为 3 项，retirement 为 4 项，严格 Clippy-v2 通过。

完整日志：`/tmp/oviraptor-source-broker-full.log`、`/tmp/oviraptor-source-broker-clippy-final.log`、`/tmp/oviraptor-source-broker-fmt-final.log`、`/tmp/oviraptor-source-broker-diff-final.log`。

退役 allowlist 没有放宽：迁移文件仅增加 analysis_policy_version 一行，移除此行精确重现旧受审摘要，详见分析视图审计。路由 fixture 只改一处说明与三个断言，不再把没有合同/视图/run 的 snapshot 当权限；反向恢复精确重现旧摘要 `4fe7634783f11d1adbaed032bbe0fc328fe4c14f7a5a8cc877c5e4a593434d81`，新摘要 `336c88b444a58901aa8fee6692464e40d3476bce6379868f990834fd6e2b39d8`，四个历史字面量和退役拒绝断言保持不变。真实授权正例由新增入口测试覆盖。

## 仍未完成

1. 后续已接通分析结果和灰盒 sourceClaims 的 attempt/view/实际导入 bundle/revision 接受回执，验证进度见 `NEST_SOURCE_RESULT_RECEIPT_AUDIT.md`。本审计的 1075／30 门禁是该后续增量之前的结果。
2. source assignment.finish 尚不是持久 assignment/mailbox 结束；source 专用 child-run 和独立 Reviewer 链未接通。现有工具授权仍仅支持 single coordinator / multi web_executor，不因本修复放开任意 child。
3. CI root、当前 candidate/revision 资格、去重及缺少独立 review 时的通过语义待修复。
4. 完整派发冻结、沙箱/工具供应、凭据生命周期、资产/知识/UI、未知效果恢复与授权环境验收继续按 Master Plan 实施。没有部署、外部目标访问或新增 Host Agent。
