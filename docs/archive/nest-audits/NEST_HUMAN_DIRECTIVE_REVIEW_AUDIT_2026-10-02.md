# 人工草案判断、事务回执与聊天视图审计

日期：2026-10-02。Master 开发中；本批完成人工审批、修改和拒绝的持久化与前端接线，尚未完成逐角色真实执行闭环或桌面验收。

## 当前代码证明的问题与修改

原 confirm/cancel 路径缺少独立的修改、带理由拒绝及不可变人工判断回执。加入签名/schema 的00仅为编译前提，不计红测。九项真实生产 helper/临时 SQLite 合同首次3/9；实现后7/9。实际审批仍调用原 confirm_bound_draft，原队列、冻结草案及新回执在同一 IMMEDIATE 中提交；写后核对完整队列与原 Root、C、Source Native JSON。旧已确认记录只读重放，不回填新判断。

revise/reject 只接受原未确认、未入队、当前原 C/lease/fence 对应的草案。修改经过原 parser/Source focus/fact guards，产生独立 source message、UUID、revision+1/hash；仅终止旧草案，新草案仍须独立确认，不自动执行。拒绝保存脱敏理由并终止草案，不产生队列、worker、assignment、工具或新能力。已入队、claimed/assigned/applied/completed 工作不可通过该接口撤回；原 cancel 仍是独立关闭流程。

新 journal 的 draft/revision、receipt_id、sequence 全 unique 碰撞保护，在 recursive_triggers=OFF 也不能 REPLACE 隐删旧记录。私有 RAII writer 只允许判断 lane 的实际 direct 写入和四个已存在 canonical event emitter 的事件 INSERT；完整旧草案/队列/回执/host/event 行及新增事件 scope/entity/payload 写后核验。触发器写项目、其他 Root、其他草案、费用或 Source JSON、IGNORE/ABORT/错误事件全部回滚，同一连接移除故障后可正常使用。

首次扩大141项为138/141。新回归是损坏人工回执使整个时间线读失败；现在只将该条标记 receipt_unverified/human_review_receipt_unverified，保留其余时间线。SQL/表缺失错误仍向调用者传播，读取不修复历史。补充故障合同还实际证明，仅校验回执身份会接受 executionCompleted=true 等伪执行字段（0/1）。现在核对完整20字段结构、原冻结动作顺序/角色/intent/capability/queue、terminal/not_started/null execution receipt，及修改后独立草案身份、版本和原 scope；审批不能冒称角色执行。

语义修复第一次14/15，实际拒绝理由含已脱敏标记时重复脱敏改变了 fingerprint。最小修正将输入脱敏和已保存文本核验分开，不重复脱敏旧标记；最终15/15。此失败和修正保留，不把第一次14/15计为通过。

前端真实 AgentDialog SFC、production composable 和 api.ts wrapper 接线：修改/拒绝编辑器、不可撤回队列提示、持久时间线回执；全部冻结草案字段比对，原 Root/target/recipient/source message 与 successor 绑定，拒绝的 successor 三字段必须同为 null。项目/任务/attempt/线程切换与 unmount 隔离迟到成功、错误、刷新与 finally，不允许旧请求释放新编辑器锁。原 send 线程缓存语义保留。

人工回执中的 ordered actions 是判断计划：executionState=not_started、executionReceipt=null、executionCompleted=false。真实 scheduler 合同确认不支持的多角色组合仍 deferred，Reviewer 仍需要冻结候选；本批没有把角色字符串或审批按钮当作执行器。

## 实际证据与边界

以下均为 /tmp 文件。Cargo offline/locked、nice -n15、-j1、测试单线程；各 Cargo 串行，运行期间没有修改其源码。

| 阶段 | 实际结果 | 日志 |
| --- | --- | --- |
| 原审批/修改/拒绝合同 | 3/9，14.82秒 | oviraptor-human-review-red.log |
| 初次实装 | 7/9，14.42秒 | oviraptor-human-review-backend-green.log |
| 写保护扩展首次 | 8/12，16.99秒 | oviraptor-human-review-write-guard-red.log |
| 修正夹具后的实际业务副作用 | 0/1，0.44秒 | oviraptor-human-review-business-write-red-fixed-fixture.log |
| 写保护完整实装 | 12/12，23.49秒 | oviraptor-human-review-write-guard-green.log |
| 首次扩大 | 138/141，242.12秒 | oviraptor-human-review-final-affected.log |
| 原代码隔离核验 | 1/3；两个相同预算失败，1.69秒 | oviraptor-human-review-prior-probe-with-build.log |
| 时间线回归与原人工合同 | 13/13，26.26秒 | oviraptor-human-review-regression-final.log |
| JSON/scope 损坏隔离与 DB 错误 | 1/1，1.28秒 | oviraptor-human-review-corrupt-projection-final.log |
| 伪执行判断回执 | 0/1，0.48秒 | oviraptor-human-review-decision-contract-red.log |
| 最后人工/损坏回执合同 | 15/15，30.28秒 | oviraptor-human-review-decision-contract-green-final.log |
| 最后扩大（见两项排除说明） | 144/144，255.14秒 | oviraptor-human-review-all-affected-semantic-final.log |
| 最后严格全目标全特性 Clippy | exit0，11.65秒 | oviraptor-human-review-clippy-semantic-final.log |
| 独立 importer | 39/39，2.53秒 | oviraptor-human-review-importer-final.log |
| 完整限定名 exact 退役登记 | 1/1，0.58秒 | oviraptor-human-review-literal-final.log |
| 正确生命周期夹具的前端红测 | 108/128，1.63秒 | oviraptor-human-review-frontend-red-corrected-fixture.log |
| 同合同修复 | 128/128，1.69秒 | oviraptor-human-review-frontend-green-corrected-fixture.log |
| 最后受影响前端 | 489/489，10.44秒 | oviraptor-human-review-frontend-final-affected.log |
| 最后 vue-tsc | exit0 | oviraptor-human-review-vue-tsc-final.log |

集合重叠，不相加。新增14个后端合同，主库1816→1830；没有跑全1830。Importer 在最终回执语义修复前通过；此后其 db/schema/source 字节没有改变，最终全目标 Clippy 也覆盖该编译目标。

144项命令使用 directive/source_guidance 过滤，并**明确排除**两个已在原代码失败的正向合同：directive_proposal_native_loop_receives_actual_specialist_result、directive_reentry_real_target_does_not_retry_unknown_proposal_or_block_mapper。原934代码路径逐 SHA 重建，所有字节匹配422e0d…；另补原 build.rs（SHA487059eaf8a947b80f20a9aacac038a5047b2ad69d2401b827376c67d6fe847f）作编译辅助，实际1/3证明两个 budget_projection_contract_conflict 已存在，旧时间线合同原来通过。初次遗漏 build.rs 的 OUT_DIR 编译失败不算行为证据。元数据在 oviraptor-human-review-prior-probe.json。

这些正向夹具将原 Single Root 用于 Multi/C 路径，违反已存在原预算限制。接下来需真实新任务 Multi creator、原启动 work_dir/HMAC 和 supervised executor；禁止给旧 Root 补模式或放宽预算。两个合同仍未通过，不能把144项称作全部回归或完整 Multi 验收。扩大过滤另含三项已存在安全草案/Coordinator claim/DB upgrade 合同，精确增减名单见 oviraptor-human-review-affected-filter-review.json。

最初多角色夹具没有真实 claim_pending_directives，补生产 claim 后才测试实际 scheduler。业务副作用夹具给同 scope 加第二个当前 Root 导致 recipient ambiguous，先把该合成对照 Root 设为 completed（完整 JSON 保留），再获得实际0/1副作用红测。前端首104/128有四个夹具错误：项目A→B→A在一个未渲染批次，child 实际没有收到B；补每次真实 Vue render 后，隔离原 controller 再跑有效108/128，剩余20处确为生产校验缺口。未把夹具错误算生产缺陷。

## 逐文件改动与数据作用域

25代码路径：12已有+13新增，无删除、重置、自动提交或批量覆盖。基线934路径422e0dd666776d6e59b5107b1c8e5f663b8bae06a61d5dde53edf37e988f4754；最后947路径f5e406f84a4c80348045bcb4bf721cff9c772bcadc0023f4cd2c7979c8951302。922个范围外原路径逐字节不变，HEAD保持59be3d86d25adda5b1f975759bf256ef3b94f32b。

此摘要沿用已登记源码/tools/resources/Cargo/配置路径，不包含 build.rs、icons/capabilities/dist 或整个仓库所有文件。原文本和既有差异分别保存在 oviraptor-human-review-before.json、*-prior-diffs；阶段合并证明、前端隔离 controller、最后差异、scope、snapshot、16 Rust 叶格式记录及原三文档字节备份均在同名前缀 /tmp 文件中。16叶 fmt、git diff --check 通过，新手写文件最大346行。原 draft_store386→417、AgentDialog400→416、types1688→1705 是既存大文件薄接线/格式债，未准许新大模块。

只使用临时 SQLite、已有生产 Source fixture、实际 localhost 模型与前端传输夹具。未接触真实 DB/CAS、资产、安装、git提交或两个授权URL。新判断 journal 通过 draft FK 支持原父级 cascade；没有清理旧业务记录或把历史确认转换为新授权。前端 harness/localhost 合同不是实际 Tauri 桌面或目标功能验收。

## 未完成与风险

人工三决策及原 cancel 已接线，但实际 ordered 每角色执行 journal、后续动态决策、暂停/新 Reviewer 收口还未完成。新 writer/readback 持有完整旧事件行的内存和线性读取成本；大历史数据需后续有界核验，不能删历史或省略完整性。Commit ambiguous 仍须读取原回执，不自动重发。

继续原 Root 最终 elapsed、超限/过期原金融事实、精确对账/dynamic、可信新 Single/Multi/HMAC/联合 writer 与持续 Root SDK tick、持久 OS/worker 恢复及安全重派、全部真实角色/Browser Broker、Web helper/SDK/安装逐路日志。Source Root 已在初始 SDK 前注册；Source 新预算 producer 仍另需实现。InputParser 部分此前被自动安全审查拦截，未完整交付，不重试绕过或记完成。

用户新增 UI 美观与工作过程展示要求：正在并行准备任务中心、聊天和执行/日志的信息层次、状态卡片和可展开详情；尚未应用/验收，不能冒充新智能体能力。优先保持真实反馈、上下文、模型/工具执行和独立 Reviewer 核验。

最后仍须完整门禁、当前安装包/真实 App 启动与强杀恢复、授权 URL 匿名只读验收；登录身份待用户提供。Master/Goal不标完成；Goal工具仍 paused，按用户继续授权持续开发。
