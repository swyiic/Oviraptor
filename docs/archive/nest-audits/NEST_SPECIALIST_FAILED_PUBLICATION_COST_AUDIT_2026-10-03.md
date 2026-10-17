# 专家业务发布失败的原费用审计 · 2026-10-03

HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`未变；本批原dirty增量5路径，无重置、批量覆盖、提交、真实DB/CAS/资产/安装或授权URL操作。Master未完成。

## 实际问题与作用域

真正ordinary creator/HMAC/原Root/C及UUID worker出生后只读Mapper调用实际loopback SDK一次。BEFORE received更新/ModelRoundCompleted事件/快照的ABORT或IGNORE令正常业务事务回滚，也回滚原provider费用。首次0/1在消耗请求0而期待1失败（0.42秒）；当时SDK1已达到，随后费用fact/全表重放检查未达到，其余矩阵未达到。

record_received保留原解析/脱敏与正常成功原子receipt/event/checkpoint语义；失败时helper返回后事务已经drop/rollback，私有文件RW/no-CREATE连接在BEGIN前至COMMIT保持同一authorizer，仅允许无trigger accessor的直接INSERT原agent_model_cost_facts和agent_budget_entries，其他写入/DDL/pragma/trigger副作用拒绝。原Root财务和原C以及model_facts原child/worker/role/request/hash/pristine派发前后核验同一锁，绝不创建或补齐身份。只保留hash和原usage/provenance，不把业务失败正文送入邮箱、ACK或finding，不授予执行/重试/退款。已成功提交晚费用时用本地cost_saved标志保留原拒绝，不再追加错误的费用失败。

## 验证与实际失败历史

生产后32/33（46.98秒）：两新实际SDK测试已通过，6业务故障费用请求consumed1，3费用写入故障保持完整回滚；一旧四角色测试在最后tools为空断言失败。前面8真实SDK、4Root费用、4独立child费用与邮箱/关闭断言已达到。旧Root现在有4本地注册工具；测试改核type/name/parameters与每轮原request tools完全一致，四child仍必须空tools，原8调用/费用/邮箱断言保留，不改生产权限。

追加两本地财务边界测试：原caller authorizer保持、相同原已付fact重放不增费用、变化正文hash拒绝且全表不变；传入caller打开事务不得加入或commit，费用拒绝返回且caller事务/全部rows保持。这两项使用本地原dispatch和模拟usage，只验证财务边界，不能当实际SDK验收。六+三矩阵则均走实际生产transport/loopback。

补后39/39（50.69秒）。最后91/91（494.99秒）扩大到Source角色准入、独立Reviewer、金融退出和原晚费用，严格Clippy/导入39/exact退役1通过；没有全Rust/完整应用验收。application comparison保留所有非金融/诊断表；允许的native_sdk_log与sqlite_sequence仍由其已有诊断合同验证，不宣称该测试独自证明所有physical rowid。原费用和model_cost fact身份/hash与请求核验、重复transport全application不变且SDK仍1。

- clippy-final：exit0，wall18.56秒；`/tmp/oviraptor-specialist-failed-cost-clippy-final.log`。
- affected-final：exit0，wall495.65秒；`/tmp/oviraptor-specialist-failed-cost-affected-final.log`。
- importer-final：exit0，wall33.94秒；`/tmp/oviraptor-specialist-failed-cost-importer-final.log`。
- literal-final：exit0，wall1.38秒；`/tmp/oviraptor-specialist-failed-cost-literal-final.log`。

1159集合SHA `ff879e710aafa7869017a3b8486c0365f23f2eea3e70eca133ceaffc51b6bcfb`；5路径3已有/2新，1154原范围外保持；原before/prior和逐文件reviewed-merge-diffs保存于`/tmp/oviraptor-specialist-failed-cost`前缀，diff --check0，新Rust叶独立fmt，旧文件格式保持。tests补丁0fde7132、production最终46f2f2eb、旧tools夹具b98a88b6；早先c7b5f4大缩进版本未应用，应用前提取helper减小差异。一次错误传入SHA被断言拒绝，无文件改动，随后按实测SHA应用。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests_specialist_financial.rs` | 278 | 是 |
| `src-tauri/src/commands/agent_tests.rs` | 127 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/receipt.rs` | 309 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/receipt/failed_cost.rs` | 69 | 是 |
| `src-tauri/src/commands/agent_tests_specialist_transport.rs` | 535 | 否 |

## 未完成与风险

这是有效received费用在业务失败后的保留；正常/晚/uncertain/notsent SDK写入权限隔离、非法usage处理、进程在原响应与本地持久化之间崩溃、显式精确对账/续跑仍需独立证明。ClientSide草案尚未合入，必须把业务语义核验放在原费用保留边界以内，不能盲目沿用旧草案先读当前assignment再保存费用。完整多角色/Broker/真实并发/人工聊天/最终门禁与安装/URL均未完成。

UI轨迹新主题目前只是/tmp草案，与本批Rust范围分开，尚未合入；InputParser旧自动审批拒绝保持、不重试或绕过。Goal active。
