# 2026-10-04 当前付费 Root 删除拒绝原因与精确保留范围

本批只收口删除拒绝的诊断与保护边界。**未实现付费任务物理删除，不把归档作为删除成功，不关闭 REM-A01 或 Master。**

原删除合同见 `NEST_SCAN_DELETION_AUDIT.md`：删除数据库任务及关联记录，未结/在途/未知不能清除，不操作产物文件/PID。当前付费 Root 的 Tick/时间线有原 run 的 RESTRICT 外键和不可变 trigger；旧删除状态检查还只接受 completed/failed/cancelled，当前真正关闭的 Root 使用 terminal。这两个合同尚未完成财务凭证独立保留与任务关联删除的设计。

## 问题与作用域

实际新 creator/HMAC/原 Root/C/父监督 → localhost Root SDK → 原费用/原 publication → 原财务退出；释放测试拥有的父监督后，仅给任务 UI completed 标签。第一项真 Red 0/1：删除返回“仍有未结算”，不能说明该原 Root 已关闭但原审计凭证仍要求保留。运行 0.66 秒、编译 32.20 秒。代码和 schema 另证实：将 run 标签改成旧 completed 也不能解决原付费 Tick RESTRICT；该改标签不是合法结算方式。Red 首例失败即停止循环，不能宣称 Red 实际执行到了改标签第二例。

仅修改删除入口：同一原 IMMEDIATE/FULL 事务、原 retired-data/状态/清理/所有权检查之后，写 tombstone/解除子链接/DELETE 之前，按原 scan ID 检查 Tick 到 run 的关联及原 timeline scan ID。存在原审计凭证则返回 `native_paid_audit_retention_required` 与中文保留/归档说明，查询失败保留并报错。不新增写表、不改原 status、不移除 immutable/外键、不解析旧格式、不删除真实数据。

## 回归与边界

新增三项实际 SDK 回归，均为临时 SQLite/localhost、脚本模型答案；这些不证明真实模型推理质量或安装态功能：

- 真付费 Root 已 terminal，以及测试故意改成旧 completed 两场景：拒绝原因准确，全应用物理行保持；原 SDK 1、费用 1、publication 1，Native JSON 证据文件逐字节保持。
- 原付费任务运行时，另一个独立 draft 删除成功且幂等；除该任务行/墓碑外全部表保持，原账单与所有 trigger 不改。全局有 paid 记录不能误挡另一 scan。
- 真实 503 原未知账单经过原退出后，任务 paused 仍不能删除；全行保持、SDK 1、publication 0、model_requests consumed 0/indeterminate 1，不退款或重发。

最初两项连旧删除回归 12/12（8.52 秒），补未知后 13/13（9.84 秒）。最终按编译产物逐名相关 25/25（13.38 秒，包含任务/项目删除、历史 trace 退役、旧别名与 Web 回执绑定）；严格 all-target/all-feature Clippy 0（19.78 秒），导入器 39/39、exact 退役 1/1。日志和实际名单在 `/tmp/oviraptor-native-deletion-retention-final-*`。不是完整 Rust/UI/Native JSON 黄金门禁；六旧失败和其他 E2E 债仍在。

## 变更保护与剩余事项

逐文件快照在修改前保存原文本和已有 Git 差异；本批 3 代码路径、2 已有/1 新。1279 文件集合 SHA `6676b63b57d6e48abbe09ff6514051be84f12032ce47db1a47c0acb21f0e59a8`，1276 原范围外文件保持，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 不变，diff--check 0，最终三份增量差异已逐文件阅读。

未改真实数据库/CAS/资产、Native 资源、前端、安装包或授权 URL；无提交。Master 的 14 项剩余总表已置顶并保持 Goal active。完整付费任务删除、十维动态 grant/精确核对/续跑、六类监督、全角色/真正并行、聊天/日志/整体 UI、完整门禁与安装/授权目标仍未完成。下一项从真实出生角色和预算回归缺口继续，不能以本批安全拒绝代表最终删除功能实现。
