# 普通 Web 真新 Root 预算与财务创建审计 · 2026-10-02

本批两个独立红/修步骤已合入：v2生产者与同创建事务财务owner。Master、全Scope财务和整体功能仍未完成。本批仅隔离临时SQLite和localhost夹具；无真实库/CAS/资产、安装、URL或提交操作。

## 原问题与最小改动

实际普通publisher原来传None budget；虽有独立声明API和v2测试，生产任务仍无v2预算。新增producer仅从原私有VerifiedMode+Native plan、创建分支导出十维声明，同Root Mode writer的第五合法写入保存。执行槽3、实际race batch0独立，browser/write/upload0，不赋新能力。旧Mode-only重放不补v2；原Native plan/hash/projection逐字保留。

财务owner原本到Single prepare或Root claim才初始化，新Root四属性为0，runtime_open后来写原预算。真实原创建现在直接INSERT四属性；private NewlyInsertedNativeRoot借用仅在该IMMEDIATE事务授第一次财务创建。先Mode/v2，随后Single nullC或Multi原C1 UUID（到期不超过600秒/原Root截止），原control UUID、十limit和createdAt origin。无UPSERT、粗预算、child、费用、SDK或目标工作。

ModeRootWriter只安装一份authorizer，raw born initializer不覆盖setter。Mode-onlySingle16/Multi17，BothSingle17/Multi18准确total_changes。旧C/owner/limit/origin/model/HTTP/fine/coarse行含rowid和原Root/sidecars/event/projection均捕获，最后同证明复核。canonical builtin Root INSERT emitter及无TEMP shadow核验。IGNORE、业务/其他Root副作用、REPLACE和最后投影换C整体拒绝/回滚。

已有Mode缺owner不因zero-history、C1或真实接管C2获得control/claim；Single准入纯load_single；Multi/外层run在acquireC/reentry/Root/粗账本前纯load_original且验证原C。历史Mode adapter只cfg(test)，保留真creator/startup/原HMAC/workdir语义，用来产生原历史物件，不给执行授权。

## 实际红绿与错误记录

首次01patch aggregate上下文因新runtime include漂移未应用；误继续运行过滤得到0tests，`fresh-web-budget-red.log`明确不当红/绿。重基仅一行include后实际编译27.31秒、1/3红（0.50秒）：生产缺声明/IGNORE成功；旧Mode-only不回填原来已保护。02后实际相关36/36（16.35秒），`fresh-web-budget-green-affected.log`。

财务01同样仅aggregate窄重基，真实5合同0/5（1.09秒，compile27.24秒），`web-born-finance-red.log`：新四属性0、Single旧owner收编、core control C1收编、实际prepare C2仍成功、财务IGNORE被接受。core多case循环首次C1失败而未跑后续case，不能说所有C2/claim红已被单独观察；实际prepare确实重获C2。修后全部循环通过覆盖。

完整阅读sealed02 SHA922ecfde1fd3b97266d37950288a1bfd4c94404fad6f79dbad9c61d456aeb345后精确应用，03两测试hunks按已有格式重基：新Multi C1有原金融意义；旧Mode-only用明确历史creator。最后41/41（20.38秒，compile50.88秒），`web-born-finance-green-affected.log`，集合包括前36和新增5，不相加。

扩大83项实际82/83（100.27秒），`web-born-finance-original-regression.log`；唯一旧Singleprepare测试以旧无ownerRoot期待迟建control，现应拒。没有放回fallback：该正向迁到真实Singlecreator/publisher，创建前原plan确定、open前已有finance、open后financialrows不变，两次prepare完整全表只读，rename明确loads_original_creation_control。最后相关10/10（6.12秒，compile16.84秒），`web-born-finance-original-final.log`。未把83宣称最后全绿。

首次严格Clippy发现废弃initialize_control无生产调用，`web-born-finance-clippy-final.log`失败。原Single只读路径不再需要该入口，helper保留仅cfg(test)供原core合同，未allow(dead_code)；g2后续wrapper须读原owner而不得复活迟建授权。最后严格all-target/all-feature exit0（23.94秒），`web-born-finance-clippy-test-only-final.log`；独立import39/39（2.60秒）及退役字面量1/1（0.51秒）为最后cfg状态。没有前端改动，因此此批不重复UI套件。所有Cargo串行nice15/offline/locked/j1/testthreads1。

## 工作树保护与快照

17代码路径12已有/5新增，无删除。新文件最大274行；其他新增finance203、schema emitter50、budget producer76、budget test148。10限定Rust叶fmt检查0，旧大store/backend/Root文件保留格式，不全仓格式化。git diff --check0，所有已有文件实际增量hunks已读。

原983路径SHAf7c556f7e5c1c9cb8c331abcb2f08d9aa60743ffc14908aaac477aefc12274a3；budget阶段985 SHA5535045040c94d4373f5085954565948677a9cb7e4e6ac67f148ad83c4a01009；最终988 SHA87ec6e574923d64a7052c098953ea5cc02db3b9d08667afe688479de0c1b97ae，971范围外原路径不变，HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b不变。首financial seal15-path SHA7e3f2e8b...是cfg(test)之前，不当最终。

阶段before/dirtydiff/hunks分别保存 `/tmp/oviraptor-fresh-web-budget-*` 与 `/tmp/oviraptor-web-born-finance-*`，联合最初before/scope/snapshot在 `/tmp/oviraptor-web-created-budget-finance-*`。集合沿原src/src-tauri/src/resources/tools/Cargo/配置/exactallowlist，不含build.rs/docs/icons/capabilities/dist或全仓。三文档旧正文严格保留在 `web-created-budget-finance-docs-before.json`。

## 未完成与风险

独立live审计已证明generic RootOwner::initialize对无Mode旧Single仍能mint；registered native_model_transport可实际发SDK，外层Mode gate不能替代此API。下一批要真实SDK红后使生产initialize仅原owner读取；Web借已有fresh token，Source需私有FreshSourceRoot借用原创建TX，不用pristine bool；lower-level测试改显式历史fixture初始化，禁止生产cfg(test)fallback。

Source创建财务、其独立slot/batch producer仍未完成。Root real bootstrap/paid recovery/changed facts/全ReAct/utility/唯一Reducer、专职角色和BrowserBroker、Single finally/同Root暂停恢复/known-unsent/超期终态/dynamic/人工对账、聊天有序真实角色执行、SDK/Web所有路径日志继续缺失。模式tombstone/实际Native settled delete仍需独立合同。全库、前端、安装App/真实强杀和授权URL在框架完成之后，不因本批门禁宣称完成。Goal工具paused但用户继续授权有效；未标complete。
