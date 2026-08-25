# Oviraptor · 窃蛋龙

一个面向授权资产管理场景的跨平台桌面端。
去除老版扫描器特征，全部采用Strix作为底层agent驱动，尽最大可能优化云端模型空烧token，本地无限制
界面使用 Vue 3，桌面壳和任务编排使用 Tauri 2 / Rust，数据保存在应用私有目录中的 SQLite 文件里；不要求系统预装 SQLite。

## 已实现

- 项目管理：按企业或工作范围建立项目，项目之间隔离目标、人工判定和变化记录。
- 种子导入：支持单条输入和 TXT/CSV 批量导入，可识别公司名、域名、IP、CIDR、ICP 和关键词。
- 内置 Worker：采集、归并、P1/P2/P3 分层、存活探测、SRC 结果压缩和 HackerOne 官方 API 同步脚本随应用打包。
- 任务编排：依次调用内置采集、分层和探测 Worker，实时记录进度和输出日志，可取消任务；也可以显式配置外部脚本目录覆盖内置版本。
- 配置方案：Python 路径、脚本目录、FOFA 配置、采集档位、限速、并发、超时、P1/P2/P3 及其他层级、内容分类规则均可保存为多个方案。
- 资产数据库：候选与探测结果增量写入 SQLite；保存首次发现、最近发现、最近存活、人工结论和软删除状态。
- 镜像对比：每次运行只保存资产当前状态和变化事件，不重复保存整份 CSV 镜像；原始任务输出仍按运行目录保留，便于审计。
- 查询与导出：全文检索、多字段 AND/OR 查询、项目过滤、自定义显示列、自定义字段 CSV 导出。
- 人工确认：批量标记确认/不确定，软删除后可恢复。
- 内容隔离：博彩、色情和反向语境关键词由配置方案生成任务快照，探测脚本实际加载；仪表盘和侧栏可直接进入隔离区查看，规则变化会使旧探测缓存失效。
- 增量去重：同一项目的相同网络端点复用已有资产记录并保留人工结论；历史重复项只做可恢复的软隔离，不物理删除。
- 项目生命周期：空项目可删除；已有资产的项目禁止删除，可改为归档；已有项目可从概览或项目管理直接再次扫描探测。
- 后台与提醒：macOS 关闭主窗口后驻留状态栏，任务运行时状态栏图标动态旋转；启动时提醒超期项目和上次中断的任务。
- 应用设置：可配置项目未更新时间提醒，并上传 PNG 自定义应用和状态栏图标或恢复默认图标。
- SRC 清洗：自动软隔离 5xx、无法访问入口及同站点的 HTTP/HTTPS/www 重复项，原始 CSV 和数据库记录仍保留。
- HackerOne 看板：官方 Hacker API 项目、Policy、Structured Scope、排除项、收藏和变化提醒；可把允许提交的网络 Scope 发送到资产项目。
- 本地代理：配置 Clash HTTP 代理后同时作用于 FOFA、存活探测和 HackerOne 同步。

## 本地开发

开发构建需要 Node.js、Rust 和 Python 3。发布后的 macOS 应用可在“配置中心 → 运行环境”中自动准备应用专用 Python 虚拟环境、Python 模块、Node.js、redis-cli、Docker Desktop 和 Strix CLI。Windows 自动安装使用 winget 准备 Python 3.12、Node.js LTS、Docker Desktop、Tailscale 和 Python 模块；Strix CLI 与 redis-cli 若未检测到，页面会显示手动步骤。安装过程持续显示 stdout、stderr 和失败阶段，不再只显示旋转状态。

```bash
npm install
npm run tauri dev
```

前端检查与桌面端检查：

```bash
npm run build
cd src-tauri && cargo check
```

业务模块边界、Tauri 命令归属和新增代码规则见 [`docs/architecture.md`](docs/architecture.md)。

生成当前平台安装包：

```bash
npm run tauri build
```

## 首次配置

1. 打开“配置中心”，编辑系统默认方案；需要不同参数时使用“从默认方案创建”。系统默认方案不可删除，普通方案可删除。
2. 在 `FOFA account / email` 和 `FOFA key` 中填写凭据。数据库文件在 macOS/Linux 上强制为当前用户可读写（`0600`）；运行时临时 INI 同样为 `0600`，任务结束后删除，任务快照和日志不会保存明文 Key。
3. `Scripts directory` 默认留空，使用应用内置的 1/2/3/4 Worker。只有需要调试或覆盖脚本时才填写外部目录。
4. `Legacy config path` 仅用于兼容旧版：FOFA Key 留空时才会读取该 INI。
5. 调整采集限速和探测并发。大量目标建议先以保守速率验证配额。
6. 新建项目，导入目标，然后选择“完整流程”。

应用数据库、配置、任务快照和原始输出统一放在用户主目录的 `oviraptor/` 中，数据库文件名为 `oviraptor.sqlite3`。升级后首次启动会把历史数据库安全复制到新位置，并把过渡数据库归档到 `oviraptor/database-backups/`；原历史目录暂时保留作为可恢复备份。导出文件单独放在下载目录的 `oviraptor/` 中。

macOS 默认路径：

```text
数据：~/oviraptor/oviraptor.sqlite3
导出：~/Downloads/oviraptor/
```

## 远程 Worker

推荐两台电脑加入同一个 Tailscale Tailnet，不开放公网端口，也不需要 OpenSSH、端口映射或反向代理。

1. 在 Intel Mac 或 Windows 上安装对应平台的 Oviraptor，完成“运行环境”检测。
2. 在“Worker 节点”中开启“本机 Worker”。服务只监听检测到的 `100.x.x.x` Tailscale 地址。
3. 把页面显示的节点地址和访问令牌粘贴到 M1 主控端。
4. 主控端可检测远端环境、查看与暂停/继续/取消任务，并按项目增量同步扫描结果。

M1 不能直接产出 Windows 原生安装包。仓库的 `Build Oviraptor Workers` GitHub Actions 会分别在 Intel macOS 和 Windows x64 官方构建机上生成安装包；本地 Intel macOS 构建也可执行 `npm run tauri:build:mac-intel`。应用本体不捆绑 Python、Docker、Node.js 与 Strix 的大型运行时，Worker 首次运行时检测并按平台安装。

## 数据保留策略

数据库不保存每次查询的完整资产副本，而是保存一份当前资产、项目关联和事件差异，因此长期运行时增长主要来自新资产、变化事件和日志。任务原始 CSV 会占据更多空间；确认审计期结束后，可单独归档旧的 `runs/` 目录，不要直接删除 SQLite 文件。

“失去资产”表示某资产在一次完整任务中没有再次出现，并不等价于服务宕机；存活结论以最近一次 probe 结果为准。自动内容分类用于隔离高置信结果，仍建议抽查 `blocked_content`。

已有 P1/P2/P3 探测结果可以用附带的增量导入工具写入数据库，源 CSV 不会被修改：

```bash
python3 tools/import_existing_results.py \
  --input-dir /path/to/probe_output \
  --project-name "历史资产"
```


仅对已获得授权的企业和网络范围执行采集与探测。
