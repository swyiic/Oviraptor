# 指纹查询与布尔条件升级（1.1.118）

实际页面位于 **设置 → 指纹库 → 指纹检索**。支持名称、ID 和特征文本搜索，忽略大小写和全半角差异；总数按精确 ID 去重，内置和自定义来源可筛选，每页 25 条。展开后展示 Header、Cookie、正文、特征路径及完整带括号的条件树。读取失败时不显示陈旧的成功统计。

现有 `fingerprints.user.json` 中未带 condition 的规则保持原来的 ANY 行为。新规则只增加到 `fingerprints.boolean.json`，必须有 schemaVersion=2 和 booleanRuntime=condition-tree-v1；旧 App 不读取这个新文件，避免旧加载器忽略 AND。内置文件与原 699 条 Any 文件均未修改。

条件树支持 all/any 嵌套，叶节点为 body/header/cookie；不是可以执行的脚本，不接受未知节点、空条件组、空匹配正则、超限树或与旧信号混杂的规则。最多 16 层、256 个节点；本次实际新增规则最多 3 层、7 个节点，没有 Cookie 或路径探测。

```json
{
  "schemaVersion": 2,
  "booleanRuntime": "condition-tree-v1",
  "fingerprints": [{
    "id": "example-application",
    "name": "Example application",
    "condition": {
      "type": "all",
      "children": [
        {"type": "body", "pattern": "BrandMarker"},
        {"type": "any", "children": [
          {"type": "body", "pattern": "FeatureA"},
          {"type": "header", "name": "x-feature", "pattern": "FeatureB"}
        ]}
      ]
    }
  }]
}
```

上例要求 BrandMarker 且 FeatureA/FeatureB 至少一个成立。不能将三项扁平化为 OR。平面 matchMode=all 会明确拒绝，须使用条件树。用户新增或批量导入会先验证；批量导入合并并保留已有 ID，不再用文件替换删除现有规则。

本次数据：原有 23 内置 +699 Any +76 新条件 =798；76 新增包括 67 条 AND、9 条混合。有限复核隔离了 8 条不可靠规则，精确正向 DNF 覆盖证明去掉了 2 条冗余规则。每个可满足分支必须有产品线索；共享脚本、缓存头或通用登录标记只能辅助 AND，不能独立触发 OR。平台 app 标签、图标、协议、未知字段、敏感会话与型号证据不明的来源仍单独保留，未为增加数量改变原条件。

共享包及逐条账本在 `/Users/swyiic/Desktop/Wiki/指纹包_布尔条件_20261030/`。完整共享包为 `oviraptor-all-798.fingerprints.json`；应用会按 ID 去重并将条件树与旧规则分开保存。原材料全部保留。

验证：53 项转换测试、21 项实际组件回归、16 项指纹逻辑/包测试和 7 项实际扫描模块回归通过；新增 76 条原生解析/合成匹配通过，6 组正常公开信息与通用页面响应中没有新增误命中。没有启动真实 App、访问真实数据库/CAS 或扫描真实资产。浏览器可视化访问被权限门禁拒绝，没有绕过访问限制；界面证据限于真实 SFC 编译、状态回归和构建结果。

```sh
python3 -m unittest discover -s tools -p 'test_*wiki*fingerprint*.py'
node --test tools/test_fingerprint_pack_ui.mjs
npm run build
CARGO_TERM_COLOR=never cargo test --manifest-path src-tauri/Cargo.toml --lib scanning::fingerprint --offline
CARGO_TERM_COLOR=never cargo test --manifest-path src-tauri/Cargo.toml --lib scanning::exposure_probe::tests --offline
```

新版已构建：`/Users/swyiic/Desktop/Rust/Oviraptor/src-tauri/target/release/bundle/Oviraptor-1.1.118-macos-arm64.zip`（macOS Apple Silicon，1.1.118）。退出旧版后解压并打开其中的 Oviraptor.app；旧版安装位置没有替换，App 没有由本次工具启动。原生条件运行时标记、当前查询页面资源嵌入、严格深层代码签名及 ZIP 完整性已核对。

完整 798 条共享包还通过了当前生产批量导入器的全新临时目录往返：导入 699 旧格式 +76 条件树，保留 23 条内置，回读共 798；重复导入两份配置文件字节均未改变，无网络/真实数据库访问。
