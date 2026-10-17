# docs/archive/

历史日期戳审计、交付说明与任务提示的归档区。**活真源**仍是仓库根 [`ARCHITECTURE.md`](../../ARCHITECTURE.md)、[`docs/APP_DEVELOPER_GUIDE.md`](../APP_DEVELOPER_GUIDE.md) 与各包 README；勿把本目录当作当前验收依据。

体积证据树（原 `docs/agent-first-20261016/`、`docs/asset-accuracy-20261015/`）已外置出仓，见 [`../OFFLOAD_NOTE_20261017.md`](../OFFLOAD_NOTE_20261017.md)。

## 子目录地图（本次搬迁后）

| 子目录 | 文件数 | 内容 |
| --- | ---: | --- |
| [`nest-audits/`](nest-audits/) | **285** | 全部 `NEST_*.md`（审计 / 交付 / 设计 / 计划） |
| [`agent-first/`](agent-first/) | **23** | `AGENT_FIRST_*`、平台拆分规格、agent 平台相关日期笔记 |
| [`asset-exposure/`](asset-exposure/) | **12** | `ASSET_EXPOSURE_*`、暴露面计划、Qoder handoff |
| [`general-audits/`](general-audits/) | **16** | 模块边界 / 增量集成 / 外部资产精度 / Native·Rust 需求 / 已归档发行说明 / 根 `transfer.md` |
| [`task-prompts/`](task-prompts/) | **3** | 历史任务提示与 overnight goal |
| [`integration/`](integration/) | **4** | 原 `docs/integration/` 模块搬迁 / 验证 JSON |
| [`local-incremental-20261015/`](local-incremental-20261015/) | **7** | 原 `docs/local-incremental-20261015/` 映射与校验 JSON |
| **合计** | **350** | 含 markdown + 上述 JSON |

## 仍留在 `docs/` 根（未搬）

| 路径 | 原因 |
| --- | --- |
| `APP_DEVELOPER_GUIDE.md` | 活应用开发指南 |
| `OFFLOAD_NOTE_20261017.md` | 证据外置说明（活指针） |
| `architecture.md` | 旧架构 stub；内容已吸收进根 `ARCHITECTURE.md`，路径暂留以免外部书签断裂 |
| `OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md` | 同上（文首状态已过时，以根架构为准） |
| `progress.md` | 超长实施流水；未归入明确审计桶，暂留 |

## 不搬迁（明确排除）

- 仓库根 `ARCHITECTURE.md`、`README.md`
- 各 `packages/*/README.md`、`apps/*/README.md`
- 本文件 `docs/archive/README.md`
