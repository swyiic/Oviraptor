# 2026-10-04 原 Source 工具阶段 SDK 在途终态边界审计

实际原 Source creator/analyzer/frozen Root/C/UUID worker/Supervisor 下，请求已到 localhost provider 且阻塞，旧结束入口仍发布 terminal。首负向 0/1（runtime 0.90 秒/编译 35.83 秒）。仅在原 Source task、完整 round request/hash/财务/C/预算与历史核验后领取 source-round-sdk OS owner，原 gateway 保持至 SDK、原费用、ModelLog 和本轮本地工具返回；Root 原结束同一 SQLite 写锁验证所有原 Source round，按 child 去重，仅 probe 已有原 inode，guard 保持到事务退出。

首修 1/1（0.95 秒/编译 40.85 秒）；补实际 SDK 返回后允许关闭的双向边界 1/1（0.91 秒/编译 13.97 秒），原非墙钟费用、Source rounds 和终态回放保持。新增 5/5（7.29 秒/编译 30.45 秒）：第一轮在途和第一轮已付/第二轮在途均拒绝提前结束；实际取消在 provider 释放前返回；缺失或 foreign 锁无法替代且同原 request/预留重入不 CREATE；hash/request/worker 损坏全行保持；实际 assignment.finish 付费完成后允许 Root 关闭、费用和回执保持、禁止下一轮及重复 SDK。第二轮等待两份实际 provider 记录，避免把先前已付请求错当当前到达。

纯财务 PendingRound 可克隆而不携执行所有权；生产入口只走 start_for_transport，纯财务 start_authorized 仅测试。第一严格 Clippy 实际失败于旧财务 API 已无生产引用，逐引用确认后加 cfg(test)，未全局关闭警告、未放宽权限。初边界 patch SHA 校验一次不匹配，在写入之前拒绝；纠正为实际已审查内容 SHA 后才应用。没有 schema、Native JSON、授权、费用、退款或重发语义变化。

扩大 372 名首轮 371 过/1 失败（runtime 1410.81 秒/wall 1422.29 秒）；唯一失败为 source_reviewer_ci_projection_write_faults_roll_back_gate_without_replaying_model。独立同编译产物诊断实际仅 SDK 1/预期 7，在第一份付费事件写入处 not authorized（runtime 18.27 秒）。旧用例过早安装 terminal trigger；SQLite authorizer 预编译触发器写权限，即使 WHEN 为 false，仍会在第一份回执处拒绝。仅该用例把五种故障安装点迁到原实际七次 SDK 和审核 delivered 之后，保留 gate/审核/费用 7/0/终态回滚断言，生产权限不放宽；单项 1/1（29.89 秒/编译 41.09 秒）。结果逐名覆盖 372，分开记录首轮 371+修后 1，不宣称一次全绿。该已有测试文件 545 行是结构债，新运行时文件和新用例均低于 400 行。

迁移后严格 Clippy exit 0（10.47 秒）；导入 39/39（wall 36.66 秒），exact retirement 1/1（wall 1.24 秒）。首轮严格已 0（13.83 秒），原失败日志和诊断全部保留。测试时机迁移未改变生产代码，未重复整组 24 分钟运行，后门禁针对当前完整编译产物。

8 路径 6 已有/2 新，1227 集合 SHA 00bc29fda22365d91c0e095f12337fe5688f630545dee61b157b40f6fb7a9850，1219 原范围外文件保持；HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b，diff --check 0。逐文件原文、既有 Git 差异、合并差异、patch 校验与全部日志在 /tmp/oviraptor-source-round-inflight-closure-*。仅 repo/临时 SQLite/localhost，未操作真实 DB/CAS/资产/安装/授权 URL/提交；前端无变更，不借旧 UI/build 充当本批验证。

Master 未完成：本批是 Source 工具阶段实际 SDK/本轮本地处理边界，独立 fixture 直接进入 schema1 Source tools，不冒充完整七 SDK Native Source 流程、真实模型能力或全部 worker join。Source 已保存 finish 的其他恢复入口、Single/HTTP/工具/进程全退出、原 paused/过期/换 C/重启、普通 no-call/typed unsent 收尾、动态十维 grant/精确 provider 对账/显式续跑、六触发/15角色/General ReAct/Broker/真实并发、聊天/逐路日志/整体 UI/其余旧 E2E/正常 paid 删除，最后完整门禁/安装 app 打开/授权 URL/模型质量仍待完成。下一步以真实创建 Single 复现原财务收尾替换 caller authorizer。InputParser 原自动审批拒绝未重试或绕过，完整原因不可见，不猜测；Goal 工具旧 blocked，用户已继续授权，持续开发未标 complete。
