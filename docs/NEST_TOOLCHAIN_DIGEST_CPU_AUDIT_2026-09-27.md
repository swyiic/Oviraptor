# 工具链绑定摘要 CPU 热点修复与质量门禁

日期：2026-09-27。范围：本机已采样的文件摘要热点，不是所有 CPU 问题或整个 Master Plan 的完成证明。

## 1. 原始证据

- 前一轮默认 features Rust 测试 session 99537 已收取 exit 0：主库 1267 项通过，0 失败，801.08 秒。日志 `/tmp/oviraptor-source-coverage-rust-full.log`。该命令不含 `--all-targets --all-features`，未覆盖 feature-gated 历史导入器，不称为 §15 完整门禁。
- 该测试进程的一秒栈采样 `/tmp/oviraptor-source-coverage-web-binding-cpu-sample.txt` 显示，Web 恢复测试中大量采样落在 `web_tool_candidate_binding` 的 SHA-256 路径（703 个样本在 update 调用处）。采样对象为当时实际运行的 `oviraptor_lib-8ba9c62869222da1`。
- 此证据定位了重复读取 Node／浏览器候选文件时的一个热点，不能单独证明用户此前 `oviraptor_lib-4066a088024b7cd5` 达到 360% 的完整原因。旧进程与本轮进程不能混为一谈。

## 2. 实现与不变项

`src-tauri/src/commands/web_toolchain_binding.rs` 仅将大文件流式 SHA-256 的实现替换为项目已经直接依赖的 `aws-lc-rs` 摘要 Context；没有新增依赖、下载工具或改变摘要算法。

保留以下合同：

- 绝对路径、UTF-8 路径和 canonical 解析；缺失文件仍返回原 missing 结构。
- 普通文件限制、512 MiB 单文件限制、2 GiB 总字节预算和候选数量限制。
- Unix 下 `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`。
- 打开前、已打开文件、读取后句柄／路径元数据及 canonical 目标的一致性检查。
- 64 KiB 有界缓冲、完整字节读取、实际读取长度和剩余字节预算检查。
- JSON 字段、文件 stamp、64 字符小写 SHA-256 及派发回执比较语义。
- 预检只读、不执行候选文件。

不新增按路径／mtime 信任的摘要缓存，不减少校验次数，不通过删除授权／完整性门禁提速。现有校验本身不是不可变安装或最终执行时的完整沙箱证明，本修复也不作此承诺。

## 3. 兼容性回归和本机计时

新增 `web_toolchain_streaming_digest_preserves_sha256_schema_and_byte_budget`：

- 空文件、`abc` 两组已知 SHA-256 向量。
- 含所有字节值的二进制输入，SHA-256 的 padding／64 字节边界、64 KiB 读取边界及跨多个缓冲输入。
- 与原 `Sha256::digest` 逐字节结果对应，完整 JSON 结构不变。
- 精确预算和多余预算均正确扣减；不足预算拒绝且不扣减。

测试先在原实现运行通过，再替换实现；这是行为保持的性能修复，不虚构功能红测。已有安装／修改／移除、不执行候选、路径枚举、特殊文件与超限、符号链接改指向、替换后私有回执失效等测试保留。

| 检查 | 结果 | 本机日志 |
| --- | --- | --- |
| 原实现工具链专项，含新增兼容测试 | 7 通过，0.25 秒 | `/tmp/oviraptor-toolchain-digest-before.log` |
| 新实现工具链专项 | 7 通过，0.19 秒 | `/tmp/oviraptor-toolchain-digest-after.log` |
| 原实现真实恢复重建用例 | 1 通过，15.41 秒 | `/tmp/oviraptor-toolchain-recovery-before.log` |
| 新实现同名恢复重建用例 | 1 通过，8.31 秒 | `/tmp/oviraptor-toolchain-recovery-after.log` |
| 精确 all-targets / all-features Rust 门禁 | session 66538 exit 0；主库 1268／导入器 30 通过，0 失败；主库 760.10 秒 | `/tmp/oviraptor-toolchain-digest-all-targets-all-features.log` |
| 严格 Clippy，全 targets／features | session 86742 exit 0，`-D warnings`，16.08 秒 | `/tmp/oviraptor-toolchain-digest-clippy.log` |
| cargo fmt --all --check | exit 0 | `/tmp/oviraptor-toolchain-digest-fmt.log` |
| 实际 UI 组件全套 | session 35710 exit 0，196 通过，0 失败／跳过 | `/tmp/oviraptor-toolchain-digest-ui.log` |
| vue-tsc／Vite 构建 | session 67085 exit 0；主 JS 848.11 kB 分包告警保留 | `/tmp/oviraptor-toolchain-digest-build.log` |
| Native 浏览器本机回环 | session 60353 exit 0；匿名与身份比较均 complete，identity 隔离通过 | `/tmp/oviraptor-toolchain-digest-native-runtime.log` |
| 工作树空白检查 | `git diff --check` 无输出，exit 0；本轮两份未跟踪 Rust 文件及新审计另经 no-index --check，无空白错误 | 本机命令输出 |

恢复用例为 `web_recovery_real_reconstruction_is_readonly_and_preserves_native_web_inputs`；同一工作树、同一台 macOS 主机、相同候选枚举和单测试线程运行，计时不包含编译。单次前后计时包含数据库准备、文件 IO 和缓存影响，不是正式跨平台基准，不设置脆弱的耗时断言，也不能推导生产吞吐、峰值 CPU 或硬资源上限。

## 4. 执行约束和剩余工作

一次仅运行一个 Cargo，使用 `nice -n 15`、`-j 1`、`--test-threads=1`；只在确认前一个命令终态后启动下一个。`nice` 不是 CPU 硬限制。Master Plan §15 的通用命令已逐项串行执行并收取成功终态；默认基线和精确全量的 features／测试数不同，不能拿两次主库总耗时直接归因为单一修复效果。各专项案例已经纳入全量，未为计数重复执行全部阶段过滤命令。

本轮结束时进程检查未发现 Cargo、rustc、主库／导入器测试进程。检查仅证明当时无这些残留，不是持续资源监控。实际浏览器回环仅使用 `127.0.0.1` 夹具，证明匿名采集、身份隔离和禁止越域的现有断言通过；不是 Tauri WebView／IPC、用户目标或跨平台安装包验收。

没有部署、调用外部模型、访问用户 URL、启用主机采集或改变 Web 授权。

总体源码覆盖独立 Reviewer（当前仍只有准备材料）、初评／工具阶段完整恢复、未知结果核对、真实强杀重启、费用／性能及跨平台发布等仍是 Master Plan 未完成项。此修复不把这些状态翻为完成。

## 5. 同轮退役只读检查

本轮按 §14 执行固定路径搜索，未因路径不存在而把扫描错误算通过：

- 广义 `strix` 等关键词搜索保留在 `/tmp/oviraptor-toolchain-retirement-inventory.log`，共 495 个匹配行。该数量不是活动执行路径数，也不是“已全部清零”的证据。
- 旧启动符号组合仅命中 `native_pipeline/tests_native_code.rs` 与 `tests_native_greybox.rs` 的禁止字符串断言；已读取上下文，均为测试防回退检查，不是启动调用。
- 旧更新／启动 Tauri 接口组合在 `src` 与 `src-tauri/src/lib.rs` 无匹配，`rg` exit 1。
- `agent_runtime/multi_agent` 内 `run_native_agent(` 无匹配，`rg` exit 1。
- 旧配置键搜索仍命中迁移映射与历史回归；本轮未改迁移行为、残留白名单或其摘要。广义清单没有在本轮重新逐条语义审查，不把搜索结果当作重新完成全部退役审计。
- 当前全量日志中的 4 项 `retirement_literal_*` 均已通过，核对了固定扫描范围和既有逐文件摘要白名单。本轮性能文件不增加退役字面量豁免。
