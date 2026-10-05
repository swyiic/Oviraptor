# 原父监督存活与在途阻断审计（2026-10-02）

Master 继续开发，尚未完成。本增量不更新安装包、不访问授权 URL、不提交 git；只操作隔离测试库和 localhost。此前真实 Nest 的精确 59 行清理及完整资产/CAS保留，以真实清理专项审计为准。

## 问题及红测

1. 父 WorkerSupervisor 的 Drop 停止了后台线程，但原 Coordinator/worker 的数据库租约仍有效，模型取消检查仍允许执行。实际旧入口红测 0/1：`/tmp/oviraptor-supervision-liveness-red.log`。
2. Source 初评在检查原父实例前冻结关注点、签发 worker；即使 transport 最后拒绝，业务库已产生变化。实际红测 15/16，唯一失败是新增初评入口无写入断言：`/tmp/oviraptor-supervision-liveness-source-entry-red.log`。
3. Source 工具的取消检查先取得 SQLite IMMEDIATE 写锁。原父已停止时仍可等待 10 秒 busy timeout；修正夹具的实际红测 0/1，10.91 秒：`/tmp/oviraptor-supervision-liveness-writer-red-fixed-fixture.log`。
4. 仅提前检查父凭证仍不足：健康父的检查若先卡在写锁，无法继续轮询父状态。增加持有实际 writer 的健康检查后实际 0/1，10.88 秒：`/tmp/oviraptor-supervision-liveness-live-writer-red.log`。

中途夹具错误单列：首次新增组合有私有夹具模块不可见和 Transaction/Connection 类型错误；随后 12/15 中三项是把追加式费用回执误断言为幂等成功、以及 endpoint 的 seen 列表在 handler 返回后才追加的异步断言错误。另一个 writer 夹具漏设 Source started_at，未到达目标边界。均保留原日志，修正测试构造与等待，未降低生产权限、账本或唯一回执约束；不当作目标问题红测。

## 最小实现与作用域

- 共享 `multi_agent/supervision_ticket.rs` 保存一次父实例的 alive、故障、规范数据库路径及原 Coordinator 身份。执行凭证只持 Weak，不延长父存活；Drop 先撤活再 join，线程异常/退出也撤活。新服务即使同 Root/epoch/fence，也不能复活旧凭证；不使用可替换的全局“当前 Root 健康”注册表。
- 凭证核验数据库、scan、attempt、target、Root、epoch 和 fence；健康续期的时间值不构成新身份。普通工具还核验实际 run 属于同一 Root 和当前 Coordinator。凭证不签发权限、不改预算、不续 worker。
- Web prepare 在 Mapper 之前挂原实例凭证，所有克隆 context 保留它；实际 specialist SDK 取消和普通 Tool Broker 准入检查凭证。Source 在真实父激活后取得凭证，初评、工具、Reviewer 使用同一 context，拥有 context 的取消检查也保留凭证。
- Source 初评/工具入口在关注点冻结和新 worker 签发前检查；模型费用保存后、响应消费及工具事务授权时检查。Source 取消检查改为 DEFERRED：不需要续期时只读，不取得 writer；确需续期仍验证原活调用和原授权，写入/触发器失败继续拒绝，不忽略权限错误。
- financial fact、原 dispatch 的 received/uncertain/unsent 保存不要求活父凭证。父停止不能退掉已发生的原费用；新业务、工具或响应消费仍拒绝。旧未知调用保留预算与并发槽，不重发。
- 当前 Native JSON、历史原费用、旧 worker 身份、到期撤权、资产/CAS和真实库数据未在本增量改写。没有新增旧格式正向兼容、schema 回填或“恢复权限”的接口。

## 合同与验证

新增 9 项合同，主库由 1711 到 1720：父 Drop、同租约新父不复活、跨库/actor/run、异步父失败、工具拒绝但原费用可保存、Web 真实在途取消、Source 真实在途取消、Source writer 下快速健康轮询及停止、初评停止时不签发。

- 初步 16/16 和扩大 153/153 属于最终 writer 修复前的证据，不宣称当前完整全量。
- 最后 writer 修复及格式后定向 **44/44，46.17 秒，exit 0**：`/tmp/oviraptor-supervision-liveness-final-contracts-after-writer.log`；包含 17 项新旧监督合同、Source tool/specialist 消费路径及严格字面量扫描，集合重叠不相加。
- 最后格式/DEFERRED 修复后的扩大受影响回归 **154/154，102.27 秒，exit 0**：`/tmp/oviraptor-supervision-liveness-final-affected-after-writer.log`。
- 最终全目标/全特性严格 Clippy **exit 0，11.56 秒**：`/tmp/oviraptor-supervision-liveness-final-clippy.log`；导入工具 **39/39，2.35 秒，exit 0**：`/tmp/oviraptor-supervision-liveness-final-importer.log`。8 文件 scoped rustfmt check、git diff --check 及 870 路径最后复核通过。前一 Clippy 28.59 秒在最后 DEFERRED 修复前，不当最终快照。

真实 localhost 测试等待实际请求到达后停止父，要求提供方返回前取消；一条请求、零业务 mailbox、未知费用/槽位保留，后续重复入口无写入和无重发。provider bill 独立合同使用原真实 claim 和 typed response 保存函数，证明原费用保存与执行权分离；不冒充实际提供方在父停止后仍返回收费响应的端到端场景。

## 文件和结构

25 个已有文件仅接线/必要边界修改，5 个新增，无删除。新增手写文件：共享凭证 142 行、专用 transport context 27 行、Web/公共存活合同 171 行、Web 实际 transport 95 行、Source 专项合同 214 行；均低于 400 行。

已有文件：supervisor198、multi_agent/mod31、agent_tests_multi_agent55、agent_tests_fixtures209、agent_tools_dispatch297、child_transport309、prepare323、Source authority311、initial43、tool_execution186、tools331、tests入口98、attempt authority39、Source coverage_budget117、coverage_reviewer389、dispatch395、expired_saved222、heartbeat117、reviewer_fixture317、specialist_cancellation298、specialists386、unsent_production116。

既存结构债：agent_contract1334/agent_backend1317 各只加 context 字段/默认值；Source guidance829 仅补两个既有夹具字段。新存活职责已抽共享层，未在超大文件追加业务逻辑。负责人为当前 Codex 开发任务；后续按上下文契约、普通 Web 构建/执行、Source guidance 合同拆分，保留原权限与恢复回归。本批不做无关搬迁或全局格式覆盖。

基线是前一真实清理工具快照 865 路径；当前 870 路径及 git HEAD 已保存：`/tmp/oviraptor-supervision-liveness-code-snapshot.json`，摘要 `eda10dce251e08393dbe27f34999d07300a03b5646399d3896120489b48e1179`。完整逐文件差异、修改前文本和作用域在 `/tmp/oviraptor-supervision-liveness-*`。没有自动提交。

## 未完成和风险

- 本实例凭证覆盖当前同进程执行；独立 worker 进程/OS dispatcher 的父死亡通知、实际安装 App 强杀/完整恢复矩阵未实现。
- Root 时限后台治理、当前父安全续租及原费用跨 Coordinator 恢复还待开发；本批不续期 Coordinator/worker、也不把 Root 时间重置。
- Source/Reviewer/canonical 请求、已明确未发出和已知付费前缀的安全重派仍未完成。当前 Web 无派发替换范围不扩大。
- Root/Single 全预算、动态 grant、精确人工对账、剩余角色真实执行、聊天完整四态/动作闭环、逐路实时日志，以及最终完整门禁/构建/安装态/授权 URL 未完成。
- SQLite 需真实续租时的 writer 竞争与恢复还须结合完整监督治理验收；DEFERRED 不忽略失败，不把数据库不可验证状态当作健康执行权。

继续上述框架任务，不将局部回归或只读诊断视作 Master 完成。
