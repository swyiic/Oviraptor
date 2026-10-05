# 后台 Worker 到期监督审计（2026-10-02）

## 问题和作用域

此前只有回调或显式清理可原子撤权；无回调的到期worker没有后台撤销。未来1秒期限后等待自然时间的真实合同0/1复现仍running，随后接生产IMMEDIATE撤权。此测试主动设置未来worker期限，Coordinator仍有600秒有效期，不证明常规统一600秒签发期限、Root期限治理或独立OS进程完整恢复。

服务仅绑定现有数据库的当前Native/multi、自根、无parent/assignment的Coordinator；禁止创建数据库或初始化schema。固定原actor，接管后原服务报stale，允许同Root新actor撤回旧权限，跨Root不修改原worker。未知模型声明保留原预算/并发槽、Root准入继续拒绝。服务不拥有模型/目标能力、不续租、不重派、不退款、不结清未知费用。

## 最小实现和真实入口

`supervisor.rs`负责线程、停止通道、失败状态及Root限定sweep。闲时只读检查，发现到期才取得写事务；writer短暂争用保持服务健康、等待下一轮，不写半成品。每次事务复核当前Coordinator和Native Root，所有到期child调用既有完整原子撤权证明，最后写后再核验，保留协作事件提交通知。缺失current worker、坏期限/角色/绑定和真实写故障拒绝，不补历史权限。Drop唤醒并join服务。

Web从Mapper准备前启动，到会话结束持有服务，Identity/公开入口/Executor/Authorization/Review及交付边界检查故障。Executor先保存原财务结算，再拒绝监督失效后的普通业务发布。Source实际producer从Root激活后持有服务，在初始分析/工具/候选及覆盖Reviewer/收口边界检查状态；错误仍保留真实原费用与未决资源。

## 失败与门禁

- 自然过期stub合同0/1，修复后1/1；初始5/5不能代表最终接线。
- 首次接线扩大370项因多项Source失败主动停止，exit101；单独真实候选审核0/1、3.60秒确认`worker_supervisor_lock:database is locked`。修为只读预查与争用等待后7/7；正常0.65秒writer持锁后撤权也已验证。
- 缺worker首次负测在夹具删除被不可删guard拒绝，未复现待修合同；修正为损坏恢复夹具后实际0/1，再补服务拒绝。最后9/9、9.47秒，含8项新监督合同与一项既有Source真实审核。
- 最后格式后扩大373/373、exit0、1588.72秒；严格全目标全特性Clippy exit0、13.75秒；导入工具39/39、2.34秒；作用域fmt/差异及代码摘要逐文件核对通过。集合重叠，不相加，不是最终全量/整体验收。

最终862路径摘要`5fafbc33b2730a73fbb61daaa31b7a36abade5766c48e831231ee975956b2f22`，HEAD仍`59be3d86d25adda5b1f975759bf256ef3b94f32b`。日志、修改前文本、scope和摘要在`/tmp/oviraptor-worker-supervisor-*`。7个已有文件的既有未提交内容保持，新增2个文件，无删除、重置或自动提交。

## 文件与体量

- `src-tauri/src/agent_runtime/multi_agent/mod.rs`：30行。
- `src-tauri/src/agent_runtime/multi_agent/supervisor.rs`：197行。
- `src-tauri/src/commands/agent_tests_multi_agent.rs`：53行。
- `src-tauri/src/commands/multi_agent/execution.rs`：237行。
- `src-tauri/src/commands/multi_agent/prepare.rs`：322行。
- `src-tauri/src/commands/multi_agent/review.rs`：356行。
- `src-tauri/src/commands/multi_agent/tests/worker_supervisor.rs`：273行。
- `src-tauri/src/commands/multi_agent_runtime.rs`：23行。
- `src-tauri/src/commands/native_source_assessment_authority.rs`：308行。

新实现197行、新合同273行；其余按原业务模块接线，最大356行，未新增重复策略或迁移。既有撤权和原费用证明复用，不移资产职责到监督层。

## 未完成与数据风险

这只是到期监督组件。父监督退出/故障的在途立即阻断、完整Root clock/Coordinator存活治理、OS进程真实生产调度与恢复、所有角色/模式、Source/Reviewer/canonical请求和跨Coordinator预算恢复尚未交付。Root与worker同时到期时固定actor会停止并拒绝新执行，需要后续控制面恢复，不能据此宣称完整Supervisor已完成。

完整预算/人工对账、聊天四态闭环、逐路实时日志及最终完整门禁、当前安装态和两个授权URL继续开发。Rust验证仅临时库和localhost；另按用户新授权只读盘点真实库并保存备份、在备份隔离副本试验旧数据清理，见独立数据盘点审计。真实业务行、资产、CAS、安装包未改，没有提交。Goal旧工具状态不能当作完成或停止工作的理由，继续Master目标。
