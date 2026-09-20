# 交给 Qwen 的执行提示

请在 `/Users/swyiic/Desktop/Rust/Oviraptor` 中完成原生 Agent Runtime 可靠性整改。

开始前必须完整阅读，不能只读取摘要：

1. `docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md`
2. `docs/NATIVE_AGENT_EXECUTOR_TASK.md`
3. `docs/NATIVE_AGENT_RUNTIME_REMEDIATION_REQUIREMENTS.md`

第三份文档是本轮的强制验收合同。你必须严格按其中第 15 节顺序实施，并满足第 18 节完成定义。

工作要求：

- 先检查当前代码和现有测试，不要根据旧聊天记录猜测代码状态。
- 先添加能复现缺陷的失败测试，再修改实现。
- 不得使用 `git reset --hard`、`git checkout -- .`、`git clean -fd` 或覆盖用户未提交修改。
- 不得复制第二套状态机、终态聚合器、执行计划或重试入口。
- 不得通过放宽哈希/attempt 校验、扩大授权域名、删除证据字段等方式让测试表面通过。
- 续跑必须在新 attempt 中显式继承父 attempt，不能静默转成 fresh。
- frozen execution plan 必须完整继承，不能只继承 backend。
- 浏览器 entry、CDP 捕获请求和重定向必须走统一范围判定。
- 原始 HTTP 证据和模型可见结果必须分离，未脱敏秘密不能进入模型历史。
- `roleId`、`tenantId`、`departmentId`、`permissionId` 必须作为重要权限差异。
- coverage 必须绑定真实 evidence/request/tool invocation id，不能相信模型填写的 family。
- confirmed finding 必须绑定两条不同且真实执行过的 request id 及响应差异 artifact。
- checkpoint v1 必须迁移或明确报不兼容，不能被当成不存在后自动重跑。
- 工具参数必须执行完整 JSON Schema 校验。

不要在完成全部整改前更新版本号或发布说明。每完成一个阶段运行相应测试，最后执行：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build
git diff --check
```

还必须运行整改任务书第 14 节规定的本地 mock 端到端用例。禁止以真实生产网站代替自动化测试。

最终回复必须严格按照整改任务书第 17 节给出逐项结果。任何未完成内容必须明确写“未完成”，不得用“已优化”“基本支持”“后续可完善”替代。

