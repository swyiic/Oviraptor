# Web 派发工具入口身份与即时熔断复核（2026-09-26）

后续状态：普通 Web、从未派发且输入未变化的人工恢复 API／UI 见 `NEST_WEB_MANUAL_RECOVERY_AUDIT_2026-09-26.md`。本文「不是同 attempt 恢复 API／UI」描述本次工具绑定增量的历史边界；不再代表后续代码完全没有恢复入口。工具链封存、沙箱和未知执行结果对账限制仍然有效。

## 1. 本增量的交付边界

承接 `NEST_WEB_DISPATCH_BINDING_AUDIT_2026-09-26.md`。普通 Web attempt 的私有启动绑定新增 **Node／浏览器候选入口文件清单**；首次领取执行权前后重新生成清单，工具安装、删除、内容替换或链接改指均不能被当作原配置。加入即时熔断查询，修复启动事务提交后新增目标熔断仍可能取得派发权的窗口。

这不是完整隔离沙箱，不是全部依赖供应链证明，不是同 attempt 恢复 API／UI，也不是整个 Master Plan 完成。未新增 Host Agent；继续 `web_only`。本轮没有部署、没有访问用户提供的外部 URL。

## 2. 实现

### 2.1 一个候选发现来源

- `helper_node_candidates` 提取 Node 候选枚举，实际版本选择器和绑定使用同一个函数。
- 枚举包含显式 `OVIRAPTOR_NODE_EXECUTABLE`、runtime PATH 中的入口和非 Windows 的原有系统回退路径。
- 保留不存在的候选，而不是只登记已选中的 Node。否则在未变更 PATH 的情况下安装更高版本工具，可能改变实际选择结果而不改变绑定。
- 浏览器候选位置提取为 `resources/config/browser-locations.json`，Rust 清单和 Node 浏览器助手共同读取；不维护两套手写列表。
- 该资源在既有 Tauri `resources/config/*` 打包范围内；`verify_bundled_worker` 在启动浏览器助手前验证它与编译内嵌内容一致。缺失或篡改均拒绝。
- Windows 的 runtime PATH 不再主动拼入 Unix 的 Homebrew／系统目录。Windows 实际执行验收仍未完成，不因该条件分支存在而宣称已支持全部文件系统保证。

### 2.2 只读清单，不运行候选程序

`web_toolchain_binding` 不调用 `node -v`，不启动浏览器，不下载或安装工具。对每个候选记录：

1. 候选路径与存在性。
2. 存在时的规范化路径、长度和修改时间。
3. Unix 设备号、inode、权限模式和 ctime。
4. 文件内容 SHA-256。

工具安装常用符号链接，因此允许候选解析到常规文件；同时绑定原候选和最终路径。打开最终文件使用 Unix `O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC`，并核对打开前、句柄、读后路径及链接解析的一致性。拒绝目录、FIFO、相对路径、不可读资源、非 UTF-8 路径及超限输入。

限制：单个入口文件最大 512 MiB、每次清单实际读取总量最大 2 GiB、Node 与浏览器合计最多 256 个候选。摘要使用流式读取，不把二进制全部加载到内存。没有用 mtime 缓存替代内容验证，也没有进行启动速度承诺。

清单只是原内存 descriptor 的一个字段，经既有 HMAC 纳入私有派发凭据；不另存明文凭据或把环境变量发往 IPC。新增 `OVIRAPTOR_NODE_EXECUTABLE`、代理、TLS 及选定动态加载环境变量绑定。错误码不包含工具内容、认证材料或环境变量值。

### 2.3 即时熔断不是静态配置

`web_binding_message` 在同一领取事务中查询当前 attempt 目标对应的有效 `sentinel_fuse_zone`。查询失败拒绝；目标已熔断返回 `web_binding_target_fused`。

首次 preflight 和 claim 后 preflight 都走该查询。AFTER claim trigger 新增熔断时，claim 与 trigger 写入一起回滚；不构造失败 guard、不发出工具／模型／目标请求、不重写 scan 或 branch 终态。已归档的熔断和不相关 URL 不阻止该 attempt。

这只封住**首次派发准入窗口**；不是任务运行期间每个请求、所有分支或所有任务类型的完整授权证明。恢复时还需要生命周期锁、当前 attempt、范围、身份、预算、后端计划及运行中撤权处理。

## 3. 修复上轮回归失败

`binding-full.log` 和 `binding-final-full.log` 都实际报告 848 成功／1 失败，不能写成 849 全绿。失败的旧任务测试在 macOS 用 `/var/...` 别名访问规范化为 `/private/var/...` 的目录，先触发 `web_binding_directory_not_canonical`，未走到原定的 `web_binding_receipt_missing`。

已只修正测试 fixture 的路径为生产持久化采用的 canonical path，保留生产目录检查。此前“历史任务不补造凭证”测试的最终验收以本增量最新全量日志为准。

同时保留前增量修正的 SIGKILL 验证：子进程 stdin 保持到退出，检查 kill 调用成功，并在 Unix 断言信号，不能用任意非零退出冒充强杀证据。

## 4. 自动化证据

新增 `tests_web_toolchain_binding.rs` 的 6 项测试：

- 不存在→安装→相同长度改写→删除；可执行脚本 fixture 不被运行。
- Node 共享候选枚举、缺失 override 保留、稳定去重及系统回退。
- 共用浏览器配置的 macOS／Linux 候选顺序和 Windows 环境根组合。
- 相对路径、目录、FIFO、超大稀疏文件、候选数量与总读取预算拒绝。
- Unix 符号链接改指和相同内容不同 inode 替换改变绑定。
- 工具变化使真实私有 HMAC 凭据失效，且不产生 claim、不改变扫描终态。

扩充真实 bundled worker 完整性测试：浏览器位置配置缺失／篡改拒绝，恢复编译内嵌内容后通过。原只读 runtime 测试验证新 descriptor 确实含 Node override 和工具清单。

另新增 1 项派发熔断回归，使用真实 `NativeBranchGuard::claim_with_preflight`：有效目标熔断拒绝、归档／无关目标不误拦、AFTER claim 新增熔断原子回滚、解除后仅正常领取一次。

最终质量门禁全部通过，最新 Rust 链进程退出码为 0：

| 门禁 | 实际结果 | 日志（前缀 `/tmp/oviraptor-20260926-toolchain-`） |
| --- | --- | --- |
| `cargo fmt --check` | 通过 | `verified-fmt.log` |
| 离线全 targets／features Clippy，`-D warnings` | 通过 | `verified-clippy.log` |
| 离线全 targets／features Rust，串行测试 | 主库 856／历史导入器 30，0 失败 | `verified-full.log` |
| 实际 Vue SFC 测试 | 76 通过，0 失败 | `ui.log` |
| `npm run build` | 通过；主 JS 792.37 kB 拆包警告保留 | `build.log` |
| Native 本地浏览器回环 | 通过，8 次请求，身份隔离 true | `native.log` |
| `git diff --check` | 通过 | `diff.log` |

第一次工具增量全量 `final-full.log` 为主库 855／导入器 30 通过，但不包含随后新增的熔断测试；当前验收只以最后的 `verified-full.log` 为准。6 项定向测试的 `targeted.log` 只作为中途证据。完整源代码验收结束后仅更新审计和进度 Markdown，不再改动本轮受测 Rust／JavaScript／JSON。

## 5. 仍需完成，不能放宽的边界

1. **入口文件清单不是完整工具链封存。** Node 共享库、浏览器 app/framework、脚本解释器及二级依赖不在此清单里；选定环境变量也不是全部可能影响子进程的环境。
2. 校验后到实际 spawn 之间仍存在宿主工具更新窗口。当前没有不可变镜像／包快照或从已验证句柄执行机制，不能宣称抵抗本地管理员或并发替换。工具供应、隔离运行与逐调用约束需要独立交付。
3. 显式同 attempt 恢复必须人工发起，持有生命周期锁和 branch 活锁、证明 never-claimed、复核完整授权并在执行前持久 claim。要把已取得的 guard 交给 worker，不能恢复入口 claim 后让 launcher 再 claim。
4. 已 claim、无历史凭据及未知副作用必须另走结果对账；不能清空 claim、生成替代 key 或用 UI 轮询自动重放。
5. UI 尚无配置变化专用状态卡和安全恢复按钮；同 attempt 恢复不得因本增量改成 `automaticReplayAllowed=true`。
6. 仍未完成源码／CI／combined 完整启动事务、完整桌面强杀／断电验证、独立 Linux 隔离运行、安装包与实际授权环境验收，以及 Master Plan 其余多角色／知识生命周期要求。

历史 JSON 兼容、Strix 退役清单均未因本增量放宽；不存在为了通过测试增加历史运行时豁免的操作。
