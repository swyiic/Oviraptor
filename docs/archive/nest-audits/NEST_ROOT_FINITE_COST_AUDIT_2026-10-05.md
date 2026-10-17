## 2026-10-05 用户明确要求暂停：当前批次检查点及完整剩余范围

用户已明确要求“停止掉，把剩余的任务汇总一下，额度不够了，要等下周了”。开发在本检查点停止，随后将Goal设为paused；Master未完成，不再等待此前的暂停确认，不自动续跑开发。下方旧active、生产修复待实施及历史在途描述是历史记录，以本段为准。

当前finite-root-cost批次已实现：一致真实费用超过派发预估时，仅在原冻结共享预算及原各维硬限额内补足该原请求费用；新内部版本3冻结该规则，现存Native版本1/2保留原财务语义和身份，不升级旧回执。原SDK received回执先独立提交，后续费用writer的IGNORE/ABORT/ROLLBACK或越界写失败不抹掉原账单；原回执未完成结算/终态证明时阻止新工作。真正超限、共享余额已被child占用、缺失/不一致usage继续保留未决，不自动重发。付费重放核验原费用物理行与原证明，不按当前余额重新解释。

数据范围仅临时SQLite/CAS与localhost实际SDK夹具。实际费用input50,000/cached10,000/output10/total50,010；原任务硬限额200,000足额与40,000超限场景分别验证。六项包括足额结算、真正超限、四种费用writer故障、坏/缺失usage、child占用共享余额及原实际费用行篡改；最终6/6通过（7.90秒）。同源码严格all-features/all-targets Clippy -Dwarnings通过，退役50/50通过（含literal/当前Native JSON），两个新Rust叶fmt及scope diff检查通过。显式对账/授权恢复仍未完成；仅有原回执及阻止新工作并不构成聊天恢复闭环。

381项关联回归已启动，暂停检查点尚未确认终态，最近确认158项通过、未观察到失败，不能写成381通过。运行session47880，runner /tmp/oviraptor-finite-root-cost-final-run.py；日志/tmp/oviraptor-finite-root-cost-final-final.log，最终结果/tmp/oviraptor-finite-root-cost-final-final-result.json（检查点尚不存在）。已启动本地回归可自行结束，不触发后续开发；续接先读取原结果/日志并核对进程状态，不盲目重启已有runner。列出2446项Rust但未全量；上一源码369/369与之前350项前端模拟IPC不能作为本批整体功能/安装态验收。

当前源码SHA737f73e8fcdd4f031789987d49e8b4b852a203ed78705ab83ebacf311f94fe90，1418源码文件，17修改路径（15已有/2新增），1401原范围外源码与HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b保持。阶段finite-root-cost已begin，不得重复begin。前像/逐文件原Git差异/合并差异/范围封存保留于/tmp/oviraptor-finite-root-cost-*；本检查点文档原像备份/tmp/oviraptor-finite-root-cost-paused-docs。未重置、未批量覆盖源码、未自动提交，未操作真实DB/CAS/asset、安装App或授权URL。

续接顺序：先核验该381回归终态并处理真实失败；再完成原未知/已收到未结算费用的明确对账与授权恢复、全部十维预算，随后15角色真实执行、六种Root监督、真并行、证据独立审核、用户聊天/逐路日志及整体UI；Code/Greybox/数据归属同步收口。最后才做完整门禁、同源安装App打开/历史崩溃及授权URL、真实模型质量/USD验收。授权URL为http://121.224.72.77:8082/tcPspWebFrontend/views/system/login.html和https://sndhmt.com/operation/templates/index/index.html?lang=cn，匿名无破坏只读范围，登录身份待用户提供。真实Nest/业务/CAS写入虽获授权，后续清理仍须精确盘点和备份，asset不得删除。

完整REM-A01—A14剩余清单见[NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md](NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md)顶部。

---

# Root 有限预算真实费用问题证明（2026-10-05）

Master未完成。本阶段只新增实际生产链路测试，尚未修改生产财务行为、提供对账写入或恢复权限。

问题：RootModelCall::receipt::settle、Tick::cost_unknown和Tick::cost_rows均把有限预算中供应商报告total超过本次预估视作未知。预估是派发前占用，不能直接代替冻结总额对实际已报告账单的判断。Root shared::available已经按原Root已消费/最大类别占用及原child总额计算共享余额。

数据范围：当前真实creator/HMAC/Native原Root/C与活父夹具，临时SQLite/CAS和localhost实际SDK；原硬总额分别从出生冻结200,000与40,000、requests20。响应实际报告input50,000/cached10,000/output10/total50,010/request1，账单一致，原预估小于实际总额。原received usageReported=true和独立终态证明均已持久，各一次SDK；未更改Native plan或硬限额。

两项选择=报告2，1通过/1真实失败。200,000足额场景在“真实已报告费用应已知”断言失败，错误budget_indeterminate_requires_reconciliation。40,000超原限额场景正确不发布、不重发，保留原费用和未决；重放typed/物理全表零写。测试1.14秒、阶段35.9964秒。runner session96435已终态，结果/tmp/oviraptor-finite-root-cost-red-final-result.json，原日志/tmp/oviraptor-finite-root-cost-red-final.log。

修复约束：只补原请求实际费用在原冻结共享总额内的不足预留；验证所有原类别和其它worker已占用，不扩大硬限额或从替换C/attempt借额。付款事实必须先保持，后续准入/发布失败不能抹掉实际received。未知/不一致/真正超总额继续阻止新工作和自动重发。需要冻结新财务规则，使旧Native版本1/2的原账单、unknown标记和物理证明按原规则核验；不能用当前余额重算旧判断或后补原证明。现有Kind::Reconcile只能将原未决同额转消费，无法直接表达不同于原预估的各维实际账单，不能拿它假称已精确对账。以上是待实现约束，不是验证通过的实现。

最小范围2路径：agent_tests.rs增加新叶，agent_tests_root_finite_cost.rs两项实际用例。1415原范围外源码和HEAD59be3d86保持，当前1417源码SHA b077c29e5db8c647dcf66085b80e86a73c00dcace2de1402055f0a60067e2ff2；新叶fmt/scope diff0。此前369/369属于f507d5be，不是本批验收；当前2442项Rust未全量。

后续先实现冻结的财务规则、共享余额和原费用验证，增加余额竞争/其它未决费用/坏账单/writer故障/旧回执与重放负向，再跑关联门禁。完整Master仍有显式对账恢复、全角色十维预算、六监督、真并行、用户聊天/日志、整体UI及最终安装/URL/真实模型质量未完。真实数据授权不等于本批实际操作；未碰真实DB/CAS/asset/App/URL，未自动提交。
