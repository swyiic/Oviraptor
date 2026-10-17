# 工作台浏览器身份重试与发布核对审计

状态：灰盒浏览器认证准备／发布增量；不是工作台完整冻结派发、凭据全生命周期或主机执行实现。

## 1. 已确认的问题

`code_analysis.rs` 的认证回退原先直接复用旧 `auth-session.json`。单身份 `schemaVersion=1` 只恢复 `auth_session_id`，没有恢复用于策略与归属校验的 `auth_session_ids`；矩阵恢复 ID 后也直接复制历史凭据，不重查当前会话是否有效、是否过期以及是否仍属于任务。依赖准备期间发生的身份变化，发布事务原先只核对归属，没有核对认证材料与准备文件的一致性。

## 2. 实际实现

- 新增生产 `workbench_auth.rs`，灰盒首次准备、显式身份重试、历史浏览器身份回退共用当前会话读取；读取在同一 SQLite 快照中完成，避免多个身份来自不同读取时点。
- 历史 JSON 只提供身份 ID 提示。单身份／矩阵统一返回完整、有序身份列表，并从当前数据库重建认证文件；不复制历史 Cookie、Header 或 Storage 到新 attempt。
- 重试要求身份属于当前 project 和 scan，且已不属于草稿；有效状态、数据库期限、文档期限、schema、文档 ID/project、非空 scope 都须通过。矩阵继续要求不同认证材料及共同捕获 scope。
- 空身份、重复身份、超过五个身份、损坏／未知浏览器 schema 不会降级为匿名或原始 Header 认证。旧原始 cookie/bearer/header 文档仍交给既有 type/value 检查；本增量没有为它们新增浏览器会话续期能力。
- `publish_workbench_start` 在所有事务写入／触发器之后、提交之前，重查当前身份和准备文件深度相等、任务认证上下文及 policy 身份集合一致。失败使任务、attempt、身份绑定、目标、分支与派发槽位全部回滚，线程不会因本次发布启动。
- 新 attempt 准备／发布失败不改写旧 attempt 文件。更换凭据后，需要重新准备认证文件；不修改过去证据冒充当时用了新身份。
- 新的文件／版本不匹配错误使用固定错误码，不把认证 JSON 或凭据值写入错误信息。

## 3. 验证范围

新增 8 项真实生产 helper／发布事务测试，均为临时本地数据库，不访问外部站点：

1. 单身份恢复完整 policy handle，使用更新后的当前凭据及名称，原文件字节不变。
2. invalid/capturing/needs_check、过期／损坏期限、其他任务／未绑定／草稿、其他项目、删除身份全部拒绝。
3. 显式身份重试走相同归属与文档校验，损坏数据库文档不被接受。
4. 单身份／矩阵结构损坏、空／重复／非字符串 ID、未知版本及伪装原始认证不能绕过；既有原始认证兼容路径不变。
5. 两身份恢复稳定顺序；当前材料重复、没有共同 scope 或超过五个身份拒绝。
6. 真实发布触发器在末尾使身份失效／过期、改变凭据、篡改文档 ID/project/scope/期限，全部回滚；移除故障后可正常发布。
7. 准备文件缺失／损坏／不一致、身份列表丢失、policy 不一致、认证标记／类型冲突，全部拒绝发布。
8. 第二次 attempt 的旧文件与当前凭据不一致时拒绝；历史 attempt 保持不变，重新准备后才发布第二次 attempt。

共享工作台 fixture 已改为真实有效的浏览器身份及私有认证文件，不以空文档绕过新检查。首次编译因共用 policy 身份解析器为模块私有而失败；将其改为 crate 内可见以复用同一语义，没有复制或放松规则。日志 `/tmp/oviraptor-workbench-auth-targeted.log` 保留，这不是修改前红测证据。

修正后工作台组合定向 **28 项通过**，见 `/tmp/oviraptor-workbench-auth-targeted-v2.log`。定向后的严格全目标／全功能 Clippy 与 fmt 检查通过，见 `/tmp/oviraptor-workbench-auth-clippy.log`、`/tmp/oviraptor-workbench-auth-fmt.log`。

最终 Rust 顺序流水线 session 9772 已退出 0：

- `cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- --test-threads=1`：主库 **1007 项**、历史导入 **30 项**通过，零失败；主库用时 497.98 秒。日志 `/tmp/oviraptor-workbench-auth-full.log`。其中包含静态退役字面量／活动符号守卫、CLI 陷阱、历史导入、多智能体及用户指令既有回归；没有据此宣称未覆盖的打包／真实目标验收完成。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings` 通过，日志 `/tmp/oviraptor-workbench-auth-final-clippy.log`。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --all --check` 通过，日志 `/tmp/oviraptor-workbench-auth-final-fmt.log`（空输出）。

完整回归开始后直到终态没有修改／重新编译 Rust，避免干扰绑定测试对真实执行文件的校验；没有重启测试或更新退役 allowlist。

前端及浏览器组合流水线 session 15895 已退出 0：七组实际 UI 测试 **146 项通过**、`npm run build` 通过、`node tools/test_native_runtime.cjs` 的双 localhost 真实浏览器回环通过。日志分别为 `/tmp/oviraptor-workbench-auth-ui.log`、`/tmp/oviraptor-workbench-auth-build.log`、`/tmp/oviraptor-workbench-auth-loopback.log`。主 JS 828.82 kB，拆包警告仍保留。`git diff --check` 通过；本轮未更改前端行为。

## 4. 未覆盖及后续要求

后续实现更新：下文记录的重试 instruction／skill 名／损坏 JSON 默认值问题，已由 `NEST_WORKBENCH_RETRY_INTENT_AUDIT.md` 修复并单独验证。该后续还在完整 policy fixture 下发现并修复认证单身份字段掩盖空多身份列表的问题。此处原验证结果保留为当时增量证据，不代替后续终态。

- 本增量的核对截止点是发布事务提交前。提交到实际派发之间，以及执行中的配置／凭据撤销、磁盘文件变化，还需要工作台自己的完整冻结 binding 与每次工具操作前验证。普通 Web binding 的结论不能直接套用。
- 另行复核到 `scan_control.rs::rescan_workbench_scan` 仍把人工 instruction 重建为空串，按历史 skill 名查询当前 ID，且 task JSON 读取／解析失败会默认空对象。它们不由认证刷新修复；下一步应使重试严格恢复可验证的原任务意图／预算／skills，损坏或不一致时明确拒绝，不使用默认值扩大执行范围。
- 不替代目标精确 scope 授权、工具链版本绑定、完整预算和进程撤销合同。
- 原始手工 cookie/bearer/header 文档没有浏览器会话库的有效期／撤销元数据；没有宣称该兼容路径已经获得同等生命周期保障。
- 历史 JSON 保留用于回看；没有当前有效且正确绑定的身份时，不能仅靠旧文件重新获得执行权限。
- source／greybox 恢复、所有专家执行、知识／skills 生命周期、资产分析及桌面／授权环境验收仍待 Master Plan 继续推进。

本轮未部署、未访问用户提供的外部 URL、未新增 Host Agent；主目标保持未完成。
