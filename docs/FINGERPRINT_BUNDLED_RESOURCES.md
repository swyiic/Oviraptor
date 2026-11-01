# 随应用发布指纹资源：1.1.119

2026-10-30，本地 macOS arm64 发布构建验证完成。

## 资源与行为

- 全量包：`src-tauri/resources/fingerprint-packs/default.fingerprints.json`，798 个唯一 ID（原始 23 + 兼容规则 699 + 条件树 76）。
- 清单：同目录 `manifest.json`。Tauri 明确包含清单和 `*.fingerprints.json`，无需手工复制。后续更新见 [资源维护说明](../src-tauri/resources/fingerprint-packs/README.md)。
- 普通 Tauri 构建先用 Rust 校验文件/规则哈希、数量、全局 ID 唯一性、严格字段、条件树和实际 Rust 正则；未索引包或不合格包会阻止构建。
- 校验后的同一份规则也嵌入原生二进制，不依赖运行位置或外部数据库。空数据目录直接加载 798 条，不创建用户指纹文件。
- 原始 23 条继续采用原文件的定义、顺序和匹配行为。用户规则按同 ID 覆盖默认项，已有 699 + 76 条副本有效总数仍为 798；移除自定义覆盖后恢复默认值。
- 用户配置不做自动迁移或清理。独立指纹包不包含 CVE 库、PoC 或数据库。

## 验证

116 项相关回归通过：资源校验 14、历史指纹转换/布尔语义 53、查询界面 21、Rust 条件树/指纹包 17、生产加载/匹配链路 11。完整前端类型检查及生产构建通过，正常 Tauri 发布构建通过。

原生回归包括：空配置 798 条且不写用户文件；已有 699 + 76 条副本去重且字节不变；自定义覆盖和恢复默认；全量包重复导入不产生用户副本。另以实际源码离线适配器复核空目录、当前配置及全量导入，结果一致。

成品 App 的 `Contents/Resources/resources/fingerprint-packs/default.fingerprints.json` 及同目录清单均与源码资源逐字节一致；包内 798 个唯一 ID，未发现数据库文件。App 为 arm64，版本 1.1.119，深层严格签名检查通过；当前前端入口和 `condition-tree-v1` 已包含在二进制中。

当前用户的两份指纹文件、原始指纹与主动检查规则文件均与本轮开始时 SHA-256 一致。未安装/启动新版 App，未操作真实数据库或 CAS；界面行为证据限于源码回归和前端构建。

## 产物

- App：`src-tauri/target/release/bundle/macos/Oviraptor.app`
- ZIP：`src-tauri/target/release/bundle/Oviraptor-1.1.119-macos-arm64.zip`，31,118,645 字节，压缩完整性检查通过。
- ZIP SHA-256：`56fb8273941bd57a7637862c5b0dd7dea811c06d907a1bd1e5811e7988abd9d2`
- 指纹资源 SHA-256：`e0b55405548d7414a26fd4de2ed28852547b8088ec187d885afb1ccd52615726`
- 规范化规则 SHA-256：`cc6954e907010e5b8fc62dd5b51d494bf844afa0a79c9754c70fb8247d8ff6a5`

本轮日志和机器校验结果保留在 `/private/tmp/oviraptor-bundled-packs-20261030/`；临时目录可能被系统清理，长期维护以仓库内资源清单与上述构建校验为准。
