# Web 启动配置绑定与派发复核审计（2026-09-26）

后续人工恢复状态见 `NEST_WEB_MANUAL_RECOVERY_AUDIT_2026-09-26.md`：已有严格限定的普通 Web、从未派发任务恢复入口。下文「恢复 API／UI 未交付」保留为原绑定增量的历史快照，不代表所有后续代码；已领取／未知副作用、其他任务类型与完整桌面验收仍未完成。

后续修正及工具入口清单／即时熔断增量见 `NEST_WEB_TOOLCHAIN_BINDING_AUDIT_2026-09-26.md`。本轮两次旧全量快照实际均为 848 成功／1 个 fixture 路径失败，不是 849 全绿；失败原因、修复和最新全量门禁以该后续审计为准。下方 13 项为原绑定测试范围，后续另增加派发熔断测试。

## 1. 结论与交付范围

本增量承接 `NEST_BRANCH_DISPATCH_RECEIPT_AUDIT_2026-09-26.md`，给**普通 Web 确认／续跑／重试产生的新 attempt**增加启动输入的私有绑定，并在首次派发领取事务中复核。它不是恢复按钮，也不把已有 claim 清空，不补造历史记录，不新增 Host Agent，不改变历史 JSON 的只读兼容。

解决的问题：有“从未领取执行权”的凭据，还不能说明可以用当前配置执行。启动提交到实际工作线程启动之间，模型、代理、身份、策略、目标或文件可能已经变化。原 Web pipeline 还会在启动浏览器后重新读取全局 settings，使一个 attempt 混用不同配置。本增量绑定输入，拒绝未确认的替换，并让普通 Web worker 使用启动时传入的 settings 与前端时限配置。

**目标仍进行中。** 同 attempt 的人工恢复 API／UI、重新授权与熔断预检、已派发未知结果对账、源码／CI／combined 的完整启动事务、完整桌面强杀验收仍未交付。本增量不能作为这些项目的完成证据。

## 2. 新增数据与隐私边界

`native_web_dispatch_bindings` 保存：

- `scan_id`、`attempt_number`、固定 `branch=web`。
- `schema_version=1`。
- 固定 32 字节的 `binding_tag`（HMAC-SHA256）。
- 外键指向同一个分支的持久派发记录。

每个新 attempt 独占目录另有随机 32 字节 `.web-dispatch-key`。首次独占创建，不覆盖；Unix 模式为 0600，目录为 0700。验证缺少密钥时直接拒绝，不能自动重建或重新签发凭据。初始化旧数据库不回填绑定。

包含密钥、代理认证、完整模型设置的 descriptor 仅留在内存；数据库不复制这些原文，不返回到 IPC，也不进入诊断日志。错误码不包含摘要输入。HMAC **不等于对现有 auth JSON 的加密**：既有身份文件仍遵守原来的存储合同。本功能也不承诺抵御已经能同时修改数据库、密钥和应用代码的本地管理员。

## 3. 实际绑定内容

1. scan／attempt／工作目录路径；Unix 还绑定目录设备号和 inode。
2. 任务归属、任务类型、执行面相关输入、任务文件、技能名称、当前 attempt 与状态。
3. Web context 的环境、认证描述和策略；项目身份与活动状态。
4. 本 attempt 的执行模式、状态、work_dir 和后端计划。
5. 目标 ID、归属、公司、URL、状态及所属 attempt。
6. 生效的 settings、模型运行参数、代理／noProxy、预算、有效 Web 策略、技能指令、worker 路径。
7. 验证过的 bundled worker、当前运行应用文件 SHA-256、应用版本、runtime PATH、实际前端时限配置。
8. 浏览器／Node 相关的选定环境变量，包括浏览器路径、CDP transport、NODE_OPTIONS／NODE_PATH 与浏览器发现目录。
9. marker、task、targets JSON／TXT、指令文件、可选 prompt audit、可选身份文件的内容摘要与存在性。

不绑定 UI 摘要、`updated_at`、项目描述，避免无关展示更新阻止派发。settings 当前按整份生效对象绑定；改变其中无执行影响的字段也可能要求重新准备，尚未做字段级差异解释。

认证文件同时与当前身份存储重新比对，重用现有身份校验函数检查项目归属、有效状态、到期时间、认证材料及多身份互异性。不能仅凭落盘 auth JSON 判断身份仍可用。

## 4. 启动提交与文件保护

普通 Web startup 保留已有 scan 控制锁与 IMMEDIATE 事务，并对启动提交连接设置 FULL 同步级别。

先准备启动文件、写入任务／目标／attempt／branch／dispatch，然后创建绑定：

1. 对 startup 独占目录校验 canonical 路径及身份。
2. 读取有界文件；Unix 通过目录描述符 `openat`，禁止跟随最终 symlink；拒绝非普通文件、硬链接、多余权限的密钥、超限文件。FIFO 以非阻塞方式打开后拒绝，避免验证挂住。
3. 同步已读取启动文件及新建密钥；Unix 同步 attempt 目录和新建父目录。
4. 严格 INSERT 绑定；拒绝 IGNORE、覆盖或缺失写入。
5. 重新计算并验证实际数据库、文件和 tag；检查 dispatch 仍然未领取。
6. 成功后进入原有 commit→launch 流程。

已知提交前错误回滚数据库，只清理原 startup 独占目录里的已知文件；新增密钥加入该列表。未知 commit 结果仍保留文件、不派发，不凭异常猜测“肯定未提交”。未做硬件掉电测试，不能据文件同步调用宣称硬盘故障下全链路已经验收。

## 5. 首次派发的实际执行边界

普通 Web launcher 携带 `WebDispatchAdmission`，包含启动时的 settings、前端时限配置及 runtime PATH 重建所需目录。工作线程先调用 `NativeBranchGuard::claim_with_preflight`：

1. 取得原 scan／attempt／branch OS 活锁。
2. 新连接 FULL + IMMEDIATE 事务。
3. 用当前数据库／模型环境／worker／配置重新构造 descriptor，验证原绑定。
4. 进行已有的一次性 claim CAS 与准入检查。
5. 再次校验绑定，防止 claim 的 AFTER trigger 修改配置后仍交付。
6. 成功提交才构造有失败清理责任的 guard。
7. 才能进入模型资源策略、前端 producer、浏览器与目标执行。

绑定校验本身不是派发授权；它可以用于 claim 同事务的前后复核，不能替代 claim CAS。已经领取的记录即使配置仍一致，也不能重复派发。通用无 preflight 的 `claim` 遇到已绑定 Web attempt 会拒绝，避免新路径意外绕过绑定。

校验失败没有执行 guard，不改另一调用者的终态，不把未执行任务标成扫描失败，不清空 claim。原启动日志记录拒绝原因。**尚未增加“配置发生变化”专用状态卡和人工恢复入口**，不能承诺用户已经能在 UI 直接恢复该 attempt。

普通 Web pipeline 不再在启动浏览器后重新读取 settings，producer 不再自行重新读取前端时限 JSON。源码／combined 仍保留明确的旧调用方式；不得宣传所有任务类型都已冻结。

## 6. 本轮自动化覆盖

新增 `tests_web_dispatch_binding.rs`，13 项测试，使用临时数据库／文件，无外部网络。

- 成功提交、连接重开后只读校验、私有 key 权限、验证不产生 claim。
- 模型凭据、端点、独立 Reviewer 配置、代理、预算、runtime PATH、build 变化拒绝；不把原文放入错误。
- INSERT ABORT／IGNORE、AFTER 删除／tag 篡改／目标／context／任务文件／生效 settings／claim 篡改回滚整个 startup。
- 老库不回填；缺失或错误 key 不重新生成。
- 每种启动文件变化、必需文件缺失、非预期 auth 文件拒绝。
- 暂停、替换 attempt、任务类型、策略、目标、后端计划、项目归档、准备租约、删除墓碑及缺失 dispatch 拒绝。
- 身份失效、过期／坏时间、跨项目、材料变化或删除拒绝。
- Unix symlink／硬链接／FIFO／超大文件／密钥权限拒绝；替换目录即使复制原密钥和全部文件也不能通过 inode 绑定。
- 延迟外键导致 COMMIT 失败：数据库无新 dispatch／binding，保留 owned 文件，不返回执行权。
- 生产只读 runtime resolver、不落库修改策略、真实 bundled worker 与真实运行应用摘要被纳入 descriptor。
- 使用生产 live preflight＋guard：配置变化无 claim／无失败 cleanup；claim AFTER trigger 修改配置整体回滚；恢复原配置后只可领取一次；释放 OS 锁也不能重放。
- 不同 attempt 目录随机 key／tag 不同，搬运密钥与 tag 不能跨目录通过。
- 重复注册不覆盖 tag、不轮换 key。

测试没有启动完整 Tauri 扫描、没有做本增量的真实进程强杀或断电实验，也没有验证 Windows 文件系统边界。前一增量的真实子进程 claim 强杀测试继续作为独立回归，不扩大其证明范围。

全量回归另发现旧强杀测试只检查非零退出，stdin 在 `Child::wait` 前后关闭时可能出现 EOF panic。现单独持有 stdin writer 直到退出，检查 kill 成功，并在 Unix 断言 SIGKILL，而不是接受任意失败退出。该修正加强已有三阶段 claim 强杀证据，不替代本增量尚缺少的完整应用强杀验收。

## 7. 质量门禁与下一步

本轮日志前缀为 `/tmp/oviraptor-20260926-binding-`。两次全量均为 848 成功／1 失败：旧任务测试使用 macOS 非规范化临时路径，提前触发目录检查。严格 Clippy／fmt、UI 76 项、构建和本地浏览器回环通过。后续已修正 fixture，最终新快照的完整验收见 `NEST_WEB_TOOLCHAIN_BINDING_AUDIT_2026-09-26.md`，不得把中途定向结果或旧全量失败写成全绿。

下一步仍是显式同 attempt 恢复，而不是把失败任务偷偷改成新任务：

1. 人工动作携带明确 scan／attempt，取得生命周期锁与分支锁。
2. 先证明从未 claim；旧记录缺失、已有 claim、结果未知都不得走这条恢复路径。
3. 重新检查项目／授权／目标／熔断／环境租约／身份／预算／后端计划与现有绑定。
4. 完成工具供应／可执行文件身份策略：本轮绑定 worker／应用及 PATH，但**没有绑定 PATH 中 Node 和浏览器二进制的完整内容**，不得据此声称完整工具链封存。
5. 同事务 claim，交付已经拥有执行权的 guard，避免恢复与原 launcher 竞态造成两次工作。
6. 实际 UI 动作、防迟到回包、只读轮询不重派、故障注入及完整桌面强杀验收。

不能把本文件的校验函数直接暴露为“恢复成功”API，也不能只因为有 binding 就让 UI 的 `automaticReplayAllowed` 变成 true。
