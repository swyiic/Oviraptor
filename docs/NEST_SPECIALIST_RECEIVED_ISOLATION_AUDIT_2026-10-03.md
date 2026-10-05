# 专家 received 回执写入隔离审计 · 2026-10-03

HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`不变；5路径3已有/2新，无重置、批量覆盖、提交、真实库/CAS/资产/安装/URL操作。Master未完成。

## 实际问题与最小范围

真实ordinary creator/HMAC/原Root/C/UUID worker调用loopback生产SDK，provider200响应。原received事务触发器能将projects.status从active改成archived。首0/1（0.38秒）在项目全行比较失败；transport result已返回，但后续Err/费用/重放/最终SDK次数未断言，后面event/snapshot/consume三分支未到。

新增73行private_writer，RW/no-CREATE/FKON/syncFULL/busy250ms，在record_response原BEGIN之前到其COMMIT/ROLLBACK保留同一hook。只main/no-accessor允许直接INSERT原events/snapshots/budget_entries/model_cost_facts，UPDATE原specialist回执六字段、snapshot四字段与user_directives payload/updated_at；所有其他SQL写入/DDL/pragma/附带trigger写意图拒绝。唯一例外是原agent_collaboration_directive_delivery向collaboration_events的INSERT，它在同一原业务事务、执行状态确认后且任何业务写入前核sqlite_master完整原定义；借名替换/缺失拒绝，费用不丢失。复用web_creation_schema的原canonical函数，加封闭名字/错误参数helper；原Root创建verify仍相同trigger和错误，带引号字节核验算法不变，不新增表/旧回填。

record_received原解析/脱敏/拒绝标记及原record_response整个成功事务保持。只在wrapper经私有连接调用，成功费用/event/快照仍原子；业务失败仍到已存在受限fee-only原owner/C/worker/request fallback。晚已提交费用仍用cost_saved保留原拒绝，不重复写。接线只2新增模块/验证语句及一调用替换，不改Native JSON/邮箱/Reviewer裁决/授权。

## 验证和限度

四实际provider200分支：received/event/snapshot附带projects更新被拒绝，业务回滚、原有效账单consumed1/indeterminate0由fallback保存；consume附带projects更新亦拒绝，原费用与业务完整回滚，返回次费用错误、原派发/预留阻断重发。同actual transport重入所有application rows不变，每分支SDK始终1。

两个actual provider200 emitter分支（缺失/替换成SELECT1）：返回封闭emitter冲突、无次费用错误，原consumed1/indeterminate0、不发布业务，重入全application不变且SDK1。并未假装调用了human多动作链；source_guidance正向actual transport/独立Reviewer/送达恢复已有受影响合同在82集合内通过。

另一本地正常/晚边界使用真实原dispatch和模拟usage，无provider调用：正常成功保留caller projects INSERT拒绝hook，晚model_cost fact trigger试改业务被拒绝且所有application rows保持；随后caller hook仍各触发一次。本地合同不是实际SDK验收。旧caller打开事务拒绝、成本重放/哈希冲突、实际Source取消/晚回执均在相关集合内通过。application比较器排除SDK日志和sqlite_sequence，费用允许时再排除两金融表；不独自声明全部rowid/日志内容或安装验证。

82/82（311.08秒）后只追加边界测试，生产未再修改；9/9（7.95秒）含5原fresh finance和1真实Single/Multi创建，精确名字交集1、并集90，不能写91或当全部Rust。其后严格Clippy、导入39、exact退役1通过，未重复未变化长回归。

- clippy-final：exit0，wall18.83秒；`/tmp/oviraptor-specialist-received-isolation-clippy-final.log`。
- importer-final：exit0，wall29.99秒；`/tmp/oviraptor-specialist-received-isolation-importer-final.log`。
- literal-final：exit0，wall1.34秒；`/tmp/oviraptor-specialist-received-isolation-literal-final.log`。

集合1166 SHA `24a16c5d05235dbae64bf513797a619cbf4899b25db6cfd4d25f18f83d791278`，1161原范围外保持；before/prior/reviewed-merge-diffs在`/tmp/oviraptor-specialist-received-isolation`前缀，逐文件全文已审查，diff --check0，新Rust叶独立fmt，旧父文件格式保留。tests SHA f2262847、production14b6cca7、emitter/boundary tests35596769。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 131 | 否 |
| `src-tauri/src/commands/agent_tests_specialist_received_isolation.rs` | 104 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/receipt.rs` | 316 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/receipt/private_writer.rs` | 73 | 是 |
| `src-tauri/src/collaboration_events/web_creation_schema.rs` | 73 | 否 |

## 未完成与风险

本批封普通专家回执SQL写入，不是全部SDK链隔离已验收。clock异常字符串notsent、非法usage、响应与持久化间强杀、精确provider对账/显式继续、动态grant、全角色/Broker/真实并发/人工聊天/整体UI/旧九E2E/正常paid删除和最终门禁/安装/URL均未完成。ClientSide/Human SDK草案仍未合入；其业务语义必须在原费用保留边界内，不能据角色字符串补权。当前loopback/临时库不代表配置模型质量或安装状态。InputParser既有自动审批拒绝保持，不重试或绕过；Goal active。
