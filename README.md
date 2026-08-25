# Oviraptor · 窃蛋龙

一个面向授权资产管理场景的跨平台桌面端。
从自动化挖掘src扫描器改进而来
现去除老版扫描器特征，全部采用agent

## 已实现

- 项目管理
- 资产搜集
- 完全strix化，解决云端大模型空烧token问题，LLM自动适配7B/9B/27B/35B模型的最大token和上下文宽度
- 针对前后端项目进行优化
- 针对登录后多账号数据进行优化
- 扫描项目越多，会进行自我升级，沉淀知识复用
- 想不起来了


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
