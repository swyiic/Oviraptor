# 原付费 Root 决策到聊天游标及真实前端消费者审计（2026-10-03）

状态：本批相关门禁完成；Master、完整智能体框架及最终功能验收未完成。

## 问题与实际作用域

原 Root SDK 的已付费决策有独立 model event sequence，却未进入实际聊天 collaboration sequence。旧状态/历史消费者不能展示完整公开决策，也不能把模型事件序号当聊天游标。仅临时 SQLite 与 localhost 真 SDK，原 Web Multi creator/C/control/账单/Native JSON；无真实数据库、CAS、资产、安装、授权 URL、提交或重置操作。

新 publication 在同一 IMMEDIATE 事务绑定原 call/control/model event 到唯一 committed collaboration sequence。原 request/publication/model_completed JSON 字节及完整物理 eventProof 保留；两个新增直插入口服从原 authorizer，触发器额外业务写或外 scope 写仍被拒绝。通道或映射 IGNORE 都拒绝成功并回滚 publication，已付费用和保存语义保留，恢复原数据后同 API 本地补发、零额外 SDK。

独立 immutable mapping 的四类唯一键拒绝 INSERT OR REPLACE，更新/删除拒绝；collaboration sequence 与 entity 的替换也拒绝。未把旧缺 mapping 的 publication 自动收编或读侧补写。历史/状态在调用者单一 WAL 快照校验原 owner/Native/full scope/原 request/hash/dispatch/reserve/usage/所有成本行/summary/publication/完整 eventProof，再核当前 committed channel。分页过滤之前审计本 scope 和跨 scope 的 call/entity/cursor alias，损坏原 model event、孤儿 channel/mapping 和未知付款拒绝。原权限失效不被保存聊天重新授权；读取不使用当前 C 取得执行权、不回调 SDK、不 ACK。

真实 AgentDialog、composables、RootDecisionSummary 和 projectionContract 已接结构化公开摘要卡片，仅选择五类 bounded public summary 与原账本 usage。严格验证七字段 record、五字段 usage、原 Root/tuple/call/thread/cursor/status/persisted；损坏记录在提交游标和 read ACK 前拒绝。原 trace 卡片和主题保留；无 private CoT、模型 body、tool wire 或不明 raw 字段展示，建议及费用说明不作为派发、金额或完成证明。空摘要列表不呈现整体完成。

## 实际红绿和夹具边界

后端真实编译 48.95 秒后首 1/9（运行 8.21 秒）。8 项分别在首断言暴露缺真实聊天映射、触发 alias 发布错误成功、channel/mapping IGNORE 未拒绝、外 scope alias 缺 mapping 前提、原 event 损坏仍可读、单 WAL 读取缺行和 immutable mapping 缺失。未知账单/新 attempt 拒绝项原已通过；首失败之后的循环不得声称实际红状态已覆盖。实现后 9/9（6.44 秒）。

前端真实 SFC/composables/子组件测试首 1/8，仅 project/attempt late fencing 已通过；实现后 8/8（1058 毫秒）。IPC/host 是合成输入，卡片没有 stub；这些不是 Tauri 真实付费 UI 或安装验收。新八项注册默认 test:ui 与 test:agent-dialog。

Clippy 首次仅新增 public_projection cmp_owned；以具名 canonical 字符串保留逐字节校验，无 lint 豁免。共享补丁逐个窄 rebase：测试 include 与共享 snapshot helper 限定，publication 三 hook 保留现有完整 eventProof，其余原补丁不整文件覆盖。初始代码路径已脏，全量 prior diff 和本批增量分别保存。

## 最后门禁与保护

最后严格 all-features/all-targets Clippy exit0，wall14.84秒；相关 106/106、runtime55.61秒/wall65.49秒；导入 39/39、runtime2.86秒/wall40.37秒；exact 退役登记 1/1、wall1.30秒。串行 offline/locked/-j1/testthreads1/nice15，不是全量 Rust。

默认前端 800/800、runtime32186.609毫秒/wall32.84秒，vue-tsc 与 Vite exit0、wall6.71秒/Vite2.19秒。前端通过后仅 Rust canonical 比较机械修正，随后 Rust 最后门禁包含该修正。五新 Rust 叶 fmt/check0，无全仓格式化。

23代码路径15已有/8新，新max240行；1053精确收集路径SHA f84d5fff9a59cb13608f0a278eafd8ea7b6e8c4922cc76c4ba7d16ec44c8a08f，1030范围外原路径保持。HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b、git diff --check通过。收集边界为src、src-tauri/src/resources、tools、package/lock/Cargo/tauri/精确退役登记，不含docs/build.rs/icons/capabilities/dist/安装包，不能称全仓未变。原内容、已脏差异、逐文件增量及快照在 /tmp/oviraptor-root-paid-chat-before.json、-prior-diffs、-reviewed-merge-diffs、-scope-final.json、-code-snapshot.json；实际执行日志和门禁 json 同前缀。

## 未完成及风险

聊天公开付费摘要已经接通，不代表有序人工动作真实执行、SDK/Web全路日志、完整 ReAct/所有变化触发、全部专职角色/Broker/真实Concurrency或预算异常/暂停闭环完成。新 mapping 是不可删除财务证明，真实 paid Root 的 terminal cleanup 必须另证明作用域；不能据旧无真实付费 Root 的删除测试称全部删除完成。Source 已知义务的纯 reducer 终态、本地工具智能决策和其余闭环继续开发。评分权重未经效果校准；最后完整门禁、当前安装启动和授权 URL 尚未开始。先前 InputParser 自动审批拒绝仍未交付，不绕过。Goal active，不标 Master 完成。
