# Root 本地工具显示审计 · 2026-10-03

原dirty工作树增量，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`未变；无重置、提交、真实DB/CAS/资产/安装/授权URL操作。Master未完成。

## 问题与作用域

真实Root2SDK付费本地工具后，public chat投影没有工具名称；新增0/1（1.30秒）在localTools==null失败，后续历史/重放断言未达到。前端真实SFC19/22（2169ms）：聊天/轨迹工具未显示，损坏步骤仍展示普通摘要。scope只包含9路径。

纯读取在完整原request/invoice/paid step/publication/timeline验证后，仅追加localTools静态名称数组，结果正文/参数/ID/私有assistant不输出；没有改持久化event或Native plan。已付工具步骤名称1–4个，无步骤原7字段保持。轨迹以原localStep闭合形状解析，只选四个注册工具名；未知/空/超量/重复ID/额外字段步骤拒显示，当前聊天游标在malformed投影下不前进。前端验证是显示边界，不授任何权限。

实际Rust2SDK证明本地步骤、最终7字段摘要、历史页/重复只读输出一致、原Native JSON和所有rowid保持，0目标HTTP；原付费/发布负向回归保持。UI四新测试使用真实组件+合成IPC，只证明消费者，不算安装/LLM验收。capability预算标签写“预算与能力”，不会把旧硬上限工具说成动态账本。

## 验证与视觉

生产修后Rust31/31（34.14秒）、相关UI22/22，扩大99/99。默认前端823/823（38.459秒）发生在最后三CSS规则修正之前；视觉发现深色Root标题继承旧trace深色文字及ul/li缩进/相邻margin，仅补主题color和工具列表特异性。之后相关22/22（4.125秒）、vue-tsc/Vite再次通过，没有重复全前端或声称其覆盖新CSS。

实际SFC SSR+合成IPC浏览器：1280深色/600浅色，document scrollWidth分别等于viewport；三tool标签同一top，无横溢出；Root标题颜色分别rgb242/29，根代理查图。截图`/tmp/oviraptor-root-local-tool-display-visual/dark-wide.jpg`、`light-narrow.jpg`。fixture带明确“非安装验收”；原整体trace最近记录/metrics主题仍需后续整体UI处理。浏览器临时tab3已关闭、viewport已reset、临时server已停止，未启动实际Oviraptor。

最终门禁：
- ui-final：exit0，wall7.44秒；`/tmp/oviraptor-root-local-tool-display-ui-final.log`。
- build-final：exit0，wall6.6秒；`/tmp/oviraptor-root-local-tool-display-build-final.log`。
- clippy-final：exit0，wall15.49秒；`/tmp/oviraptor-root-local-tool-display-clippy-final.log`。
- affected-final：exit0，wall308.79秒；`/tmp/oviraptor-root-local-tool-display-affected-final.log`。
- importer-final：exit0，wall26.45秒；`/tmp/oviraptor-root-local-tool-display-importer-final.log`。
- literal-final：exit0，wall1.29秒；`/tmp/oviraptor-root-local-tool-display-literal-final.log`。

1156集合SHA `6d689470ca8514c9f340b9dce62926c18e7d7a7c0ed9e04961d5082b03be9def`，9路径8已有/1新，1147原范围外保持；原before/prior及增量逐文件reviewed-merge-diffs保留在`/tmp/oviraptor-root-local-tool-display`前缀。新Rust叶独立fmt，原文件格式保持，diff --check通过。tests补丁c7264aab、production原878550未用（ES2020不支持Object.hasOwn，应用前校正为87ddf6f7）；其余最小中文标签/三CSS增量完整审阅。

| 路径 | 行数 | 本批新增 |
|---|---|---|
| `tools/test_root_decision_chat_ui.cjs` | 121 | 否 |
| `tools/test_structured_decision_ui.cjs` | 124 | 否 |
| `src-tauri/src/commands/agent_tests.rs` | 125 | 否 |
| `src-tauri/src/commands/agent_tests_root_local_tool_display.rs` | 107 | 是 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/local.rs` | 175 | 否 |
| `src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/public_projection.rs` | 137 | 否 |
| `src/features/sentinel/traces/rootDecisionContract.ts` | 118 | 否 |
| `src/features/sentinel/timeline/rootDecisionContract.ts` | 42 | 否 |
| `src/features/sentinel/components/RootDecisionSummary.vue` | 79 | 否 |

## 未完成与风险

固定工具展示不是完整Codex脑力或通用自主工作；动态grant/精确对账/显式续跑、六触发/全部角色/Broker/真实并发、唯一终态quiescence/重启恢复、人工有序聊天、整体UI、旧九E2E和正常已付任务删除未完成。局部全部通过不是完整门禁/安装态/授权URL验收。

InputParser原自动审批拒绝保持，当前摘要没有具体理由，不绕过、不标交付。Goal active，继续开发。
