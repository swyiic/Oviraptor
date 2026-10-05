# 基础预算全唯一键不变性审计 · 2026-10-02

本窄批已实际红测、修复及相关门禁；Master 和全预算生命周期仍未完成。仅临时 SQLite 测试，没有真实数据库/CAS/资产、模型/目标请求、安装或提交操作。

原 clock/limits/entries 只有 UPDATE/DELETE 保护；recursive_triggers=OFF 时，INSERT OR REPLACE 可先删除冲突旧行而不触发删除保护。作用域包括 clock 的 root_run_id、limits 的完整(root_run_id,dimension)、entries 的 entry_id 及完整(root_run_id,idempotency_key)，一次冲突两旧行也需拒绝。

六项合同调用真实 scheduler，原 Root.created_at 在首次 freeze 前设置，由原 worker 和实际 wall Reserve/Consume 创建原费用，逐表保留全应用快照、完整行及 rowid。不用假 consume/worker 扩权证明。

先仅增加测试，实际 1/6，0.75 秒：五项失败证明原时钟/限制/费用替换、outer IGNORE 静默及启动增量仍没有保护；原调度与精确费用重放正向通过。完整日志 `/tmp/oviraptor-budget-unique-red.log`，不是静态 SQLite 推断。

最小修复是 schema.rs 一次 include_str 和 18 行新 SQL，三条 BEFORE INSERT 在碰撞解析前 RAISE(ABORT)。所有四类唯一键包含相同值、单/双碰撞均拒绝。原 producer 重放先 SELECT 全字段验证，不再 INSERT，因此保留正常原合同。没有 DROP 旧保护、迁移/重写行、补限额/身份、修改 Native JSON 或创建新授权。

实际预算唯一键、原 finisher、clock 和 journal 相关 40/40，16.49 秒，`/tmp/oviraptor-budget-unique-green-affected.log`；其中启动增量用真实 db::initialize 在已有临时 Native 历史安装保护并验证完整物理行不变。测试里的 DROP TRIGGER 仅删除本测试拥有的临时新增保护以模拟升级，真实库未打开。

严格 all-targets/all-features Clippy 退出 0，11.64 秒；独立导入器 39/39，2.47 秒；精确退役 allowlist 1/1，0.54 秒。记录 `*-clippy-final.log`、`*-importer-final.log`、`*-literal-final.log`。Cargo 串行 nice15/offline/locked/-j1，合同单线程。两相关 Rust 叶 fmt 和 git diff --check 通过。此无前端修改，未重复 UI 组。

四代码路径：两已有、两新增，无删除。新测试 307 行、SQL18行；原 aggregate include 和 schema 保留其其他 dirty 字节。基线 961 路径 `110ced35f72dfcb2178f5ef1a130673e6be515215598391405da0530ee0e2ad6`；最终 963 路径 `5bcba44b0b76b5e5542457954500507103c7fbbeb255ee35faf0ce2dd3e74e6e`，959 个范围外原路径不变。HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 保持。登记宇宙沿用源码/tools/resources/Cargo/配置，不含 build.rs/icons/capabilities/dist 或整个仓库。

原文/dirty 差异/最终差异/scope/snapshot/三文档原字节保存在 `/tmp/oviraptor-budget-unique-*`。全历史 NULL entry_id、恶意替换同名保护或整体 schema attestation 不由本批解决；不清理历史或将全新行的 INSERT 当执行授权。

Web model journal 三类唯一键、original assignment attempt 六类唯一键的独立负向保护仍待后批。超期/过期原耗时事实与受限终态、Single finally/真正暂停、fresh Source 预算生产、dynamic/人工对账、Mode/真实持续 Root 决策/角色/聊天/全路日志及最终全量/安装/URL仍未完成。
