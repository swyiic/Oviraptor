# 原 Source 暂停结果显式恢复审计（2026-10-05）

Master/Goal 状态：未完成 / active。本文仅证明确切已关闭、已知耗尽失败的旧 paused+pending 原结果消费，不证明整个恢复、角色/模型、UI、安装包或URL验收。

## 问题与真实数据作用域

从签名Native Source发布与原SDK生产者产生原Mapper/Analyst5/7请求及失败worker，实际恢复终态writer先关闭Root、实际暂停请求后保留父调用inode阻止现代finalizer；释放后以实际quiescence锁和既有旧顺序业务publisher得到paused+pending。没有插入假的Root终态、失败worker、费用或Exit。该夹具复现旧业务顺序，不运行旧安装二进制；SDK为本地脚本服务，不证明真实模型推理或供应商美元。

原删除prepare精确拒绝 source_branch_report_results_changed，读取全库typed rows/rowid零写。普通结果消费者因attempt已paused不能消费，原负测0/1。新增入口后原费用、Root/时钟/退出/材料/日志与临时CAS均保留。

## 最小实现与授权边界

`recover_native_source_pause_result` 注册真实Tauri命令，按scan/attempt返回类型化原Root结果回执。持有scan控制、全部原父调用、Source rounds与specialist SDK退出锁至提交；在私有READ_WRITE已有DB事务重新核验当前paused attempt、唯一原Root、原财务owner/epoch/fence、确切已关闭失败、原冻结runtime/CI材料、前序与失败worker、原费用/两账本、完整Exit及当前原路径/目标/claim。

专有writer仅允许main顶层更新native_scan_branches的status/checkpoint/report_json/updated_at和sentinel_targets的status/last_attempt_number。拒绝trigger写入、Root/费用/业务/资产、插入/删除/DDL。重用原结果消费者与完整删除proof；任一失败回滚六列。没有迁移、建立旧金融身份、重关Root、续租、退款、追加SDK/费用/日志或制造审核。已partial仅核验后返回changed=false。

租约自然到期可读原财务身份，但整个原C物理行仍绑定Exit。终态后手动改到期时间属于损坏，必须拒绝；本批没有放松原收据哈希。实际短TTL在原Root收口前设定，等待自然到期、验证执行准入拒绝，再恢复结果；C原字节/费用/Exit保持，paid删除与重开库冷核验成立。

状态页仅在当前paused+pending Source展示显式核对按钮；服务端继续独立核验原事实。无自动恢复/执行/继续扫描。操作单飞，绑定scan/attempt/status代次，卸载或切换后的迟到回执不更新新视图；错回执不显示成功。错误展示不回显原SQL、私有路径或密钥。沿用当前Native主题变量并提供窄屏布局；SFC/IPC脚本合同不替代安装App/WebView真实验收。

## 开发负向证据

六新增具名Rust测试，多个内部场景不计成多个测试：

- 两实际原失败生产者SDK5/7；恢复只消费六列，重复零写，普通执行拒绝，删除/冷审计/重开库与CAS保留。
- 实际短TTL自然到期，执行拒绝但原结果可消费；不续租、不借新attempt、不重发。
- 19类实际原行损坏，包括C整行/epoch/fence替换、撤权、Root理由/截止、缺Exit、worker/粗账本、round未确认/响应哈希、attempt/路径/目标、伪审核报告/foreign checkpoint/缺claim、runtime/CI错配。不可变行的损坏场景使用临时库拆触发器，不能视为真实供应商未知费用已确认。
- 9类投影忽略/ABORT及恶意Root/费用/asset/业务/scan/sibling写入，拒绝并全库回滚；临时asset哨兵/CAS保留。
- 原Source父调用、Source round、specialist SDK三种实际inode忙锁；缺原父inode不重建。
- 未关闭Root即使旧业务paused仍拒绝，原已付费用和pending义务不改变。

Rust负向扩大首轮5/6：过期夹具漏快照事务；补事务后0/1因终态后改原C整行被正确拒绝。最终改为实际自然到期1/1（编译35.56秒/测试9.81秒），另保留整行损坏负向。没有改生产退出、identity或费用proof。

前端新增7具名SFC/API测试，加既有状态页共57；首轮55/57漏向真实编译模板传props，修正夹具后57/57。生产SFC与template/style真实编译，IPC脚本替代仅限此开发合同；不证明整个UI或真实IPC。

## 最终关联检查

最终同源码关联41/41（测试285.95秒/阶段287.01秒，选择=报告=通过），严格all-features/all-targets Clippy0（41.05秒），同二进制退役50/50（5.62秒，含literal/当前Native JSON），状态页及新SFC专项57/57、TypeScript/Vite构建0、四叶局部fmt和范围diff0。没有ignore；当前2356项Rust全量、安装IPC/整体UI/URL/真实模型质量仍未验收。

Rust选择41具名case来自暂停/quiescence/原失败删除/私有writer及本批6项；cargo JSON定位实际当前二进制，--list核对，再exact执行，源码逐文件摘要在执行前后保持。关联与50退役集合可能重叠，不相加；此前所有局部结果均为历史。UI构建在最后仅Rust/测试夹具调整前执行，全部生产UI输入随后保持；不是安装App可打开证明。

## 工作树保护与可追溯文件

本批代码路径13（7已有/6新），逐文件前像、原git diff及最终增量保存；所有新叶<400行，已有lib仅增加命令注册。源集合1370 SHA `c475308f86e290c5acf950d543158e0435c5cf677be86e28c2d192b588a7c7d5`；1357原范围外SHA保持，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b`保持。原writer、SDK/费用/Exit生产者、冷审计、shared fixture及全部既有测试原字节不改。

开发证据位于 `/tmp/oviraptor-source-pause-recovery-*`：baseline/before/scope-final/code-snapshot、prior-diffs/reviewed-merge-diffs、red/first-green/negative-run/expired-green/expired-natural、ui-run/ui-final/ui-build、final-build.jsonl/final-test-list/final-binary/final-result/final.log、clippy、retirement及docs前像/最终增量。临时文件不是长期交付依赖；本审计保留结果与边界。

只操作临时Git/SQLite/CAS/localhost脚本SDK，没有真实Nest/业务/CAS/asset、安装App、授权URL写入或自动提交。本轮未创建Codex子智能体。

## 剩余范围

全部十四项以Master顶部为准；Source其他保护/取消/未知费用/缺事实/未关闭Root/身份替换及历史attempt/其他角色仍保留义务，不能由原标签回填费用或退出。十维动态预算/供应商对账、六类Root监督、全15角色真实推理/工具与并行、同聊天闭环和逐路日志、整体UI、回归债、非Web/真实DockerWindows、数据知识及框架后的全量/安装App打开/两URL匿名只读/真实模型质量仍未完成。
