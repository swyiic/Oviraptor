# 资产探测与规则同步修复（1.1.124）

## 修复内容

资产探测按速率/并发、超时/重试和执行范围分组，字段与选项使用统一的紧凑布局。代码规则库统一仓库字段、操作按钮、版本与时间信息；长差异清单默认折叠，可展开滚动查看最多 200 项已保留明细。窄窗口切换单列，完整版本号和错误可以换行阅读。分类配置目录与保存范围说明同步整理；原有值、字段名、保存范围及权限约束保留。

截图中的 Git `unknown revision` 来自下载后的历史版本比较：数据库保留的旧短 SHA 在当前浅克隆中可能不存在，旧流程因比较失败把整个同步判为错误，且不会记录本次完整版本，重试会重复失败。现在同步前解析并保留完整比较基线，成功后存储完整 commit ID。

- 历史版本仍可解析：显示真实增删改。
- 历史版本不可用，但同步前本地 HEAD 可用：明确使用实际本地版本重新建立比较基线。
- 两种基线均不存在：已下载规则仍标为就绪，显示“历史比较基线不可用”，隐藏未知差异计数；后续同步可以使用新保存的完整版本。
- 网络、检出及未匹配的仓库来源错误仍如实显示，不会伪造同步成功。

OWASP 旧仓库已成为迁移说明页。内置来源改为 `OWASP-Benchmark/BenchmarkJava.git` 的 `master`；仅旧官方默认配置在显式同步时兼容迁移，自定义仓库和其他分支保持原设置。[官方迁移说明](https://github.com/OWASP/Benchmark/blob/main/README.md)、[当前仓库](https://github.com/OWASP-Benchmark/BenchmarkJava)。

## 预览

以下是实际 Vue 组件与生产样式的离线样本，数据、版本号与同步状态均为合成适配器提供，不代表真实用户仓库已经同步。

- [资产探测](ui-settings-1.1.124/probe.png)
- [代码规则库](ui-settings-1.1.124/rules.png)
- [基线不可用提示](ui-settings-1.1.124/rule-no-baseline.png)
- [差异统计与明细样式](ui-settings-1.1.124/rules-comparison.png)
- [错误状态](ui-settings-1.1.124/rules-error-state.png)
- [窄窗口规则页（深色）](ui-settings-1.1.124/rules-narrow-dark.png) / [窄窗口探测页](ui-settings-1.1.124/probe-narrow.png)

## 验证与使用

12 项后端回归通过：直接编译生产 Git/SQLite helper，用临时仓库与临时数据库覆盖首次下载、旧短 SHA、浅更新、缺失基线、失败保留和 OWASP 迁移失败后的重试。40 项原浏览器断言加 2 项窄屏实际列数检查通过；两页在 4 种宽度与 2 种主题下共 16 种布局，无横向溢出或浏览器运行错误。前端类型检查与构建通过，独立差异审查通过。

公开仓库的远端引用检查确认 Semgrep `develop`、CodeQL `main` 及迁移后的 BenchmarkJava `master` 可访问；旧 OWASP 地址的 `master` 不存在。此检查验证远端引用，不等同于完整拉取这三个仓库。

真实数据库、规则缓存与 CAS 未访问，也未安装或启动新版本。升级后在代码规则库逐项点击“同步并比较”，成功结果才会写入实际配置并清除原错误。无需删除已有缓存。完整检查及源码摘要见 [checks.json](ui-settings-1.1.124/checks.json)。

## 构建产物

[Oviraptor 1.1.124（macOS arm64 ZIP）](../src-tauri/target/release/bundle/Oviraptor-1.1.124-macos-arm64.zip)，31,163,767 字节。

原生发布构建通过；App 与 Host 分发版本均为 1.1.124。本地 ad-hoc 签名深度校验、ZIP 完整性校验通过；125 个关键文件与 App 内容逐字节一致，五个本轮源码文件摘要与构建前一致。未进行 Apple 公证。

SHA-256：`aaa01a2c7c5050f83233502c9c1d3347ccba1710eee181ceddf56936149d6fd8`。详细记录见 [release.json](ui-settings-1.1.124/release.json)。
