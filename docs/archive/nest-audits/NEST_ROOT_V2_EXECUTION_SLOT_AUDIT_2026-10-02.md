# 全新 Native Web Root v2 执行槽与实际批次预算审计

日期：2026-10-02。Master 继续开发；本批没有完成实际 Concurrency 角色、用户模式或最后整体验收。

## 已证明的问题与最小修复

- 真实 production 计划冻结、runtime_open_run、multi_agent_prepare 和 localhost Mapper SDK 合同证明，普通 child 的 lane 占用原来记成 concurrency_batches reserve=1；显式 batch=0 也被原算法改成3。重派、保存只读回执及终端人工回执同样持有这笔假批次占用。
- 新增 typed 新 Root 预算声明，仅当前同一 IMMEDIATE 事务真正 INSERT 的私有 NewlyInsertedNativeRoot 证明能冻结。新声明保存精确 scan/attempt/target/Native hash、原 JSON 文本 hash、十维 limits 和独立 capacity=3。旧 Root（包括零用量）不能补声明；缺行维持原合同，坏行直接拒绝。当前 Native JSON、hash 算法、旧费用和旧槽记录不重写。
- 声明本身是预算合同，不是目标、写入、浏览器或任意角色的授权。模型/请求/时限继承 Native 上限，browser/write/upload 仍0；真实批次 ceiling 可明示0..128。现有生产启动调用仍 None；可信 UI producer 尚未接入。
- v2 child 执行槽由已有原 assignment/worker 和实际 occupied lane 核验，不再添加假的批次 reserve/consume/release。三条 lane 的排他性和总 capacity 保留。旧 Root 沿用原十维/slot entry；未派发重派及 expiry proof 已分支核验原声明，原 worker、clock、费用不改。
- 原声明仅 UPDATE/DELETE 不可变不足以挡住 recursive_triggers=OFF 时 REPLACE 的隐式删除。新增 BEFORE INSERT 同时检查 root PK 和声明 UUID UNIQUE，两个冲突都拒绝。
- 全新 Root 创建先安装单一 NewRootWriter authorizer，贯穿 Root INSERT、声明和 Native checkpoint 投影；拒绝业务/旧 Root/其他 trigger 写。最后复核全部旧 Root typed rowid+值、旧声明、collaboration 前缀及其他投影，恰好一条当前 prepared event，投影等于原 JSON 文本，总变更恰好4；失败整事务回滚。
- 实际 Reviewer/Investigator 保存结果恢复另有0/2红测：入口先 DELETE lane，v2 释放时失去原占用证明。只将释放核验移到 DELETE 前，同一原事务和原费用键保留；IGNORE DELETE 必须完整回滚。恢复不重发 SDK、不新建/恢复能力、不 renew C、不重开执行态。

## 实际证据及快照顺序

草稿位于 /tmp/oviraptor-root-v2-slot-draft。00 是缺失 typed API/schema 的编译前提，不算行为红测；先应用01收集实际失败，再应用02。03单独证明遗漏的两个恢复消费者，04仅修顺序。

| 证据 | 结果 | 日志（/tmp/oviraptor-root-v2-slot- 前缀） |
| --- | --- | --- |
| 新11合同首次实际红测 | 4/11；7项行为失败，2.69秒 | red.log |
| 首次02编译 | E0505；Option guard仍借用事务 | green-stage2.log |
| 消耗整个 Option 后11合同 | 11/11，3.38秒 | green-stage2-fixed-borrow.log |
| 保存 Reviewer/Investigator 红测 | 0/2，execution_slot_binding_conflict，0.78秒 | review-gap-red.log |
| 04修复＋原恢复 fault matrix | 23/23，49.20秒 | green-with-legacy-recovery.log |
| 扩大受影响首次 | 209/210，182.36秒 | final-affected.log |
| 原观察者旧断言单独复现 | 0/1，0.23秒 | observer-failure-repro.log |
| 原观察者独占边界修正 | 1/1，0.24秒 | observer-ownership-green.log |
| 修正后扩大受影响 | 210/210，182.44秒，exit0 | final-affected-corrected.log |
| 首次严格 Clippy | exit101；旧 record_attempt_plan 被包装入口绕过成 dead_code | final-clippy-corrected.log |
| None 继续原 API 后最后定向 | 13/13，4.97秒，exit0 | final-wrapper-green.log |
| 最后全目标/全特性严格 Clippy | exit0，29.89秒 | final-clippy-wrapper.log |
| 最后导入 binary | 39/39，2.43秒，exit0 | final-importer-wrapper.log |
| 最后完整限定名 --exact 字面量门禁 | 1/1，0.60秒，exit0 | final-literal-wrapper.log |

原观察者实际重复 multi_agent_prepare 已被先前实现的 parent-supervisor OS 独占拒绝；原测试仍期待较晚的 SDK 未知回执错误。只修一条错误码断言为稳定 native_invocation_not_owned 前缀，原 running/capabilities=2/SDK=1 合同不变。它不证明整个重复 prepare 都只读；当前 prepare 在 service.start 前仍有 C/Root/ledger 写，后续 mode 批必须补完整 application snapshot 合同和前置所有权。

最后210集合在 API 委托静态修复之前，最后13、Clippy、导入及字面量在之后；集合重叠不相加。主库新增13合同，1785→1798。不是全1798或 UI/安装/URL 验收。

Cargo 全部 offline/locked、nice -n15、单 jobs，测试单线程串行；无并发 Cargo 或编译期间源码编辑。

## 文件与数据作用域

- 14已有＋9新增，共23代码路径；没有删除或907基线外的代码变更。基线 /tmp/oviraptor-root-v2-slot-baseline.json 为907路径 fa69137af0c65dad46375d0dbe057d61f10a01156e32fa6f9957138a3fb530f0；最后 /tmp/oviraptor-root-v2-slot-code-snapshot.json 为916路径 00e91d0bf3138a88b31c6342b18b8121f49ca24cbe70cb4171a7ffeaf4ac352e。HEAD仍59be3d86d25adda5b1f975759bf256ef3b94f32b。
- 原字节、已有 Git 差异、本批差异、格式差异和逐文件行数保存于 /tmp/oviraptor-root-v2-slot-before.json、*-prior-diffs、*-final-diffs、*-format-diffs、*-scope.json；三文档原文另存 *-docs-before.json。先核对草稿五个 patch hash 和13原文件基线再应用，没有旧 mirror 全覆盖。
- 21个主体 Rust 叶格式最终通过。另新增核验的旧 specialist_recovery 叶只改一条断言：其原343行压缩格式 baseline --check 已失败，713行格式差异；本批不批量重排，记录格式债，不能称22叶或全仓 cargo fmt通过。23文件 git diff --check exit0。
- 新手写文件最大338行。原 store 918→982、commands/agent_runtime 703→763 是既存大文件的薄 API hook/委托与局部格式规范化结构债；后续应独立拆分，不能借此扩写大文件。最初21叶 fmt失败的315行差异已保存；格式实际改变5叶，其余字节不变。
- 所有 SQL/SDK/触发器故障仅临时 SQLite 与 localhost。无真实业务库、CAS、资产、安装包、git提交或授权URL操作；此前真实Nest精确59行清理属于独立审计。本轮不复用旧资产数量宣称当前盘点。
- 检索时曾猜错 prepare 路径/通配及尚未生成的日志；这些读取诊断不算验证，实际入口已用 rg 重新发现。

## 未完成与风险

v2 只是全新 Root 的可信原子声明和槽位语义。诊断明确 race_batch_dispatch_unimplemented。实际2/3并发的角色、原 canonical action owner、一次事务派发全部请求/cleanup 预算、serial/control/batch/final/cleanup、SDK候选和独立Reviewer尚未交付。

接下来继续用户 Single/Multi 私有启动/HMAC与联合新 Root writer、完整有界 Coordinator tick/严格语义回执；Input/ClientSide/Dependency及其余真实角色、完整 browser/write/upload Broker、Source新预算producer、跨Coordinator/持久worker恢复、精确对账/dynamic/最终elapsed、聊天修改/拒绝/动作与逐路实时日志。最后才做全量门禁、当前安装态、真实桌面强杀和两个授权URL。原 Native/旧Root不补权限，保留未决费用；Master与Goal不标完成。Goal工具仍旧paused，用户继续授权下持续开发。
