# Assets 独立应用：完整原业务迁移台账

本文件记录正在实施的迁移边界，不是最终验收声明。旧桌面 Vue/Tauri 路径继续保留，`apps/assets/app.package.json` 的 v1 库存应用也保留；新的 native v2 包使用独立 manifest、独立二进制和应用私有数据库。安装、启用、项目授权与打开页面是不同操作。

## 现有合同与实际归属

旧桌面注册的 294 个 IPC 中，三个 Assets facade 包含 102 次调用、84 个去重合同。完整表见 [原合同台账](../apps/assets/contracts/README.md) 和 [机器清单](../apps/assets/contracts/legacy-ipc-contracts.json)。数量只用于防止入口遗失，不等于功能验收比例。

| 业务 | 原真实 owner | native 接线 |
|---|---|---|
| 库存、目标、人工有效性、回收/恢复、原分组与成员分页 | `commands/assets.rs`、`assets/statistics.rs` | `assets/service.rs` 受信项目入口调用原 helper |
| 页面隔离预览/应用/停用、规则目录 | `commands/page_isolation.rs`、`asset_rule_catalog.rs` | 项目规则与应用全局规则分别声明权限和 scope |
| CSV 导出 | `commands/asset_export.rs`、Exposure 原导出 owner | 原 CSV producer 后登记 `assets/export_artifacts.rs`，页面仅接 opaque receipt |
| full/collect/reprobe 任务、同 runId 恢复/取消、日志与变化 | `jobs.rs` 及其原子模块 | 应用进程持有真实任务环境、私有路径和 birth receipt；不是页面进程替代实现 |
| 归属档案、组织目录、聚合复核、确认与授权 | `commands/ownership.rs` 的 cohesive include 闭包 | `assets/ownership_service.rs`，原 21 合同 |
| 暴露面配置、采集、证据、线索/事件、人工复核与整改/复测 | `commands/exposure.rs` 及其真实子模块 | `assets/exposure_service.rs`，原 40 合同 |

新 `native-manifest.mjs` 当前声明 101 个有限 operator actions，包含原 84 名称；[动作权限表](../apps/assets/contracts/native-action-owners.json) 和 [类型化 schema](../apps/assets/native-action-schemas.json) 是可审查输入。模型工具只有 `saved_assets_stats` 和 `saved_assets_search` 两项项目内只读投影。完整 UI action、全局组织目录、规则、凭据管理、授权变更和外请求运行不因此进入模型工具目录。

`startup_status` 具有改变历史任务终态的副作用，不能作为普通只读轮询。相关 FOFA 账户测试也不是库存读取或隐式配置保存。Host 不读取资产 SQL，不拥有上述业务规则。Shared/SDK 只承担协议、沙箱、权限准入和字节传输。

## 保留原行为的适配方式

Ownership/Exposure 的 60 个有 State 命令拆出显式可信路径 helper；原 Tauri wrapper、签名、命令顺序与参数保持不变。对实际源码逐函数做反向路径替换后，60/60 原函数体字节相同；另一个无 State 的 `exposure_known_limits` 保持原体。这份证明覆盖路径提取，不能代替编译与运行回归。

额外生产变化单列：两个原 owner 各加载自己的私有后台活动模块，原 `thread::spawn` 调用由保留泛型、返回值与 panic 语义的窄 wrapper 计数。计数按真实 owner 和 canonical DB 路径分桶；应用适配入口绑定 scope，子线程继承 scope，旧桌面无 scope 调用直接使用原 std 行为。两个私有模块不共享 TLS/static。终态数据库行不能掩盖仍活着的工作线程。

`owned_work.rs` 不认领导入或旧桌面的后台历史。原事务提交后才写入应用实例与 birth identity 绑定的 receipt，因此明确不是原子“业务+receipt”提交；receipt 写入失败记为 unknown。旧非终态行缺少有效 receipt 时，先拒绝启动恢复/领养并报告 busy/uncertain。原状态、lease、费用、CAS 和事务不被重写。

## 项目、权限和数据边界

- Host project ID 通过受信映射取得 native project ID，页面只读 `assets_context`，不选择数据库、路径或其他项目。显式冲突的嵌套 query/input/selection 在原 helper 前拒绝；bulk 先完整检查，不能半批跨项目写入。
- 归属确认只写事实。执行授权另经 `inventory.authorization.manage` 和原逐行/快照 CAS。共享组织目录使用 `inventory.ownership.directory.manage` 的 application action，明确可以影响全应用项目，不用 project 0 假装项目写入。
- `inventory.jobs.run` 与 `inventory.exposure.run` 分别允许原采集/探测和暴露面外请求。页面关闭只释放 UI，不取消已接受工作；停用先撤新准入，卸载还要检查实际 busy/uncertain。
- 库存保留完整 AssetQuery、AssetPage、Asset.extra、summary、memberTotal、响应代表锚以及已看状态。人工有效性、候选来源、归属与授权互相独立；历史 probe 时间不能变成当前验证，未知不补 now。

## 独立页面与配置

`apps/assets/ui` 复用同一份 `src/features/assets` 成熟 Vue 业务组件，不复制库存逻辑。应用声明 Vue/Vite 构建依赖，打包单个 inline IIFE/CSS HTML；运行时不依赖 Host/Scanning 实现、桌面 Tauri、全局 i18n 或外部 ESM 请求。构建和打包逐一校验实际源闭包及产物 SHA。

Iframe 仍为 `sandbox="allow-scripts"`，无同源、模态框或表单权限。原确认/命名改走明确 DOM 端口。Exposure 的导入、情报改级、改判三处使用 `role=form` 容器和显式按钮；正常输入 Enter 仍可提交，textarea/select/button、IME、修饰键、重复键和 busy 状态不会多发动作。

执行配置使用应用 action。读取只显示 write-only 凭据的真实 Configured 标志；留空更新按原 owner 保留密钥，私有配置文件只人工选择并发送不超过 64 KiB 的内容，原文不回读，保存仅绑定收到的 UUID。完整已知设置采用 76 个有限字段 schema；原显式 null 保留，未知当前字段阻止保存，不用局部 patch 覆盖整份设置。Vue 编辑对象先还原为普通传输数据再复制，避免 reactive proxy 的 `DataCloneError` 使编辑器无法打开。保存不测试账户、不登录运行时、不启动任务。原模型和其他配置按值保留，配置副本不隐式继承私有导入文件。

导出保留原 CSV producer；新页面的 path 字段是 opaque receipt，不是 native 文件路径。receipt 绑定项目、应用实例、常规文件身份、大小、SHA 和七天有效期；每个最多 192 KiB 的读取重新验证；每块 `nextOffset` 均为整数结束偏移，末块满足 `nextOffset == totalBytes`。Host 只按受控 action 流式传输字节，不直接读 native 文件系统。下载失败可以重试取回同一个 receipt，不重新执行导出；“开始下载”不等于浏览器已经保存成功。

## 当前实际证据与未闭合项

证据目录为 `/workspace/validation/agent-first-20261016/`，不会把旧 checkpoint 转记成新源码通过。

| 记录 | 实际结果与范围 |
|---|---|
| `assets-native-ownership-exposure-prepared/applied-body-proof-and-source-ready-01.json` | 原 61 命令身份保留，60/60 helper 体反向字节等价；单列活动计数和新 adapter 的生产变化 |
| `assets-ui/native-phase2/checkpoint-03.json` | 111 输入前后稳定；37/37 应用 Node、118/118 旧 Exposure Node、app vue-tsc 与独立 UI build 全 exit 0。Node 的 IPC 替身/打包 fixture 不代表 native 服务运行 |
| `assets-ui/opaque-browser-04` | 早期旧 checkpoint 的 7 个真实 Chromium 场景，使用明确合成 parent/action bridge；不是当前新增配置/下载/三个 Exposure 提交面的安装验收 |
| `native-next-stage/phase2/phase2-focused-01-core.log` | 原生 core 首轮 276 pass / 3 fail；其中两项新 fixture 错误为错误 CAS 码与空目录无变更请求，另一个是独立 custody child selector。历史失败保留 |
| `assets-native-ownership-exposure-prepared/source-ready-fixture-fix-02.json` | 仅修新 Ownership 测试夹具与真实错误码，保留原 scope/CAS/不授权断言并增加真实目录/绑定版本核对 |
| `native-next-stage/phase3/phase3-focused-01-aggregate.json` | 原生定向 core 283 与 paid 15 实际全部通过，共 298；3942 个注册 case 中原 3873 名称保留、69 个新增均被定向选中。不是完整原生测试树验收 |
| `assets-ui/native-phase2/checkpoint-05.json` | Vue 实际 reactive 数据回归后，99 个输入在执行前后稳定；39/39 应用 Node、app vue-tsc、独立 UI build 全 exit 0，墙钟分别 2.269 / 3.857 / 3.426 秒。旧 Exposure 118 项对应的业务与 harness 输入仍与 checkpoint-03 相同 |
| `native-browser/full-native-02/result.json` | 真实 Host、新 nativebin、opaque Chromium 共 15 场，12 pass / 3 fail；导出末块偏移不符通用下载合同、真实 Vue 编辑 proxy 异常、以及编辑器未打开而未执行的文件选择后续场景。保留原失败，不能记成 15 场通过 |
| `assets-ui/native-phase2/reactive-profile-red-05.json` | 修复前使用真正 `vue.reactive` 复现 DataCloneError，保留红灯及输入指纹；随后 39 项通过包含此用例 |
| `assets-native-ownership-exposure-prepared/artifact-offset-source-ready-03.json` | 原生末块改为整数结束偏移，原多块字节重建测试追加严格末块断言；此修复晚于上述 298 项验证，等待专门重建和真实下载重测 |
| `native-browser/full-native-04/result.json` | 前一完整源码窗口真实安装的 15 场全部通过，包括新 native CSV 下载、配置编辑/文件导入与七个原业务页；该记录属于 UI05，不能转记成随后偏好/staging/handoff 批次通过 |
| `assets-next-stage/current-ui-101-02.json` | 中间源码窗口 240 输入前后稳定；75/75 应用 Node、158/158 旧 Exposure/plugin 与新增真实共享 SFC 回归，build 与 app vue-tsc 均 exit 0；先于后续交接回执时序修复，不转记到新源码 |
| `assets-next-stage/current-ui-101-03.json` | 最后业务修复与 SDK 交接 ACK 冻结后，HEAD `83429789` 前后相同、240 输入稳定；78/78 应用 Node（7.429 秒）、158/158 旧组与新增共享 SFC（18.868 秒），build 0（3.835 秒）、app vue-tsc 0（4.098 秒）。67 构建依赖与 509926 字节 inline 页面均按 SHA 绑定；不包括新原生 13 项或随后真实安装浏览器 |

早期 Linux devbin 的 clean-env 动态链接检查发现 GTK/WebKit 依赖，`headlessReady=false` 的失败记录保留。显式事件/活动回调拆分后，`native-next-stage/phase3/phase3-dev-clean-runtime-gate.json` 已记录两应用缺失动态依赖均为空、ready=true；这是依赖就绪证明，不等于安装验收。随后真实安装运行的 full-native-02 已跑到资产列表、项目隔离及七个成熟业务页读取，但仍有上述三场失败，修复后须在新的完整输入快照上重跑。原 Node v1 数据或其他未登记目录会触发 `asset_service_store_migration_required`，不能在旁边新建空库冒充升级成功；v1→native 的数据迁移仍需独立 owner receipt 与真实升级回归，fresh 安装通过也不自动证明这一点。

下一批已实际接线、尚待新原生/安装浏览器验收：

- 应用私有 `preferences_service` 只持久化原四个固定布局字段，38 个真实列名、列宽 68..720；完整快照绑定原实例并做 CAS。读取不初始化、不写入；初次不可用保留本页布局，冲突或写结果未知不自动覆盖远端、不中途自动重试。筛选 query 仍只在本页会话保存。13 个新原生测试已准备，不预填执行结果。
- `ui.external.open` 只让父窗口人工审阅并提供外链；offered/cancelled 都不写“已打开”。旧桌面 opener 的 void 返回仍执行原标记路径。`ui.handoff` 也不进入模型工具。
- `prepare_scan_selection` 从可信 native 项目选择最多 1000 个明确资产 ID，返回原生登记的来源 receipt、准确 URL、mode 与 digest；唯一 URL 上限沿原业务为 200，超过该限制或 512 KiB 明确拒绝拆批，不截断。页面先将用户选定 mode 写入原四键偏好并等待同一 store 的 CAS 写入结果；unknown/conflict 时不静默跳转，取消可保留已选模式。页面仅送到真实 Scanning `home` 待审阅，不能伪造扫描 ID、草稿历史或授权。收到受理回执后同步清选择并显示待审阅，不再等待资产刷新；父级收到绑定 requestId/nonce 的 SDK delivery ACK 后才切换页面，避免回执刚返回就卸载原页面。只有父级只读可用性快照确认双方安装、启用与项目 grant 后才开放入口；真正交接仍重新校验。
- Exposure 原限制仍为 20 文件、每个 2 MiB、整批 8 MiB。新的五个有限 operator actions 在页面首次 postMessage 前对完整文件做 SHA256，再用至多 192 KiB 原始块传输，经原生完整大小/摘要核对后仅提交一次原 import owner。单条 operator 1 MiB 请求上限不被放宽、不用 JSON 字符串绕过 typed schema。allChunksRecorded 不等于 contentVerified；接收中续传/丢弃均需显式动作，未知 commit 结果不自动重试、不换 UUID 再导入，原生 expired 状态明确显示过期。关闭页面不撤销已接受的原导入。

当前 typed action 为 93 + 5 staging + 2 preferences + 1 source selection = 101，模型工具仍仅两项只读投影，声明权限为 14 个。新原生 dispatcher 尚未执行的用例不由 Node API 替身代替；原 Node v1 数据迁移仍须独立 owner 证明与实际升级回归。

新质量仪表盘、AI 精炼与增量发现按用户 2026-10-16 后续指示暂停实现，列为拆分完成后的待办。没有新 Dashboard 代码或新扫描/付费 AI 动作；既有原任务管线不因这条待办而改写。候选库存不是全网覆盖，confirmed 也不自动表示主体归属或授权，这些语义应在后续设计继续保留。
