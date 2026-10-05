# Native Source 逐路持久日志、回放与写入边界审计

日期：2026-10-02。Master 开发中；本批不是全部运行路径或真实桌面验收。

## 已证明并修复

Source analyzer 的真实 host/pinned-container 入口原来等子进程 EOF 才得到日志。八项真实子进程合同首次0/8；仅实现 bounded observer 后7/8，实际 analyzer 入口仍无日志；接线后8/8。真实工具在等待 gate 时，生产适配器已提交 stdout/stderr，另一只读连接先看到行再收到身份通知。容器合同使用真实可执行 CLI 夹具进入 create/inspect/start/cleanup，不能当作已安装 Docker 验收。

观测器与原机器 JSON 原始 stdout/stderr 分开；原字节、cap、取消和后代强杀保留。共享纯 CR/LF framer，完整记录先脱敏，跨块密码/UTF-8、EOF 尾、超大记录整条省略都有合同。队列64项非阻塞、partial16KiB；丢失计 gap，单执行8192行/1MiB另有终态行。独立 journal 只开现有 DB，25ms等锁、FULL提交，通知只含身份/游标；失败不伪报完整捕获。

日志绑定真实 Native 分支 dispatch claim、scan/attempt/branch、computed invocation key、进程 UUID 与 stage。Source analyzer 在 Root 注册之前，使用已有 Source 分支准入，不伪造 Root/C/worker 或新授权。版本和每个步骤独立 execution；重复文本保留，断联后按序回放。

实际回放 helper 六项首次2/6：原 reader 在同一 WAL 快照外读取高水位，未正确重置 latest attempt 游标，db::open 还创建不存在的 DB，并允许未知/删除/跨 scope。现 READ_ONLY/no CREATE、query_only、DEFERRED 单快照；验证原 task/attempt/execution，历史 attempt 可读，非零游标须带其 attempt。最新 attempt 仅在提交切换后重置；scope/state/highwater/rows 同一快照，错误和未来游标拒绝。

实际前端 composable 接 App runner 和任务面板，隐藏停止、回来补读、重连与选择切换 fencing；每次最多4页×300并继续同一刷新调度，保留300条和 evicted 计数。严格验证 backend scope/cursor/per-stream ordinal，通知文本不进入显示。前端 harness 证明生产 composable 合同，不替代真实 Tauri/UI 验收。

Journal begin/append/finish 实际触发器与 REPLACE 红测1/4，三处行为失败。现私有 authorizer 只允许 direct main 精确诊断 lane 写，不允许触发器写业务/其他 Root/DELETE/DDL/ATTACH；INSERT/counter/row/state/immutable scope 写后逐列复核。所有 unique 的 REPLACE 在 recursive_triggers=OFF 也拒绝。失败完整回滚，不通知，同一连接移除故障后仍可用。已经 begun 的取消/EOF 诊断只追加原 scope，不创建新执行权；该合同原来已通过。

## 实际结果和阶段边界

草稿分别位于 /tmp/oviraptor-live-native-log-draft、/tmp/oviraptor-native-log-replay-draft、/tmp/oviraptor-native-log-write-guard-draft。缺 API/schema 的00只是签名前提，不算红测。

| 实际证据 | 结果 | /tmp 日志 |
| --- | --- | --- |
| observer 首次 | 0/8，2.35秒 | oviraptor-native-live-log-red.log |
| observer 未接 analyzer | 7/8，2.22秒 | oviraptor-native-live-log-observer-before-wiring.log |
| analyzer 生产接线 | 8/8，1.77秒 | oviraptor-native-live-log-green-production.log |
| core 回放首次 | 2/6，0.55秒 | oviraptor-native-log-core-red.log |
| core + observer 修复 | 14/14，2.25秒 | oviraptor-native-log-core-green-with-observer.log |
| guard 首次 | 1/4，0.04秒 | oviraptor-native-log-write-guard-red.log |
| guard + core + observer | 18/18，2.70秒 | oviraptor-native-log-write-guard-green-combined.log |
| 格式和静态 API 修复后受影响 | 36/36，2.95秒 | oviraptor-native-live-log-affected-static-fix.log |
| 原 analyzer/container/lifecycle | 16/16，7.63秒 | oviraptor-native-live-log-analyzer-lifecycle.log |
| 最后前端受影响 | 361/361，8.67秒 | oviraptor-native-live-log-frontend-affected-final.log |
| TypeScript | exit0 | oviraptor-native-live-log-vue-tsc.log |
| 最后全目标全特性严格 Clippy | exit0，wall19.63秒 | oviraptor-native-live-log-clippy-final-corrected.log |
| 最后独立导入 binary | 39/39，2.68秒，wall47.83秒 | oviraptor-native-live-log-importer-final.log |
| 完整限定名 exact 退役登记 | 1/1，0.46秒，wall13.63秒 | oviraptor-native-live-log-literal-final.log |

前端首红0/8；第一次修复7/8时发现新测试声明 failure 选项却未传 mount，最小补传 options 后8/8，不是额外生产缺陷。前序23/23命令包含不存在的 test_live_runner_log.cjs，Node 静默忽略；已用三个实际文件明确重跑23/23，再扩大到真实361/361。不声称不存在套件运行。

严格检查先失败 three static diagnostics：container 8参数、observer 类型复杂和末尾 needless deref；改具名 RunControl、Observer alias、末尾消耗 Option，未放宽 lint。其次独立 importer 无 native_pipeline 模块，db 薄 schema hook 改 direct include_str；最后测试闭包 drop_non_drop 删除无效 drop。前两次失败日志 clippy.log/clippy-final.log、后一次 clippy-shared-schema.log 保留。36/16在共享 schema/test-only最后静态调整之前，严格全目标/导入/字面量在之后。集合重叠，不相加；新增18后端合同，主库1798→1816，未跑全1816。

Cargo offline/locked、nice -n15、-j1；测试单线程。Cargo 串行，运行期间未编辑源码。

## 原改动保护及数据作用域

31代码路径：13已有+18新增，均逐文件检查，未删除或重置。基线916路径00e91d0bf3138a88b31c6342b18b8121f49ca24cbe70cb4171a7ffeaf4ac352e；最后934路径422e0dd666776d6e59b5107b1c8e5f663b8bae06a61d5dde53edf37e988f4754。HEAD保持59be3d86d25adda5b1f975759bf256ef3b94f32b；903个范围外原代码逐字节不变。

/tmp/oviraptor-native-live-log-before.json 保存31原文/新增 null，*-prior-diffs 保存既存差异，*-before-fmt.json、*-format-diffs 保存格式来源；*-final-diffs、*-scope.json 与 *-code-snapshot.json 是最后核验。21 Rust 叶 fmt、git diff --check通过，新手写文件最大281行。原 analyzer 778→828、lib558→560、App703→705 是既存大文件薄接线/局部格式债，不是新大模块许可；后续拆分需独立范围。

一次前提脚本错误：原文档已有 Markdown 双空格硬换行导致 assert 失败，调用方未检查 exit code 仍应用了00/01。随后从 draft 原文本和916摘要重建全部10原有文件，对全部16 staged路径逐字节证明00/01结果一致，恢复准确 before/baseline/prior记录；原文档正文逐字节保留。没有数据或既存代码丢失。后续每个依赖阶段检查前提 exit code 成功再变更。该错误不算门禁通过。

仅临时 SQLite、实际本地子进程和 CLI 夹具。没有真实业务 DB、CAS、资产、安装、git提交或授权URL操作；以前精确Nest清理以独立审计为准，不借旧数量作当前盘点。journal 添加 schema 不回填旧日志、不改 Native JSON。

## 未完成和风险

AST/浏览器 Web helper 仍需真实 claimed Web 准入接 observer 和 worker stage emissions；Source SDK/其他 runner 路、安装日志批次身份、骤停/open执行和当前安装App重启回放还未完成。每条 FULL提交可能有成本，压力/锁竞争明确 gap，不能当完整日志。私有 guard 正常 Result 错误会清理 hook，本批不声称 panic unwind 合同已验收。

继续 Single/Multi 可信创建与 Root tick、最终 elapsed/精确对账/dynamic、所有真实角色/Browser Broker、持久恢复/重派/canonical、聊天四态与实际逐角色执行闭环；最后全量门禁/当前安装态/真实桌面与授权URL。InputParser 子代理此前被自动安全审查标记可能网络安全风险而未完整交付，该部分不应用/不记完成。Master/Goal不标完成，Goal工具仍 paused，按用户继续授权持续开发。
