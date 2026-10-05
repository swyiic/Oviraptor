# 执行概览 UI 审计 · 2026-10-03

Master开发中；本批是前端消费者与视觉完善，不是Master框架完成。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 不变。保留原脏树、API读取/明确操作/异步fencing、全部授权及回执，不触真实数据库/CAS/安装/URL，不提交。

## 已证明的问题及作用域

已全文检查NativeRunStatus.vue、其CSS/labels、NativeBudgetDiagnostics.vue及实际SFC状态测试。旧CSS硬编码白色/浅蓝底与深色主题不符；Root总请求与Token在每个分支重复展示，容易误作分支费用。Source真实暂停保留原Running Root费用行，旧页直接显示raw running且没有说明它是历史记录。

新增暂停显示负向测试真实编译生产SFC，首0/1（504.62ms）在缺少保留记录说明处失败；后续总用量唯一断言尚未到达，不能称首red同时证明全部断言。最小修复后相关55/55（1287.42ms）到达全部断言，保留原费用说明及0恢复/结案命令。没有修改后端Root行以伪造暂停终态。

## 最小修改

- 全任务请求/Token/attempt/智能体记录数只在顶部展示一次；智能体数量明确是记录数，不推断活跃进程或完成百分比。
- 分支保留checkpoint、dispatch和updatedAt；独立分区展示智能体/目标。暂停或人工结案的智能体标签注明原执行状态；既有人工结案未决说明保留。
- 深浅主题使用现有app-surface/ink/muted/line，温和状态色；按容器宽度换行，长URL/回执不溢出。诊断stage放折叠记录，完整失败/义务仍可检查。
- 按需预算盘点只改3条样式；原读请求/十维验证/未知和零区分/无自动扣费与退款合同保持。角色/状态在本地labels翻译，未知值继续原样，不发假阶段。

## 回归与视觉边界

默认前端819/819 runtime38356.48ms（wall38.87秒），vue-tsc/Vite成功。随后实际视觉发现小卡片中Source标题被长状态挤成竖排，加一条team-card-head列排列CSS，最终相关55/55 runtime1249.69ms与`npm run build`退出0。默认819证据属于最后这一条CSS之前，未宣称再次全前端执行。最终编译样式与SFC消费者覆盖该变化。

截图最终版本：

- `/tmp/oviraptor-native-overview-visual/dark-wide.png`：1280，scrollWidth1280。
- `/tmp/oviraptor-native-overview-visual/light-narrow.png`：600，scrollWidth600。

实际生产SFC/Vue在隔离localhost预览；transport仅允许getNativeScanStatus与空事件订阅，其他IPC或native imports拒绝，输入全部合成，未执行Target/SDK或本地应用DB访问。预算面板没有展开，不算真实预算API回放。

临时Vite夹具首次依赖扫描拒绝未mock的native imports；后遇到临时root符号链接和两个Vue模块路径的夹具加载问题，改规范/private/tmp root、显式Vue别名及规范/@fs路径后实际页面才渲染。它们是夹具错误，不算生产功能red。未修改生产Vite配置、安装依赖或降低native隔离。浏览器viewport已恢复，临时tab和4135 server已关闭。

## 原工作树保护

5已有路径、无新增代码文件。1136路径aggregate `9d14980bcfc79411ae542ac55320441694275d03eb0bb1f029e902e26cf76997`，1131范围外原路径保持，diffcheck0。`/tmp/oviraptor-native-overview-ui-baseline/before/prior-diffs/reviewed-merge-diffs/scope-final/code-snapshot` 保存原文与逐文件增量；所有patch全文读取、SHA核对并git apply --check后应用。此前Source金融stage在`aa52b6e194a6177cb49359fba1002808cda4b732cd6935dbbad7d05e0e97ba95`独立封存；384 Rust扩大回归运行期间只编辑前端及/tmp，不改Rust/resource。

| 路径 | 行数 |
|---|---|
| `tools/native_status/state.cjs` | 135 |
| `src/features/sentinel/components/NativeRunStatus.vue` | 367 |
| `src/features/sentinel/components/nativeRunStatusLabels.ts` | 94 |
| `src/features/sentinel/components/nativeRunStatus.css` | 188 |
| `src/features/sentinel/components/NativeBudgetDiagnostics.vue` | 107 |

## 未完成与风险

本批仅改善执行概览，整应用导航/聊天/轨迹/结果/UI仍要继续。截图和模拟IPC不代表真实Tauri安装事件或Agent质量。Source384集合尚未完成；Root完整ReAct/六触发、全部角色/Broker/真实并发、预算动态与对账/有序人工和聊天、终态重启恢复、9旧E2E迁移及最终全量/安装/授权URL仍未完成。InputParser原自动审批拒绝不绕过；当前摘要未保存具体拒绝原因。Goal active，不标完成。
