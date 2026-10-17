# 结构化决策展示审计 · 2026-10-03

本批按用户新增的 UI 要求，改善真实轨迹和结果页的决策呈现。后端持续模型决策、用户聊天发布和有序角色执行仍在开发；本批不证明它们完成。没有真实数据库、CAS、资产、安装包、授权目标 URL 或提交操作。

## 问题和作用域

当前 Native trace IPC 的 detail 是展示用 JSON 字符串；两个实际组件直接放入 pre。观察、缺口、下一步、费用建议和真实用量缺少层次，旧模型正文也可能直接出现在展示中。先合入 tests-only，通过实际 SFC 和原 composable 的 transport harness 复现 3/10 通过、7 项失败；既有项目/任务/卸载 fencing 的三项已通过，不计作新实现。

生产修改仅是严格公开摘要 parser、共享主题卡片、两个现有 SFC 的薄 hook 和模型轮次标签。仅接受完整 Native coordinator model_round_completed 展示 envelope、原事件身份和严格有界五列表；摘要或身份无效、限长预览时不展示原模型正文。额外 body/reasoning 不进入卡片，HTML 文本继续由 Vue 转义。这个前端检查不是原财务凭据验证，也不新增执行权限。

观察、缺口、下一步建议和风险分别呈现；空列表表示没有提供内容。原 usage 的五个安全整数必须满足缓存/输入/输出/总量关系；不满足时费用未知。costNotes 明确是建议，没有金额换算、采纳按钮、假思考或进度。既有主题、目标筛选和异步作用域 fencing 保留，窄屏改为单列，费用说明支持键盘展开。

测试 loader 只增加对实际子 SFC/TS 的加载，没有替换业务状态或新卡片。新增十项合同已经加入 package.json 的默认 test:ui，后续门禁不会遗漏。

## 根代理实际证据

- 原补丁 01 SHA `4f8f3993c225c6a9e31a600f267f32c035eea8f6cb8b8d2568709977c7e4e70a` 精确 check/apply；实际 red 为 3/10，exit 1，0.938 秒，`/tmp/oviraptor-structured-decision-ui-red.log`。缺卡片、未知费用、私有正文隔离和空语义的七类失败均保留原日志。
- 修复 02 SHA `25985972e820f168fdca51be351428ff42c814713dd97dfacc131704b7041c30` 精确 check/apply；六个实际前端入口相关 71/71，包含新十项，exit 0，3.491 秒，`/tmp/oviraptor-structured-decision-ui-affected.log`。不引用草稿临时副本的 197 项作为本仓库证据。
- 默认门禁追加补丁 SHA `ea5297c04de084bd74fe9207654de2e11b2646f840e46cb0545532793b3a7705`；实际 npm run test:ui 为 792/792，exit 0，27.523 秒，`/tmp/oviraptor-structured-decision-ui-default-full.log`。与 71 项重叠，不相加。
- 实际 npm run build 的 vue-tsc 和 Vite exit 0，Vite 2.31 秒，`/tmp/oviraptor-structured-decision-ui-build.log`。此构建在默认测试命令追加前，前端生产源码与最终快照一致；命令追加没有改依赖或构建行为。
- 十路径逐文件保存原文与 dirty diff，最终差异空白检查通过。六已有、四新增，新叶最大 103 行。999 个已声明代码路径 SHA `00d6ee798faf40ab451b7319dca87c103a52dcea51c24a089e9e5cd950e5ce7c`，本批基线的 989 个范围外原路径不变，HEAD 仍 `59be3d86d25adda5b1f975759bf256ef3b94f32b`。该完整快照包含当时正在开发的 Source/Core 财务批，不能据 UI 门禁推断那些 Rust 修改已验收。

快照、范围和前后差异位于 `/tmp/oviraptor-structured-decision-ui-code-snapshot.json`、`-scope-final.json`、`-before.json`、`-reviewed-merge-diffs/`。原草稿位于 `/tmp/oviraptor-structured-decision-ui-draft`；只按增量 hunk 合入，未用镜像覆盖共享文件。

## 未完成和风险

这些是实际前端组件/消费者合同，host 和 IPC 输入是夹具；没有宣称真实模型已经付费并产生这些摘要。g2/g3 SDK、原发票、持久发布与实际聊天 cursor 必须由后续后端生产合同证明；不得把 run-local sequence 伪装为聊天 sequence。当前人工 review actions 仍有 not_started，尚未完成有序角色执行。

未做本批浏览器截图或当前安装态 WebKit 视觉/键盘验收。原 trace 条数和 20k 展示预览仍可能隐藏较晚或较大摘要，截断内容安全隐藏；完整回放能力仍需后续处理。Master、完整预算、全部角色/Broker、暂停/终态、逐路 SDK/Web 日志以及全量/安装态/授权 URL 均未完成，继续开发。
