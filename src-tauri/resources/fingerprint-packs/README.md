# 随应用发布的指纹包

`default.fingerprints.json` 保留原有 798 条指纹（原始 23 条、699 条兼容规则、76 条 AND / 混合条件规则），文件字节不改。`enterprise.fingerprints.json` 追加 23 条经单独复核的企业框架与运行组件规则，`frameworks.fingerprints.json` 再追加 11 条明确技术头规则，当前合计 **832 个唯一 ID**。企业补充的依据、限制和未确认候选见 [覆盖说明](../../../docs/FINGERPRINT_ENTERPRISE_COVERAGE.md)。仅包含产品识别特征及来源信息，不包含 CVE 库、漏洞利用代码或用户数据库。

## 自动包含与加载

- Tauri 的资源清单包含 `manifest.json` 和当前目录下的 `*.fingerprints.json`；普通 `tauri build` 即会打包，无需手工复制。
- `build.rs` 验证所有清单条目的文件 SHA-256、规范化规则 SHA-256、数量、唯一 ID、字段、条件树限制和实际 Rust 正则。失败会终止构建；未登记的指纹包也会使构建失败。
- 构建时还把相同的已验证内容嵌入原生程序。全新数据目录直接获得默认指纹，不需要导入，不创建用户规则文件，不依赖外部数据库。
- 原始 23 条的定义和顺序继续使用原始规则文件；包中相同 ID 不重复追加。旧规则匹配语义保持不变。
- 现有用户规则按 ID 覆盖默认值。已有的 699 + 76 条副本不会让总数重复增长，移除自定义覆盖后恢复默认规则。升级不改写用户配置或真实数据库。

## 更新资源

1. 使用 Ovi 独立指纹包格式导出或规范化新包，保持严格规则语义，不把无法可靠转换的条件降级为 OR。
2. 放到当前目录，以 `.fingerprints.json` 结尾。所有包之间 ID 必须唯一，单包及整个默认库均受格式/数量限制。
3. 在 `manifest.json` 的 `packs` 数组登记 `file`、整个文件的 `sha256`、去重条数 `count` 和包内规范化规则的 `ruleSha256`（对应指纹包的 `sha256`）。
4. 可先执行 `python3 tools/verify_bundled_fingerprint_pack.py` 做离线结构/哈希检查。实际 Rust 正则仍由 Cargo 构建的校验器检查。
5. 正常执行 Tauri 打包。资源包变更会触发重新校验和编译；不要在构建进行时修改资源文件。

来源完整保留在包的 `manifest.sources` 中。这里的 README 不在 Tauri 资源通配符中。

框架复核及未启用 HTML 候选见 [框架覆盖复核](../../../docs/FINGERPRINT_FRAMEWORK_AUDIT.md)；Agent 任务内查询见 [交互说明](../../../docs/FINGERPRINT_AGENT_INTEGRATION.md)。
