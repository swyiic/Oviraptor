# 源码多引擎接受来源与候选分组审计

状态：两类真实来源丢失已修复，定向验收和包含最新修改的完整 Rust 回归通过，终态见下方验证记录。本文不是源码生产多智能体编排、独立 Reviewer 或整个 Master Plan 已完成的声明。

## 1. 实际缺陷与修复

### 1.1 相同字节不能归到第一个引擎

原 `AnalysisResults::capture` 按 artifact hash 查找第一个分析器。两个分析器输出相同字节时会丢掉另一引擎；调换输入顺序会改变来源。仅按 hash 判断接受状态，还会把同 hash、但本次没有实际导入的引擎误认为已接受。

现在先核对本次实际导入 bundle 的 `{engine}.sarif` 文件名及内容摘要，再按 `(engine, hash)` 记录接受状态。多个实际接受文件对应相同记录时保留每一个来源；未导入的同 hash 引擎仍保留 `analysis_result_artifact_not_accepted:<engine>` 缺口。不会根据历史 membership 或单独的 hash 补写来源。

### 1.2 不同报告合并不能丢掉贡献来源

原导入事务只为 merged canonical record 的主 provenance 返回回执。不同字节的两个 SARIF 合并为一个逻辑候选时，另一报告的贡献来源丢失。

现在 `CanonicalRecord` 在本次 parse/merge 内保留 `contributing_artifacts` 集合；真实提交为每个 revision/artifact pair 返回回执。多个来源共享同一个真实 revision 与 envelope，不增加语义候选数。

该字段 `serde(skip, default)`，只在内存中参与本次提交，不进入历史 envelope、revision hash 或签名。旧 JSON 不能携带此字段来增加执行权限或伪造贡献来源。

## 2. 读取合同

- 保持 `AnalysisResults` / `AcceptedResult` 的持久 v1 JSON 结构；旧单来源回执原样读取，不推断第二来源，不改变原 digest。
- `AcceptedCandidate` 只是读取投影：按 **logical key + revision hash** 分组，保留全部真实 `acceptedSources`。
- 同组必须指向相同真实 revision id/envelope；不一致拒绝 `analysis_result_candidate_revision_conflict`。
- 同一 logical key 的不同 revision 不合并。需要唯一结果的 get 接口继续拒绝歧义，不任意选取一个修订。
- `analyzer.list_results` 按唯一候选修订分页、计数；两个来源不造成重复候选或虚假 truncated。
- engine filter 检查全部实际来源；旧单值 `engine` 兼容字段选择匹配的来源，同时返回完整 `acceptedSources`。
- `analyzer.get_result`、灰盒 `sourceClaims`、源码专家 `candidateRecords` 使用同一分组语义。
- assignment 的 `candidateReceipts` 仍冻结全部底层来源，不把展示去重当作权限或证据删减。

## 3. 修改位置

- `src-tauri/src/artifact_import/canonical.rs`：本次 parse/merge 的内存贡献集合。
- `src-tauri/src/artifact_import/service.rs`：真实提交的多来源回执。
- `src-tauri/src/native_pipeline/results.rs`：文件名/hash 联合接受、稳定来源排序、候选修订分组。
- `src-tauri/src/native_pipeline/tools.rs`：list/get 的分组、过滤与来源展示。
- `src-tauri/src/commands/native_source_scan.rs`：灰盒 claims 保留全部来源。
- `src-tauri/src/agent_runtime/multi_agent/source.rs`：专家初评输入保留全部来源。

未修改退役守卫的审核 allowlist，未减弱授权、独立复核或 CI 门禁。

## 4. 新增七项回归

`commands/tests_source_broker_scope.rs`：

1. `source_result_same_bytes_keep_both_engines_without_duplicate_candidates`：两个真实 analyzer fixture 输出相同 bytes，实际 importer 接受两来源；list/get/claims 只计一个候选，双方 engine filter 有效，limit=1 不误报截断。
2. `source_result_same_hash_never_accepts_an_unimported_engine`：仅真实重新导入 semgrep，把同 hash codeql 排在前面也不授予接受状态。
3. `source_result_same_bytes_attribution_does_not_depend_on_artifact_order`：实际重新导入后反转 artifact 列表，接受记录及缺口不变。
4. `source_result_legacy_single_origin_is_not_inferred_from_matching_hashes`：旧单来源 v1 回执不补来源，序列化和 digest 不变。
5. `source_result_distinct_reports_keep_every_merged_candidate_origin`：不同 SARIF bytes 合并一候选，两个实际来源共享真实 revision/envelope；envelope 不含内存贡献字段。
6. `source_result_grouping_keeps_different_revisions_separate`：通过真实二次导入产生同 key 不同 revision，读取投影保留两修订；旧冻结 attempt 仍只含原修订，补导不替换本轮结果。

`commands/tests_source_specialists.rs`：

7. `source_specialists_group_shared_revision_but_keep_both_accepted_origins`：RepoMapper/SourceAnalyst 各获得一个候选及两个来源，冻结两条底层回执；真实 scheduler、模型回执持久化、完成与 acknowledged mailbox 各一次，目标请求和工具授权仍为零。该项不是实际远端模型测试，不冒充生产自动编排；已有 localhost transport 回归另行保留。

重新导入 helper 复制本次真实 analyzer-output 的 SARIF 并调用实际 importer，不手工伪造 `committed_records`。

## 5. 失败与验证记录

### 保留的红测

- session **85072 exit 101**：前三个 same-hash 回归 **0 passed / 3 failed**，复现丢来源、顺序依赖及误接受。日志 `/tmp/oviraptor-source-provenance-red.log`。
- session **36930 exit 101**：不同内容报告合并回归 **0 passed / 1 failed**。日志 `/tmp/oviraptor-source-provenance-distinct-red.log`。
- session **26997 exit 101**：源码相关 **124 passed / 0 failed**；后续误用不存在的 `--test artifact_import` 导致命令失败，retirement/Clippy 未执行。这是验证入口错误，不是源码测试失败；已用独立导入器二进制入口纠正。日志 `/tmp/oviraptor-source-provenance-source-v2.log`、`import.log`。

### 最新定向验收

session **49429 已退出 0**：

- `cargo test --manifest-path src-tauri/Cargo.toml --lib source_result_ -- --test-threads=1`：**15 passed / 0 failed**，含最新不同修订分组回归。
- `cargo test --manifest-path src-tauri/Cargo.toml --features import-tools --bin import-existing-results -- --test-threads=1`：**30 passed / 0 failed**。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib retirement_ -- --test-threads=1`：**4 passed / 0 failed**。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`：通过。

日志前缀 `/tmp/oviraptor-source-provenance-`：`focused-v3.log`、`import-v2.log`、`retirement-v2.log`、`clippy-v2.log`。

已有界面/build/localhost browser session **19964 exit 0**：界面 **147 passed**，构建及 localhost 浏览器回环通过。主 JS **830.15 kB** 拆包警告保留。它在随后 importer lineage 和最后分组回归修改之前运行，且没有 UI 源码变更；不能代替当前完整 Rust 验收。对应 `ui.log`、`build.log`、`browser.log`。

最新界面 session **57602 exit 0**：再次执行七组实际组件测试，**147 passed / 0 failed**；日志 `/tmp/oviraptor-source-provenance-ui-final.log`。最新 localhost 浏览器 session **84015 exit 0**：`passed=true`，匿名和对照采集均 complete、观测请求 8、身份隔离通过，包含跨域子资源和重定向阻断断言；日志 `/tmp/oviraptor-source-provenance-browser-final.log`。没有访问外部目标。

完整 Rust 回归结束后，构建 session **38194 exit 0**：`npm run build`（TypeScript 检查及 Vite）通过，主 JS **830.15 kB** 拆包警告仍保留，未通过提高阈值隐藏。日志 `/tmp/oviraptor-source-provenance-build-final.log`。

### 当前完整回归

session **75538 已退出 0**：完整 `cargo test --all-targets --all-features -- --test-threads=1` 主库 **1110 passed / 0 failed**（540.30 秒）、历史导入器 **30 passed / 0 failed**；随后严格 Clippy、fmt 和 `git diff --check` 均通过。日志 `/tmp/oviraptor-source-provenance-full.log`、`clippy-full.log`、`fmt-full.log`、`diff-full.log`（后三项同前缀）。运行期间未更改 Rust 源码或构建输入。

前一发布合同的 **1103/30** 全绿只证明此前版本，不能用来证明本次七项新增修改。前序合同见 `NEST_SOURCE_RUNTIME_PUBLICATION_AUDIT.md`；结果接受基础合同见 `NEST_SOURCE_RESULT_RECEIPT_AUDIT.md`。

## 6. 必须继续完成

生产 `run_native_source_scan` 目前只验证源码模型 runtime 合同，仍未自动派发 Source Coordinator、RepoMapper、SourceAnalyst 和独立 Reviewer。实际模型美元计价、派发前费用预留与未知费用对账、源码工具权限和持久 finish、当前 candidate/revision 的 CI review 资格仍需实现。不得因本次来源修复移除 `source_review_not_completed`，也不得把两引擎报告或两份专家初评当作独立复核通过。

沙箱供应、知识/skills 治理、资产汇总及安装包/授权环境端到端仍按 Master Plan 验收。本轮未部署、未访问用户公网 URL、未启用 Host Agent。
