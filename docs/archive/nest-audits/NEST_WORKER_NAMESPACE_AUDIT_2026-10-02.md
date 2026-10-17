# 2026-10-02 Native worker 存储隔离审计（开发中）

Master/§5.2 未完成。当前增量建立同一逻辑 assignment 下独立 child 的存储边界，尚未签发生产重派，也未交付后台 Supervisor、完整预算/聊天/逐路日志和最终安装态/授权 URL 验收。

## 问题与作用域

- 专家调用以 assignment 为主键、能力租约以 assignment/capability 唯一；第二个 worker 的新记录无法与原调用/撤权证据共存。实际红测 `/tmp/oviraptor-worker-namespace-red.log` 0/2，分别命中原唯一约束。
- Source 轮次以 assignment/round 唯一，工具回执没有 child 字段和对应复合外键。实际红测 `/tmp/oviraptor-source-worker-namespace-red.log` 0/2，命中原轮次唯一约束及缺失 child 列。
- 只处理当前 Native 的四个表，不增加 Strix 正向兼容、旧 worker 默认权限或历史费用。没有真实 DB/CAS/资产/安装包/提交操作。资产表未列入生产升级写集。

## 生产修改

- 专家调用由 child 唯一；能力租约由 assignment/child/capability 唯一。调用回调、回执恢复、Source Reviewer、聊天 guidance 和调度重放均绑定精确 child，保留撤权的其他 worker 记录。
- Source 轮次以 assignment/child/round 唯一；工具回执绑定同一三列父键，call ID 仅在原 child 内去重。轮次、工具完成、检查点、历史展示连接和恢复读取带原 child 条件。
- 启动在 orchestration 列升级之后执行 IMMEDIATE Native 存储升级。识别已审阅的当前/前一版 Native 声明，未知扩展或额外引用拒绝。专家/能力和 Source 父子对共用同一事务；复制精确 rowid、SQLite 类型和值，逐向比较数量及差集，保留索引/触发器。Source 工具 child 仅来自已有父轮次，不制造 attempt/worker 身份。Source 工具不可变 guard 增加 child 身份。
- TEMP 复制表只存在于升级事务；父子对先保存两表，再先退下工具表、重建父轮次并回填原记录和回执。任何未知 schema/引用/孤儿/坏 guard/值变化均回滚，原表、行和审计约束恢复；不是旧数据清理。

## 验证状态

- 最后格式/四表回滚增强之前，作用域 worker namespace + Source rounds **27/27、exit 0（35.13 秒）**：包含两 worker 独立存储、相同 call ID、回调/工具仅写原 child、调度重放只读、全表逐值保留及 Native 升级重入。
- 专家/能力六项首次通过 6/6，已包含在 27 项中，不相加。初次能力保留测试将合法 child outbox 当作额外写入，已修成明确断言新增 child 的唯一事件；不改生产防护。
- 两次编译诊断修复为局部 borrow 生命周期和 gap audit 当前 child 查询；两次批量替换断言提前终止，Source wiring 前扩大测试实际失败，已完成精确接线。编译/接线过程日志不作为问题红测或最终绿色门禁。
- 最后代码扩大受影响回归 **292/292、exit 0（1265.14 秒）**：`/tmp/oviraptor-worker-namespace-affected.log`。含上述11个新合同、Source聊天guidance/真实传输及费用/恢复门禁；集合不相加，仍非最终全量或整体验收。最后全目标全特性严格 Clippy **exit 0（8.45秒）**；最后 namespace + 退役登记 **12/12、exit 0（8.07秒）**，其中11个namespace合同与292项重叠、不相加。共享初始化的 import-tools 目标最终 **39/39、exit 0（2.32秒）**；日志分别 `/tmp/oviraptor-worker-namespace-clippy-last.log`、`/tmp/oviraptor-worker-namespace-final-controls.log`、`/tmp/oviraptor-worker-namespace-importer-final.log`。
- 最后作用域 rustfmt/check 和 git diff --check 通过；841源码/测试/配置路径摘要 `9fad8a5073d71b94f8cd8bc0f06463c169779d61cd6660503d2632755dc1fa65` 保存在 `/tmp/oviraptor-worker-namespace-code-snapshot.json`，最后再次核验841路径无差异、HEAD仍为 `59be3d86d25adda5b1f975759bf256ef3b94f32b`。292项之后只补两个显式模块路径（共享初始化在导入工具中编译的路径差异被严格Clippy实际发现）与已存在工具guard合同的有效新父键/完成迁移断言。最后12项、39项和严格Clippy均在这些变更之后运行；最后代码摘要 `43e884f85684504979dd0944b42f4bae679b3bd901adc0403eda30c935893e19` 及最终fmt记录分别在 `/tmp/oviraptor-worker-namespace-code-snapshot-final.json`、`/tmp/oviraptor-worker-namespace-fmt-final.log`。首次Clippy两个路径错误和缺 import-tools feature 的命令诊断均保留，不作绿色证据。

## 修改保护与体量

修改前831路径和全Rust文本分别保存在 `/tmp/oviraptor-worker-namespace-baseline.json`、`/tmp/oviraptor-worker-namespace-before.json`，本批精确增量见 `/tmp/oviraptor-worker-namespace-scoped.diff`。24个已有文件、10个新文件；未重置/覆盖其他改动或删原测试。

Source validation 整块先 byte-equivalent 抽出，摘要 `cd92731bbc443bc9867d8b645ab0c530f5c14ab91e4039aba6e355c2d76b900d`，再增加 child 条件。格式展开的 Source 工具执行、Source assessment/authority、专家测试 transport 按完整函数块 byte-equivalent 移出，三个摘要见 `/tmp/oviraptor-worker-namespace-move-proof.json`。API、Native JSON 和既有安全测试保留。

`db_schema.rs` 是既存静态 schema 目录，本批只改变四个 Native 存储键及工具 guard，迁移逻辑另置两个 <=200 行模块；其2049行结构债按 Master §4.1保留，避免同时大规模迁移其他资产/配置 schema，后续按 schema 所属模块整理。其他本批手写文件均 <=400 行。

| 文件 | 当前行数 |
|---|---:|
| `src-tauri/src/db_schema.rs` | 2049 |
| `src-tauri/src/db.rs` | 372 |
| `src-tauri/src/commands/native_execution_history.rs` | 267 |
| `src-tauri/src/commands/multi_agent_gap_recovery.rs` | 170 |
| `src-tauri/src/commands/multi_agent_review_recovery.rs` | 157 |
| `src-tauri/src/commands/agent_tests_multi_agent.rs` | 46 |
| `src-tauri/src/commands/native_source_tools.rs` | 326 |
| `src-tauri/src/commands/tests.rs` | 94 |
| `src-tauri/src/commands/agent_gap_review.rs` | 375 |
| `src-tauri/src/commands/native_source_coordinator.rs` | 224 |
| `src-tauri/src/commands/multi_agent/child_transport.rs` | 307 |
| `src-tauri/src/agent_runtime/multi_agent/source_reviewer.rs` | 376 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds.rs` | 314 |
| `src-tauri/src/agent_runtime/multi_agent/source_coverage_reviewer.rs` | 383 |
| `src-tauri/src/agent_runtime/multi_agent/specialist.rs` | 341 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds/completion_audit.rs` | 104 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds/dispatch.rs` | 369 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds/reentry.rs` | 217 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds/tool_delivery.rs` | 195 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/schedule_authority.rs` | 136 |
| `src-tauri/src/agent_runtime/multi_agent/scheduler/assignment.rs` | 389 |
| `src-tauri/src/agent_runtime/multi_agent/specialist/receipt.rs` | 282 |
| `src-tauri/src/agent_runtime/multi_agent/budget/model_facts/dispatch.rs` | 87 |
| `src-tauri/src/agent_runtime/multi_agent/directive/source_guidance/projection.rs` | 151 |
| `src-tauri/src/db/worker_namespaces.rs` | 167 |
| `src-tauri/src/commands/native_source_tool_execution.rs` | 177 |
| `src-tauri/src/commands/tests_source_worker_namespaces.rs` | 117 |
| `src-tauri/src/commands/tests_source_worker_upgrade.rs` | 242 |
| `src-tauri/src/commands/native_source_assessment_authority.rs` | 302 |
| `src-tauri/src/commands/multi_agent/child_transport_tests.rs` | 113 |
| `src-tauri/src/commands/multi_agent/tests/worker_namespaces.rs` | 160 |
| `src-tauri/src/commands/multi_agent/tests/worker_namespace_migration.rs` | 155 |
| `src-tauri/src/agent_runtime/multi_agent/source_rounds/validation.rs` | 95 |
| `src-tauri/src/db/worker_namespaces/source.rs` | 183 |

## 下一步与风险

账本持久键、初始配额读取、费用/并发槽汇总仍需按原 attempt 隔离；未知或缺失回执继续禁止退款/重派。安全重派必须保留原 expired 记录、签发新 UUID/fence/ordinal/child 并原子保留逻辑合同；Reviewer 重派不得创建第二份 canonical review request。尚无生产重派或 Supervisor/真实强杀证据。当前升级只在临时 Native 数据库证明，真实用户库升级与最新安装态验收待框架完成后精确盘点和可恢复备份。Goal 工具返回旧 blocked，项目不是完成状态；继续当前已授权开发，不能标 complete。
