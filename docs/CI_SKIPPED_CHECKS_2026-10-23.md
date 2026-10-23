# 云端 29 项跳过检查的逐项核对（2026-10-23）

核对基准：main `748325f33f00091fac8c5704607b6a74ce7191f0`，[云端原始完整运行](https://github.com/swyiic/Oviraptor/actions/runs/39966804414)。以下补跑均未改变 VPN、真实配置、授权、资产或聊天记录。

## 结果与边界

- 12 项 macOS 构建证据检查 + 4 项 macOS Agent 执行检查：本机全部通过；新增独立 `macos-execution` 云端任务，避免 Linux 上的跳过掩盖平台覆盖空缺。这是测试夹具、沙箱和实际本地进程验证，不代表又完成了一次应用打包或生产模型验收。
- 3 项真实项目检查：从只读数据库连接生成一致性快照，审查范围只读快照；分流与种子索引仅写独立副本，全部通过。未运行模型或 Hunter 搜索，未改变确认范围。
- 1 项公开官方网站检查：成功读取中国移动官方公司简介，并核验两家法定实体的控股关系。它不证明其他候选域名的归属，也不扩大授权范围。
- 1 项 Hunter 连通性诊断：当前网络下 TLS 握手 EOF，未到达可验证账户响应；没有查询资产、自动重试或消耗搜索积分。诊断测试本来允许打印网络失败后退出 0，因此这里明确记为外部能力未验证，不能把退出 0 当作 Hunter 搜索成功。
- 8 项 HackerOne：用户已要求停用，保持归档跳过；不重新启用，不请求其接口。

因此，原先跳过的 29 项中，20 项已补跑通过、1 项外部网络仍未验证、8 项保持功能停用。

## 逐项清单

| # | 类别 | 原测试名 | 补跑结果 | 源码 |
|---|---|---|---|---|
| 1 | macOS 构建证据 | `successful local observation records actual command, stable inputs and fresh output without claiming acceptance` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 2 | macOS 构建证据 | `source changes during a successful builder invalidate the observation` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 3 | macOS 构建证据 | `failure, signal and successful no-op cannot adopt a preexisting bundle` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 4 | macOS 构建证据 | `unsupported configuration and preflight errors never invoke the builder` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 5 | macOS 构建证据 | `builder exceptions and output symlinks remain failed local receipts` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 6 | macOS 构建证据 | `real subprocess runner records log and terminal exit using a synthetic CLI` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 7 | macOS 构建证据 | `observation verifies three local snapshots without claiming release acceptance` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 8 | macOS 构建证据 | `observation distinguishes source, recorded bundle and installation drift` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 9 | macOS 构建证据 | `observation rejects unsuccessful, inconsistent and redirected receipt claims` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 10 | macOS 构建证据 | `observation checks saved manifest bodies and refuses malformed or missing evidence` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 11 | macOS 构建证据 | `observation rejects evidence links, oversized receipts and linked output directories` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 12 | macOS 构建证据 | `observation CLI separates matching, drift and invalid evidence exits` | 本机补跑通过；新增 macOS CI | `tools/test_build_receipt.cjs` |
| 13 | 已停用 HackerOne | `archived HackerOne: a configured non-default account is usable when the default has no credentials (feature suspended)` | 保留停用，不恢复 | `tools/test_hackerone_profile_selection.cjs` |
| 14 | 已停用 HackerOne | `archived HackerOne: explicit account drives both program sync and detail refresh without changing defaults (feature suspended)` | 保留停用，不恢复 | `tools/test_hackerone_profile_selection.cjs` |
| 15 | 已停用 HackerOne | `archived HackerOne: missing credentials retain cached browsing and emit a configuration destination, without sync (feature suspended)` | 保留停用，不恢复 | `tools/test_hackerone_profile_selection.cjs` |
| 16 | 已停用 HackerOne | `archived HackerOne: only safe profile metadata and availability are exposed to rendered account controls (feature suspended)` | 保留停用，不恢复 | `tools/test_hackerone_profile_selection.cjs` |
| 17 | 已停用 HackerOne | `archived HackerOne: in-flight synchronization captures one account and drops stale UI completion after selection changes (feature suspended)` | 保留停用，不恢复 | `tools/test_hackerone_profile_selection.cjs` |
| 18 | 已停用 HackerOne | `archived HackerOne: credential revocation during synchronization invalidates success without another network call (feature suspended)` | 保留停用，不恢复 | `tools/test_hackerone_profile_selection.cjs` |
| 19 | 已停用 HackerOne | `archived HackerOne: choosing an unconfigured profile remains explicit and never falls back to another account for sync (feature suspended)` | 保留停用，不恢复 | `tools/test_hackerone_profile_selection.cjs` |
| 20 | 已停用 HackerOne | `archived HackerOne component activity isolation (feature suspended)` | 保留停用，不恢复 | `tools/test_plugin_workspaces.cjs` |
| 21 | macOS Agent 执行 | `actual isolated execution returns failure evidence, supports repair and publishes successful outputs only` | 本机补跑通过；新增 macOS CI | `packages/agent-host/test/chat-reliability-execution.test.mjs` |
| 22 | macOS Agent 执行 | `isolated script cannot read private files, write outside, connect network, or publish after timeout/revocation` | 本机补跑通过；新增 macOS CI | `packages/agent-host/test/chat-reliability-execution.test.mjs` |
| 23 | macOS Agent 执行 | `execution acceptance requires the exact current-run successful host receipt` | 本机补跑通过；新增 macOS CI | `packages/agent-host/test/chat-reliability-execution.test.mjs` |
| 24 | macOS Agent 执行 | `macOS execution denies detached child processes and Python uses the same output boundary` | 本机补跑通过；新增 macOS CI | `packages/agent-host/test/chat-reliability-execution.test.mjs` |
| 25 | Hunter 外部网络 | `asset_discovery::hunter_diagnostics::tests::live_saved_account_connectivity_probe` | 未验证成功：TLS handshake EOF；只调用 userInfo | `src-tauri/src/assets/hunter_diagnostics_tests.rs` |
| 26 | 官方页面外部网络 | `asset_discovery::organization_evidence::tests::live_official_company_relationship` | 实测通过：两家法定实体关系正文 | `src-tauri/src/assets/discovery_organization_evidence.rs` |
| 27 | 真实项目快照 | `asset_discovery::review::scope::tests::real_project_scope_snapshot` | 一致性快照/独立副本补跑通过 | Rust 测试名见左列 |
| 28 | 真实项目快照 | `asset_discovery::review::tests::replay_local_triage_on_disposable_project_copy` | 一致性快照/独立副本补跑通过 | Rust 测试名见左列 |
| 29 | 真实项目快照 | `asset_discovery::seed_page::tests::real_snapshot_seed_history_preserves_source_data_and_unknown_first_round` | 一致性快照/独立副本补跑通过 | Rust 测试名见左列 |

## 四条关键流程的额外复测

- 聊天/文件：跨会话消息游标、文件完整读取、不可变版本、运行权限迁移；本机 macOS 两个专属文件合计 25 项通过，无跳过。
- Agent 执行：真正启动受沙箱约束的 Node/Python；验证先失败后修复、失败输出不发布、越权读写/外连受阻、超时/撤权停止、禁止分离子进程、仅当前运行的成功证据可完成任务。
- 证书补采：Host 与原生任务桥、项目绑定、独立采集权限、暂停/恢复、真实本地 TLS 握手、只取证书不发 HTTP、DER 指纹入库与证据回流；不依赖真实目标网站或付费 API。
- 任务恢复：重启后的原目标/计划/证据、未知写操作不自动重放、过期授权不能影响下一轮、冻结范围与游标、终态写入失败保留可恢复记录。Host 关键专项 34 项补跑通过，无跳过。

扩大原生复测时发现 3 项测试夹具把 macOS `/var` 临时路径别名当作正式存储路径，触发了正确的目录防护；仅将两处测试夹具创建的根目录转成 canonical path，不修改生产安全逻辑。修复后重新执行 `jobs::` 全组；不能通过关掉 symlink/alias 检查来消除失败。

## 尚不能声称已覆盖的线上行为

本地 TLS/HTTP fixture 和恢复测试验证实现路径，但不能替代用户网络下的真实目标 TLS 可达性、实时 CT/DNS 数据完整性、FOFA 额度与 Hunter 搜索/分页/计费结果。Hunter 真实线上搜索与扣费本次未执行。失败诊断留在私有本地记录，不上传 Key、账户、完整数据库或项目证据到 CI。
