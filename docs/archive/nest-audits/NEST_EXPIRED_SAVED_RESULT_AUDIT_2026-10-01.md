# Expired 原结果本地收尾开发审计（Goal active）

## 问题与作用域

当前工作树证明：原 worker 已 expired、完整专家回执已 received 时，Investigator、Mapper、Web Reviewer 和 Source Reviewer 的本地收尾仍拒绝 `worker_lease_expired`，部分入口还要求将原 worker 改成 completed。四项实际红测 `/tmp/oviraptor-expired-saved-red.log` 为 **0/4、exit 101**，不是编译失败。另一个红测 `/tmp/oviraptor-expired-saved-replay-red.log` 为 **0/1、exit 101**：Investigator 的已完成快捷返回接受了被改成未来期限的 expired 审计。随后 `/tmp/oviraptor-expired-source-completed-audit-red.log` **0/1、exit 101** 又证明 Source completed progress 在原 failure 被改写后仍返回 Delivered。

本批只使用临时 SQLite、临时工作目录和 localhost 模型端点。恢复夹具明确从暂停 worker 构造 expired，不是生产过期回收、重派、强杀或安装态证据。没有访问真实业务库/CAS、删除资产、安装应用、提交或访问授权 URL。

## 最小修改

- 新 `attempts/expired_saved.rs` 提供不可转为执行授权的本地证明。要求同 Root、当前原 Coordinator epoch/fence、有效期限、活动 scan/Root、原 child/role/lane、合法 worker UUID、真实过期时间与完成时间、确切 `worker_lease_expired`、原 reserved request、原 lane，以及全部能力已撤销。完整 received 回执、费用、事件和检查点必须能重新核验；未知、缺失或拒绝响应不进入发布。
- 事务开头保存 **worker 全列、原 specialist call 全列**；assignment/run 只允许明确的本地结算与终态字段变化，其他全列保持。最后业务发布、ACK、Source 裁决或 Gap 写入之后仍复核同一证明和当前 Coordinator，不能以重新读取的合法新状态替代原证明。
- 已接 Mapper/其他只读角色共用入口、Web Reviewer、Investigator、Source 候选与总体覆盖 Reviewer。仅逻辑 assignment/run 结清，原 expired worker 的 state、finished/failure、heartbeat/deadline、身份及其他全部列不改；只释放本 assignment 的 lane/并发槽，不恢复能力或重新请求模型。普通 live 执行继续拒绝 expired。
- Source progress 可只读识别有完整 received 证明的过期结果。Investigator 幂等快捷返回也先核验过期审计。Source 候选/覆盖共用的历史 audit 也复核 expired 的原身份、期限、完成与 failure，completed progress 不跳过；历史读取不要求活动 Root 或有效 Coordinator，也不签发权限。跨 Coordinator generation 的结果发布、安全重派与生产过期 API 仍不开放。

## 验证证据

| 证据 | 结果 |
| --- | --- |
| 最初恢复红测 | `/tmp/oviraptor-expired-saved-red.log`：0/4、exit 101 |
| 幂等审计红测 | `/tmp/oviraptor-expired-saved-replay-red.log`：0/1、exit 101 |
| 首次四条恢复合同 | `/tmp/oviraptor-expired-saved-first-green.log`：4/4、exit 0，9.38 秒；Source 用例分别执行候选/覆盖两种模式 |
| 新增扩大负向合同 | `/tmp/oviraptor-expired-saved-negative-green.log`：8/8、exit 0，33.65 秒；最后 Source audit 修复后 `/tmp/oviraptor-expired-saved-final-green.log`：8/8、exit 0，33.62 秒（历史读取夹具追加前） |
| 受影响既有合同 | `/tmp/oviraptor-expired-saved-affected.log`：125/125、exit 0，738.76 秒；发生在最后 Source audit guard 四个生产文件与一个测试文件增补前 |
| 最后 Source audit 与历史读取 | `/tmp/oviraptor-expired-source-closed-audit-affected.log`：20/20、exit 0，446.73 秒，包含末次历史读取夹具；与前组重叠，不相加 |
| 严格静态检查 | `/tmp/oviraptor-expired-saved-clippy.log`：全目标全特性 `-D warnings`，exit 0，12.79 秒 |
| 退役字面量登记 | `/tmp/oviraptor-expired-saved-retirement.log`：1/1、exit 0，0.48 秒 |
| 差异空白检查 | `/tmp/oviraptor-expired-saved-diff-check.log`：exit 0 |
| 作用域格式检查 | `/tmp/oviraptor-expired-saved-fmt.log`：exit 0 |

新增合同覆盖原行全列不变、禁止能力/运行/Coordinator 恢复、原费用仅一次、仅释放自身并发槽、canonical replay；坏期限/完成时间/failure/UUID、未知或缺失回执、坏 checkpoint、撤权、Root 取消及当前 lease 失效无本地写入。发布故障覆盖三种 Web/只读消费者及两个 Source Reviewer；完整应用表快照证明 DB 回滚。Root 结束且 Coordinator 到期的合法 Source expired 历史审计保持可读，完整应用表快照不变且未续租。损坏恢复夹具只在临时库移除 specialist 不可变触发器，以证明回执守卫和最后原证明，而非仅依赖 SQL ABORT；生产触发器未改。

## 工作树保护与快照

HEAD 仍为 `59be3d86d25adda5b1f975759bf256ef3b94f32b`。相比上批 820 路径代码摘要，本批只改 14 个已有代码文件、新增 4 个文件，缺失文件为 0；不以 git diff 为空判断 untracked 文件无改动。原 12 个生产文件全文保存在 `/tmp/oviraptor-expired-saved-before.json`；逐文件增量已检查并保存 `/tmp/oviraptor-expired-saved-scoped.diff`，两个 include 入口只增加测试登记。

本批代码/测试/配置快照 `/tmp/oviraptor-expired-saved-code-snapshot.json` 共 **824** 路径，聚合 SHA-256 `b0ec7fd5b90447ff622d89f4628d3202b0f2159ca3d9e568d675eff606779f88`；文档不在该快照中。最后受影响检查/Clippy/退役登记之后，824 路径逐文件重算均未改变；HEAD 不变，含新增审计文档的展开工作树为 924 个脏路径。初阶段摘要另存 `/tmp/oviraptor-expired-saved-pre-source-audit-code-snapshot.json`。当前新增 proof/audit 203 行，三个测试文件 174/209/222 行；修改的生产文件均未超过 400 行，没有删除原测试。

## 未完成与下一步

本批没有当前最终全量 Rust/UI、生产构建、安装态或授权 URL 验收，安装态仍是之前启动修复版。

下一切口是 **真实原子过期 API 与生产调用**：同 Root 当前 Coordinator 授权精确旧 UUID/fence/deadline，在 IMMEDIATE 事务内撤销原执行许可，保留未知预算、lane/contract/slot、调用历史和原费用归属；renew/expiry 竞争、写中 IGNORE/撤权及重复调用必须逐项证明。不能直接调用普通 finish、恢复 Root、退款未知费用或自动重派。

生产 expired/safe reassign、完整预算恢复及 Root/单智能体/动态 grant/精确人工对账、持续 Supervisor/真实强杀、聊天四态与工具交付、AST/浏览器/外部进程逐路实时日志、旧数据盘点备份、最终完整门禁/当前安装态/两个授权 URL 均未完成。当前测试是开发合同；Master 未完成，Goal 不暂停、不完成，继续先框架后共同验收和细节完善。
