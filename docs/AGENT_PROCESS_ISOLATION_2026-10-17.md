# Phase 4 进程隔离（最小方案）— 2026-10-17

对应连夜目标 G11。不碰 Nest 内核。

## 边界

| 角色 | 进程 | 职责 |
|---|---|---|
| Host（agent-host） | 主进程 | 会话、授权（grants）、`apps.call` 门禁、工具发现、IPC 调度 |
| App worker | 独立 child process（`AppProcess`） | 执行已安装应用的 tool/action；拥有应用数据目录 |
| Nest / 扫描内核 | 另案（本阶段不迁） | 仍不链入 Host 进程 |

- 应用入口：schema v1 用 Node worker（`process.execPath` + 包内 `entry`）；schema v2 native 用声明的二进制 + `--app-worker`。
- 所有调用经 JSON-RPC over stdio（`tool/call` / `action/call` / lifecycle），**权限只在 Host 判定**，worker 收到的 context 已带 `hostBoundProjectId` / `grantRevision`，worker 不能自行提权。
- Host 构建与运行时不 `require` / 链接应用业务 crate；应用逻辑只在 child 内执行。

## 崩溃隔离

1. Worker `exit` / `error` → `AppProcess.fail(code)`：拒绝未完成 pending，标记 `stopped`，native 侧可进入 `owner_unresolved` 检疫。
2. Host 主进程不随 worker 退出；store、会话、其它应用的 worker 继续可用。
3. 对已 `stopped` 的 worker，后续 `apps.call` 返回结构化错误（如 `host_app_failed` / `host_app_process_exited`），**不自动静默复活**（避免半状态双开）。
4. 恢复路径（显式）：操作员 disable/enable、Host 重启后重新 `startWorker`，或升级流程中的受控替换。强制 kill busy native owner 不是成功卸载。

## 健康可见性

- `AppManager.view()`：`status` 在 worker stopped 时为 `failed`；native owner 未决时为 `owner_unresolved`；`busy` 反映 active / nativeBusy / unresolved。
- Activity 回调只传 `appId` / `busy` 元数据，不把应用原始 payload 灌进 SSE。

## 最小验收（本提交）

- 方案本文档。
- Smoke：`packages/agent-host/test/process-isolation-smoke.test.mjs`  
  spawn fixture app → `apps.call` → `SIGKILL` worker → Host 仍可 list/store/send → 再次 call 得结构化错误。

## 非目标（本阶段不做）

- 不把 Nest/Asset 扫引擎拆成独立 OS 服务。
- 不实现自动 crash respawn 循环。
- 不改 exposure / Nest 同质主管内核。
