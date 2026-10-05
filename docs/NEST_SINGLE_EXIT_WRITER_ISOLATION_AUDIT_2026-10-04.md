# 2026-10-04 Single 原财务退出写入连接隔离审计

旧 writer 声称 private connection，实际在 caller 连接安装 authorizer 并在成功/失败后清为 None。首测试编译失败于 NativeModelRoundFailure 无 Debug，仅测试断言改为明确 panic，不修改生产类型；真正运行时负向 0/1（0.44 秒/编译 32.41 秒）：实际普通 URL creator/startup/HMAC/frozen Single/原 Root 及实际 localhost SDK 成功、已付 60 tokens/1 request 后，收尾清除 caller 的拒绝 projects UPDATE 原钩子，业务写入被错误允许。未用财务测试 issuer，未请求目标 URL。

最小改为 READ_WRITE、不 CREATE 的独立连接，10 秒 busy timeout、foreign_keys ON/synchronous FULL；原 authorizer 白名单不变，保持到独立事务 commit/drop，原调用者钩子和事务不触碰。原 caller 未提交事务仍精确拒绝，不提交或回滚 caller；内存 DB 无路径拒绝，不创建文件、补原 owner、grant 或新事实。仅原费用表和 Single exit receipt 写权，故障 IGNORE/ABORT/business collateral 全行保持。

新 4/4（1.92 秒/编译 20.98 秒）：实际 SDK 付费后收尾、原非墙钟费用不变、同回执只读回放、不可续跑；失败收尾原 hook 与所有物理行保持；caller 事务及未提交行保持；内存数据库拒绝且旧行保持。最终逐名 96/96（runtime 65.11 秒/wall 65.71 秒），严格 Clippy exit 0（15.17 秒）、导入 39/39（wall 25.26 秒）、exact retirement 1/1（wall 1.26 秒）。结果来自当前编译产物，不以旧 UI 或全套验收替代。

3 路径 2 已有/1 新，1228 集合 SHA 568e371ed8998507f06738d6739b8cca9000925be4e9ca66963a9adc8c37866b，1225 原范围外文件保持；HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b，diff --check 0。逐文件既有原文/Git 差异/合并差异、patch SHA 和失败/成功日志在 /tmp/oviraptor-single-exit-writer-isolation-*。仅 repo/临时 SQLite/localhost，无真实 DB/CAS/资产/安装/授权 URL/提交；未改变 schema、Native JSON、原费用/退款/重发/续跑授权。

Master 未完成：本批只证明 Single 财务 writer 连接隔离，未证明 SDK 在途不可提前关闭、HTTP/工具/进程完整退出和 join。下一步用原 Single 实际 SDK 阻塞复现提前财务退出，再继续 Source 其他入口、恢复/动态预算/精确对账/续跑、六触发/15角色/General ReAct/Broker/真实并发、聊天闭环/逐路日志/整体 UI/其余旧 E2E/正常 paid 删除，最后完整门禁/安装 app 打开/授权 URL/模型质量。InputParser 原自动审批拒绝保持，完整原因不可见；Goal 工具旧 blocked，用户已继续授权，持续开发未标 complete。
