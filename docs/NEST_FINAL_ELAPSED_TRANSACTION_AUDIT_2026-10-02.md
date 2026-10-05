# 原 Root 最终耗时与收尾事务审计 · 2026-10-02

此批已实装并通过下面的实际合同；Master、持续 Root 决策、全预算生命周期和整体功能仍未完成。真实数据库、CAS、资产、安装态、授权 URL 未操作；所有写入合同使用临时 SQLite。没有重置、批量替换、提交或推送。

## 原问题和最小作用域

旧共享 finisher 仅完成类采样最后耗时，其他 outcome 漏记；完成重放使用 now，可能继续收费；只有原 Root control 而没有 child 时也无法正常结束。旧 Source 在共享 finisher 后仍写 TerminalReduced，原内部检查不能覆盖最后出版副作用。Source 的 Root 注册实际已经在初始 SDK 前，此批不调整该次序，也不宣称新增了可信 Source 预算生产者。

捕获一个原事务 cutoff，所有 outcome 采同一原 elapsed；终态重放使用原 finished_at 且不收费、不生成事件。原 control 纯读 reader 不初始化、不采用 successor C；旧真实 worker 可用原金融身份，缺失原 owner 的历史拒绝，不回填。十维限制、原 origin 和当前 Native JSON 保留，elapsed 达到 hard 仍拒绝。超期原事实保存及 Limited 消费是后续独立工作，此批没有 clamp、扩 hard 或授新执行权。

私有 writer 从 BEGIN 前覆盖 Root、指令取消、费用和 Source 最后出版至原事务 commit。它核五个实际内置 emitter 定义，拒绝 TEMP 同名覆盖及未知触发器写入；保存完整旧 Native／协作事件含 rowid 的指纹，核新增事件的原 scope、实体和内容，拒绝 IGNORE 造成的缺失回执。Source 继续核完整 phase/runtime/gate/material 与精确 TerminalReduced，而不是用 SQL 定义替代结果证明。writer 在 Result 返回时清理 authorizer；此批没有证明 panic 期间的 RAII 清理。

## 实际红测与修复

- 接口先决和 16 项实际 finisher/Source 合同先运行：3/16，通过实际路径证明 13 个遗漏或副作用。记录 `/tmp/oviraptor-final-elapsed-red.log`。
- 首个修复编译失败是新泛型 closure 返回类型未明确，E0282/E0283；不是行为红测或通过。根加 `Result<T,String>` 后 16/16，5.14 秒，`*-green-type-fix.log`。
- 扩大 117 项实际合同：115/117，334.62 秒，`*-affected.log`。两项旧负向期待发生变化：未知终态触发器现在在写入前被 authorizer 拒绝，替代 C 现在不能结束旧 worker Root。没有放宽生产代码；精确拒绝错误及完整应用/物理行不变证明代替旧错误或成功期待。
- 修改两项负向期待后，final_elapsed/directive_closure/budget_clock/budget_journal 55/55，19.72 秒，`*-affected-contract-final.log`。扩大组里其他 Root/Source 合同已通过，未因无新 Source 改动重复耗时组。
- 严格 Clippy 首次发现 canonical JSON 的 cmp_owned。根改为显式序列化后比较原文本，保留字节一致门禁，未采用 Value 语义相等或 lint 抑制。最终实际 16 项加两项旧负向和精确退役 allowlist 共 19/19，5.91 秒，`*-canonical-final.log`。
- `cargo clippy --offline --locked --all-targets --all-features -j1 -- -D warnings` 退出 0，14.93 秒，`*-clippy-canonical-final.log`。导入器 39/39，3.88 秒，`*-importer-final.log`。Cargo 串行 nice 15、offline/locked、单编译作业；lib 合同单测试线程。

## 工作树保护及检查边界

17 个财务代码路径：8 已有、9 新增，无删除。9 个新文件最大 330 行。Root reader 两处根修正、clock/root 局部格式及两项旧负向期待均单独登记；不把代理草稿 SHA 当最终 SHA。14 个 Rust 叶格式检查通过；原 Source completion 180→190 行及两个既有测试文件保留原格式债，不扩大修改。原 Source 180 行的独立 fmt 检查同样失败，证据 `*-source-original-format.log`；未宣称全仓 fmt 通过。

此批与独立 UI 批串行合并登记，共 28 路径（14 已有、14 新增）。基线 947 路径 `f5e406f84a4c80348045bcb4bf721cff9c772bcadc0023f4cd2c7979c8951302`；最终实际 961 路径 `110ced35f72dfcb2178f5ef1a130673e6be515215598391405da0530ee0e2ad6`。933 个两批范围外原路径逐字节不变，HEAD 仍为 `59be3d86d25adda5b1f975759bf256ef3b94f32b`。此登记宇宙沿用源码/tools/resources/Cargo/配置，不含 build.rs、icons/capabilities/dist 或整个仓库。

原文本、既有 dirty 差异、阶段合并、最终逐文件差异、scope、格式记录和三文档原字节在 `/tmp/oviraptor-final-elapsed-*`；实际最终联合 SHA 在 `/tmp/oviraptor-final-elapsed-ui-code-snapshot.json`。较早 `*-code-snapshot.json` 是 UI/最后 canonical 修正前检查点，不是最终联合指纹。`git diff --check` 通过。UI 独立作用域与门禁见 `NEST_UI_PRESENTATION_AUDIT_2026-10-02.md`。

## 明确未完成和风险

- 全局 clock/limits/entries 所有 UNIQUE 碰撞保护尚待独立红测；Web model/assignment attempt 其他唯一键也不能据本 writer 宣称全部防护完成。
- 原 Root 超限/过期金融事实、跨 C 原金融结算和受限终态消费、Single finally 与真正暂停、fresh Source 原预算生产、dynamic grant/精确人工对账仍未完成。
- Mode/HMAC/真实 Root SDK 持续决策、真实角色执行/监督、聊天逐角色执行、SDK/Web helper 全路日志仍未完成；不把财务回执当角色执行证明。
- 旧事件全历史读取/指纹有线性成本，尚未做有界回执优化；毫秒 finished_at 的安装态消费者未验收。全量门禁、安装和授权 URL 仍在框架之后。
