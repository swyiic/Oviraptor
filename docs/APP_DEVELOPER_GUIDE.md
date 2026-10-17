# Oviraptor 应用开发指南（活文档 · v1）

> 面向 **agent-first 宿主**上的可安装目录包，不是 Vue 桌面插件路径。  
> 协议与预算以 [`packages/app-sdk/README.md`](../packages/app-sdk/README.md) 为准；宿主行为见 [`packages/agent-host/README.md`](../packages/agent-host/README.md)。  
> 总架构见根目录 [`ARCHITECTURE.md`](../ARCHITECTURE.md)。

---

## 1. 受众与前提

- Node **24+**；在仓库根构建宿主与打包工具。
- 会编写 ESM（`worker.mjs`）、理解 JSON-RPC 与「声明权限 ≠ 授予」。
- 不需要改 `packages/agent-host` 源码即可安装第三方示例包（如 `examples/echo-app`）。

---

## 2. 快速开始

```sh
# 仓库根
npm run build:agent-first
npm run build:app-packages

# 打包示例（源目录与输出必须分离）
node packages/app-sdk/bin/pack-app.mjs examples/echo-app /tmp/oviraptor-echo-package

# 启动宿主（数据目录须为绝对路径）
node .agent-build/agent-first/packages/agent-host/src/cli.mjs --data-dir /absolute/private/host-data
```

打开宿主打印的 loopback URL → Applications 安装生成的目录包 → **启用**并显式选择项目权限。安装默认禁用且零授权。

可选：`--runtime grok-acp` 显式选择官方 CLI 运行时；省略则沿用已保存偏好；`--runtime none` 为仅保存模式。宿主**不**读取 OAuth 令牌。

---

## 3. 目录包格式

### 3.1 可安装输出

必须包含：`manifest.json`、`worker.mjs`、清单声明的全部 `files`（含页面与 vendored `sdk/`）。

- 文件均为普通文件；**拒绝**符号链接与未列出文件。
- 每条路径带精确小写 SHA-256；`manifestSha256` 对 `manifest.json` 原始 UTF-8 字节哈希。
- 路径规则：便携相对路径；禁止 `..`、绝对路径、反斜杠、大小写冲突等（见 SDK）。

### 3.2 源目录 `app.package.json`

作者源含 `app.package.json`（manifest 字段不含 `files`）+ 显式文件列表；`vendorSdk:true` 时打包器写入 `sdk/` 下四个运行时模块。

```sh
node packages/app-sdk/bin/pack-app.mjs apps/assets /tmp/oviraptor-assets-package
node packages/app-sdk/bin/pack-app.mjs apps/scanning /tmp/oviraptor-scanning-package
```

预算（`MANIFEST_LIMITS`）：manifest ≤128 KiB；≤256 文件；单文件 ≤8 MiB；整包 ≤32 MiB；≤64 tools / 16 pages / 64 permissions。Host API 兼容目前精确为 `^1.0.0`。

---

## 4. Worker 协议（stdio JSON-RPC）

UTF-8、LF 分隔的 JSON-RPC 2.0；无 notification/batch。输入行 ≤128 KiB；方法结果 ≤64 KiB；并发请求 ≤32；活跃 tool ≤16。stdout 仅协议；异常对调用方变为通用 `app_internal_error`。

| Method | 要点 |
| --- | --- |
| `describe` | 返回 protocol/app/tools/pages（须与已装 manifest 一致） |
| `lifecycle/start` | `{dataDir, context}`；在此迁移自有数据；无项目绑定 |
| `tool/call` | `context` 含 `hostBoundProjectId`、`grantRevision`、`operationId` |
| `lifecycle/stop` | 可 `{busy:true}` 拒绝；进行中的 tool 时 SDK 直接 busy |

**切记**：项目范围以宿主 context 为准；不要信任 tool arguments 里的「项目 ID」替代 context。宿主在派发前与发布结果前复核 grant。

```js
import { startStdioWorker } from './sdk/worker.mjs'; // 安装包内路径

const controller = startStdioWorker({
  manifest, // 读取已安装 manifest.json
  tools: {
    echo: async (args, context) => ({
      content: [{ type: 'text', text: args.text }],
    }),
  },
  onStart: async ({ dataDir }) => ({ dataSchemaVersion: manifest.dataSchemaVersion }),
  onStop: async () => ({ busy: false }),
});
await controller.closed;
```

公开工具名为 `appId::toolName`。输入 schema 为有限子集（闭对象、有界数组等）；见 SDK README。

---

## 5. 权限与宿主生命周期

| 动作 | 效果 |
| --- | --- |
| 安装 | 复制不可变版本目录；**不**授予权限 |
| 启用 | 可参与能力交集；仍需项目 grant |
| 禁用 | 停止新准入并撤销 grant；不默认强杀已接受的持久任务 |
| 卸载 | 忙则拒绝；成功则保留数据与包字节，**不**恢复 grant |
| 升级 | 须已禁用 + 当前 revision + 另行审阅的新目录包（`POST /api/apps/:id/upgrade`） |

有效能力 = 已安装 ∩ 已启用 ∩ 兼容/健康 ∩ **当前项目显式 grant** ∩ 运行时披露。Skills 同意与应用权限分离。

---

## 6. 编写应用

1. 定义 `app.package.json`（id 用小写 DNS 标签，semver，`hostApi: "^1.0.0"`）。
2. 实现 `worker.mjs`：`startStdioWorker`，工具处理器，`onStart` 迁移，`onStop` busy 语义。
3. 页面只做宿主 iframe 握手；**不要**在页面里放 host token 或经 postMessage 调工具。
4. 自有 SQLite/文件只写在宿主分配的 `dataDir`；按 `hostBoundProjectId` 隔离。
5. 打包到空输出目录；用 `validatePackageDirectory` / 宿主安装路径再校验。

---

## 7. 页面

- 隔离 iframe；无同宿主任意 API 特权。
- 工具只经**已授权**宿主 API；页面消息按 frame 身份与 navigation nonce 白名单处理。
- 关闭页面只隐藏 UI，不等于 disable。

---

## 8. 参考：Assets 应用

见 [`apps/assets/README.md`](../apps/assets/README.md)。

| Tool | 行为摘要 |
| --- | --- |
| `inventory_import` | 最多 128 条显式记录；键 `(project, source, externalId)`；operationId 幂等 |
| `inventory_stats` / `inventory_search` | 仅已导入元数据；非「发现完备性」 |
| `legacy_snapshot_stats` | Node worker 中为 `legacy_managed_bridge_unavailable` |

非目标：provider 爬取、归属合并、扫描授权。

---

## 9. 参考：Scanning 应用

见 [`apps/scanning/README.md`](../apps/scanning/README.md)。

| Tool | 行为摘要 |
| --- | --- |
| `draft_create` | 存草稿 + 首条 journal；**不**启动引擎；返回未授权/未执行标志 |
| `draft_cancel` / `task_status` / `journal_read` | 草稿与只读状态/日志 |
| `legacy_saved_scans` | Node 中桥接不可用，不伪造扫描列表 |

`thin-service.mjs` 仅 Phase-1 表面。真实执行仍在桌面 Native / `oviraptor-scan-service` 路径。

---

## 10. Legacy bridge 注意

- Rust `legacy_app_bridge` + `oviraptor-app-worker`：只读、显式 staged `legacy-input.sqlite3`。
- 已装 Node worker **不能** spawn 该二进制。
- `createApplication(manifest, { legacyBridge })` 为受信嵌入；宿主托管运输未接通前，勿在应用里假设「已连上桌面库」。

---

## 11. Skills（宿主功能）

- 面板可预览/导入本地包、管理随包目录、按会话选择参考材料。
- 同意绑定运行时、库 revision 与内容指纹；撤销后扣留后续资源字节。
- **脚本惰性**；Skills **不**授予 app permissions。
- 不静默吞并桌面已有 Skills 数据。

---

## 12. 测试

```sh
node --test --test-concurrency=1 packages/app-sdk/test/*.test.mjs examples/echo-app/test/*.test.mjs
node --test apps/assets/test/*.test.mjs apps/scanning/test/*.test.mjs
```

绿测证明：本地 SQLite/协议/权限夹具行为。  
**不**证明：真实 Grok 付费、connected native bridge、完整域迁移、真实目标扫描。

---

## 13. 打包清单

- [ ] 源目录 ≠ 输出目录；输出不被静默覆盖
- [ ] `hostApi` 为 `^1.0.0`；无未知 manifest 字段
- [ ] 无符号链接；hashes 与字节一致；`sdk/` 仅来自 vendoring
- [ ] 每个 tool 绑定已声明 permission；schema 落在 SDK 子集内
- [ ] 结果与 describe 遵守 64 KiB 量级预算
- [ ] 文档与证据不塞进 `docs/` 大 tar/png 树

---

## 14. 证据存放

不要把大型日志、截图包、source-before/after JSON 堆进 `docs/`。架构与发布观察见 [`ARCHITECTURE.md`](../ARCHITECTURE.md) §9 与根 README 的 `tools/release/*`。历史 `NEST_*` / 日期戳审计拟迁 `docs/archive/`。

### TODO

- [ ] 壳层 `shell-apps.json` 能力标签 ↔ worker 工具 对照表（避免产品文案超实装）。
- [ ] 官方页面握手消息字段若有独立 IDL，可在此链到源码符号（当前以 host/page-client 实现为准）。
