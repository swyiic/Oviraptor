# Root 未决费用结果分类审计 · 2026-10-03

HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`未变，原dirty增量3路径；无重置/提交/真实DB/CAS/资产/安装/URL操作。Master未完成。

## 实际问题与最小修改

真实新Root通过原multi_agent_prepare调用本地SDK一次。503留下model_requests indeterminate1，200缺usage留下consumed1，二者input token未决>0、原request1/publication0且assignment/capability/http claim/specialist均0。首1/3（1.57秒）在分类Failed/tool_failure失败，两实际调用此前账本断言达到；后续SDK日志/全表只读断言未达到。负向静态用例原先通过。

仅multi_agent_bootstrap_outcome封闭识别原runtime错误budget_indeterminate_requires_reconciliation及root_tick_uncertain九个ModelError::code；返回Incomplete/REQUEST_RECONCILIATION_REQUIRED，非目标fuse。六个普通文字/畸形/原记录损坏码仍失败。没有改Root费用、SQL、派发、终态publisher、退款/重试或当前Native JSON；不能从该分类推断精确provider对账或续跑已完成。

## 验证

扩大162/163（235.71秒，wall276.33）的唯一失败在新增SDK日志期待：传输错误正确terminalState=uncertain，测试误写withheld。完整读取原diagnostics::finish及transition合同后只纠正测试条件；响应已收但usage缺失仍必须withheld，传输仍必须uncertain，无修改生产日志或放松检查。

最终163/163（226.24秒）三新用例全部通过：二次纯分类与实际原SDK日志读取后所有application rows保持，SDK数仍1；费用保留、无子任务/目标请求、无fuse、无重试。覆盖Root本地工具/逐轮预算/公开聊天/SDK日志/原预算与Native/Single finally/恢复等相关集合。不是全部Rust/安装/真实模型脑力验收。

- strict-after：exit0，wall9.37秒；`/tmp/oviraptor-root-unknown-outcome-strict-after.log`。
- affected-final：exit0，wall247.32秒；`/tmp/oviraptor-root-unknown-outcome-corrected-affected-final.log`。
- importer-final：exit0，wall23.29秒；`/tmp/oviraptor-root-unknown-outcome-corrected-importer-final.log`。
- literal-final：exit0，wall1.32秒；`/tmp/oviraptor-root-unknown-outcome-corrected-literal-final.log`。

1157集合SHA `bce71e42e7e3731332bbbf56f632a227f791ae5f2e9d1649618b9af1805867af`；3路径2已有/1新，1154原范围外保持；原before/prior和reviewed-merge-diffs在`/tmp/oviraptor-root-unknown-outcome`前缀，diff --check通过。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `src-tauri/src/commands/agent_tests.rs` | 126 | 否 |
| `src-tauri/src/commands/agent_tests_root_unknown_outcome.rs` | 121 | 是 |
| `src-tauri/src/commands/multi_agent_external_surface.rs` | 393 | 否 |

Master尚缺完整多智能体、预算动态/精确对账、恢复和人工聊天、整体UI及最终验收；InputParser旧自动审批拒绝仍保持，不重试或绕过。Goal active。
