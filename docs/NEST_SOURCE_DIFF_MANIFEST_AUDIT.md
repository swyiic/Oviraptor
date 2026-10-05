# 源码增量清单真实性审计

状态：已修复源码快照中 Git 增量清单的工作区遗漏与路径解析问题；新增 7 项回归、快照相关 22 项、Native Pipeline 61 项和残留守卫 4 项通过。最终完整 Rust 1034／30、严格 Clippy／fmt、实际 SFC 146／构建／localhost 回环均通过。**本次不是 full/diff/auto 执行范围合同已完成的声明，主计划继续进行。**

## 1. 复现的缺陷

`RepositorySnapshot::capture` 原来使用 `git diff --name-only -z <base> <HEAD>`，但分析器读取的是包含未提交修改的工作区快照。于是提交后的 dirty、staged、新文件和工作区删除可能不在增量清单中。

原命令输出经面向文本的 `git_text(...).trim()` 解码，不能保持以换行开头的合法文件名；从仓库子目录建立快照时，Git 路径与快照根目录也可能不一致。这些问题会影响源码 Broker 的 `git.changed_files`，因此必须先修复，不能直接拿旧清单裁剪分析器输入。

## 2. 本次实现与不变量

- 在已验证可用的 base 上比较工作区，而不是只比较 HEAD；合并非忽略的 untracked 路径，排序并去重。
- 关闭 rename 合并，保留旧路径删除与新路径两个名称。删除路径可以出现在变化清单，但不会虚构成快照中仍可读取的文件。
- `--relative` 和显式当前目录 pathspec 将路径限制到用户选择的源码根目录。NUL 原始字节解析保留空格、换行，不再 trim 文件名；不可解码或非相对路径报缺口。
- `rev-parse` 使用 `--end-of-options`；所有调用是参数数组，不通过 shell 拼接。
- Git 读取关闭 optional locks、fsmonitor；diff 明确关闭 external diff 和 textconv。不会为了得到清单而执行仓库配置的这些钩子。
- `assume-unchanged`／`skip-worktree` 可能隐藏工作区变化，因此当前将清单置为未知并记录 `diff_index_hidden_paths`，不清除用户的索引标记、不伪造空集。索引读取失败记录 `diff_index_unavailable`。
- 正常无变化为 `Some([])`；无效／非祖先 base 或无法可信读取清单为 `None`。两种状态不能混同。
- 快照材料化后重新检查 HEAD 和变化清单，检测捕获期间的 Git 元数据变化；已有整仓 `verify_unchanged` 和冻结副本完整性校验不变。
- 清单仍随快照持久保存。恢复和 Broker 工具读取存储结果，不重新查询已经变化的工作目录。

实现范围：`src-tauri/src/native_pipeline/snapshot.rs`；新增回归位于 `tests_snapshot_diff.rs`，由 `tests.rs` 注册。没有新增历史运行时入口或扩大残留白名单。

## 3. 回归与失败证据

新增 7 项测试：

1. 已提交、已暂存、未暂存、新文件、删除及重命名双路径；保存恢复后清单不变，未变化文件仍在整仓来源清单中，之后修改任意来源文件仍被拒绝。
2. 子目录仓库范围及包含换行的文件名。
3. 正常空集、无效 base、隐藏索引标记的区分；确认检查不会改写用户索引标记。
4. 忽略文件、新文件和损坏索引；损坏索引保持原样且不返回成功空集。
5. 非祖先 base 不能被当作没有变化。
6. external diff／textconv／fsmonitor 钩子不能在捕获时运行；另用未受约束的 Git status 作为 fsmonitor 确会执行的正对照。仅在本地临时夹具中执行。
7. 通过实际 `SourceBroker::call("git.changed_files")` 验证持久清单与未知覆盖语义，不读取捕获后的新增文件。

开发日志：

- `/tmp/oviraptor-source-scope-diff-red.log`：原逻辑下 2 项真实缺陷回归失败，session 73291 退出 101。
- `/tmp/oviraptor-source-scope-diff-green.log`：修复后原 2 项通过，session 78360 退出 0。
- `/tmp/oviraptor-source-scope-snapshot.log`：20 通过／1 失败，session 59995 退出 101；失败是新测试把两个 `update-index` 清除操作放在一次调用中，最后仍残留隐藏标记。拆成两次明确操作，未放宽生产判断或断言。
- `/tmp/oviraptor-source-scope-snapshot-v2.log`：快照相关 22 项通过，session 34653 退出 0。
- `/tmp/oviraptor-source-scope-native.log`：Native Pipeline 61 项通过。
- `/tmp/oviraptor-source-scope-retirement.log`：残留守卫 4 项通过；没有更新 allowlist。

## 4. 最终组合验证

Rust 顺序流水线 session 46228 已退出 0：Native Pipeline 61 → 残留守卫 4 → 完整 Rust 主库 1034（447.08 秒）、导入器 30（1.99 秒）→ 全目标全特性 Clippy `-D warnings` → fmt 均通过。完整阶段日志为 `/tmp/oviraptor-source-scope-full.log`、`...-clippy.log`、`...-fmt.log`。执行期间没有编辑或重新编译 Rust 输入。

前端／localhost 流水线 session 87851 已退出 0：七组实际 SFC 测试 146 通过 → vue-tsc／Vite 通过 → 本地浏览器回环 `passed=true`（匿名／比较采集 complete、8 个观察请求、身份隔离 true）。日志 `/tmp/oviraptor-source-scope-ui.log`、`...-build.log`、`...-loopback.log`。主 JS 828.82 kB，已有的 >500 kB 拆包警告仍保留。运行仅使用本地临时仓库、SQLite 和 localhost；没有部署、外部目标扫描或主机执行能力变更。

## 5. 必须继续实现的范围合同

以下仍未完成，不能用本次清单修复替代：

1. `WorkbenchStartRecord` 尚未将 `scopeMode/diffBase` 作为发布时不可变、按 attempt 的执行合同保存。`task.json` 有字段，不代表线程获得了可信的冻结范围。
2. 源码入口仍给分析器整个 `snapshot.frozen_root`；CI 仍硬编码请求 `CiScope::Diff`。正确清单本身不会自动改变实际输入范围。
3. 应保留整仓来源 manifest，另建实际分析 manifest 和独立只读材料化视图。显式 full 忽略隐藏旧 diffBase；显式 diff 不允许隐式整仓回退；auto 的回退必须有用户可见含义和原因。
4. 范围合同至少绑定 scan／attempt、源码根目录、请求模式与 base、已解析 base／HEAD、来源 tree hash、实际文件集合／摘要、实际模式与回退原因；持久化失败或恢复不一致不能重新推断权限。
5. 删除／排除／超限文件、空增量、不可用 base、Git 特殊索引和配置、扫描中变更需要明确覆盖结果。当前 Git 命令还没有统一的进程 deadline／输出上限，也未证明对所有 stat-cache／Git 配置组合的逐字节完整性；不能把七项回归扩张为任意仓库都完全覆盖的证明。
6. CodeQL 的项目上下文需求需要独立合同。不能只给残缺目录却声称完成整项目分析，也不能为满足它而隐式扩大显式 diff 的输入。
7. CI 的 scope、fileCount、分析 manifest 与真实分析器输入必须一致；源码独立 Reviewer 的 root／attempt／候选 revision 和去重仍按主计划继续实现。

验收必须检查分析器实际收到的目录与字节，而非仅检查 JSON 中的 scope 标签。现有整仓来源完整性检查不得通过裁剪 `snapshot.files` 规避。
