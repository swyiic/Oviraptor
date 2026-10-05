# Root 原 SDK 回放凭证边界审计（2026-10-04）

状态：本项相关门禁完成；Master继续开发，没有整体功能或发布完成声明。

## 问题与数据作用域

仅仓库和独立临时SQLite/localhost真实SDK，原新Web creator、冻结Native计划、Root/C/财务合同/UUID父监督保持。原TickOnce在任何IMMEDIATE和请求/历史/预算核验之前调用create=true的invocation领取；同Root已付回放、503费用未知重入、原请求hash损坏重入，均会重新创建已丢原文件。OS inode仅是现有生命周期锁的物理退出证明，原调用元数据由DB核验；文件没有HMAC内容，不能把它描述为metadata凭据或全工具join。

首3项真正0/3，runtime1.69秒、compile24.81秒。四个后续首次断言之外的收费/行断言在红状态未执行，不声称已经全部红覆盖。新增首次硬预算1000拒绝与正常已付回放对照，首1/5、runtime2.91秒/compile23.76秒；首次不足同样补造文件，正常回放原本通过。删除的仅是各临时测试在实际provider返回之后的原root-decision-sdk lock，不删生产DB/CAS/资产。

## 最小实现

只改TickOnce领取顺序：原IMMEDIATE内检查现有Root/C/parent、原帧、原本地历史及原configured model；纯load原财务owner并核原C，使用既有require_idle_original核所有原dispatch/request/hash/财务归属/费用物理证明并仅probe原文件。已有历史所得guard持有到整个SDK、原费用/ModelLog、本地publication和纯本地回放返回。无dispatch/request历史时，在原预算观察/轮次、Tick begin及最后身份/帧/历史检查成功之后才claim首次owner，仍在COMMIT之前取得，提交后才能发SDK。失败回滚不补费用、不给旧历史重新派发权限，也不删除任何锁。

修后五项5/5，runtime2.89秒、compile19.21秒：缺失/损坏原请求拒绝，缺失文件不再创建，全部149应用表逐行保持，SDK均1；首次预算拒绝SDK0且全部行/不存在的file保持；正常回放replayed=true、原file创建时间、全应用行、SDK1/原consume1保持。

## 当前验证与保护

主批逐名239/239，runtime307.10秒/wall307.75秒，含实际在途Root/下一轮、取消、旧费用、退出与本地ReAct。第一严格all-target/all-featureClippy exit0/wall18.04秒，导入39/39 runtime3.07秒/wall21.48秒，exact退役1/1 exit0/wall1.45秒。

重新核验后补入41个当前bootstrap/changed Mapper/Reviewer/Investigator反馈、实时预算、paid聊天/SDK日志合同，它们不在旧239名单中。首40/41 runtime49.16秒；唯一篡改paid snapshot测试得到更早root_decision_lifetime_original_binding_conflict而非原root_tick_original_input_conflict，功能依旧拒绝、SDK1。仅更新该已有测试的精确错误码，原SDK1/全行断言不删。最后41/41 runtime47.36秒/wall68.69秒，随后严格Clippy exit0/wall9.44秒。最后一修改仅该诊断断言，不影响前239实现；两逐名集合无交集280，不宣称同一完整主库全量。现有六失败名单/tmp/oviraptor-specialist-replay-proof-unresolved-prior-names.json没有ignore、删除或代入绿色。

serial nice15/offline/locked/j1/testthreads1；每个Cargo收终态后才下一项，Cargo中不改Rust/resources。四代码路径3已有/1新，新测试116行；1274集合SHA42abe05a9c8d18ff13019153876f53063f8c8896ae911a97336e2c12c33bf283，1270原范围外精确摘要保持，HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b、diff--check0。原文本/先前未提交差异/四逐文件merge已审查，证据前缀/tmp/oviraptor-root-replay-proof-*。首test patch ddd2b9890da265e27a281f101ea19ca11850bc52694b1585c043973abd4de4f7；extra 08eec221677ef3614f5bbe04acd543956aae00e9a1996843842b4408d14e30cb；runtime d33285378463ed104b1183fe9fe0efdcdaec2b9ea242ac274ea3cb6e783d2df3；diagnostic a551cd8553c34277200ae14e901643f7c0d48c6fff43938a50c1d53612043ddb，均先完整patch阅读/SHA/check后apply，不整文件覆盖。

## 未完成与下一步

Root目前主要三个具体反馈frame对应Master child output/reviewer两个trigger类别，不宣称其六类已全接；本项不增加新的监督/通用ReAct/dynamic grant能力，也不把本地SDK stub输出当模型质量。Client浏览器影响、候选/独立Reviewer、其他角色写权、通用在途工具join/强杀对账/显式续跑、15角色/真正并行、聊天/逐路日志/整体UI、六旧失败均继续开发。paid Root/task的不可变财务证明阻止现有物理删除，必须另证明删除与保留范围，不能放宽或删除财务触发器冒充修复。最终完整门禁、安装App打开、授权匿名URL及真实模型仍未开始。InputParser既有自动审批拒绝不重试或换名绕过，完整拒绝原因不可见；Goal工具旧blocked，用户继续授权正在执行，不标complete。
