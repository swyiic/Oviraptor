# 2026-10-04 工作台用量与最近任务可读性审计

真实 WorkbenchOverview SFC 以 Vue SSR/共享实际状态labels/作用域CSS编译，8条合成记录，仅localhost静态预览，无IPC/DB/真实模型。修改前1280视口、460侧栏的实际CSS：两列222px，任务行白底rgb255、元信息9px，用量10px/低对比度。以现有组件及数据证明问题后只改3路径：当前已加载任务用量明确范围，去掉重复总计；任务标题/既有中文状态/原模型和三项用量分层，未命名与未知状态安全回退；主题背景/文字/11px元信息/16px总用量、可用宽度自动布局、键盘focus。原scans/模式过滤/顺序/最多6项/原emit对象/用量求和与格式不变，不改财务、执行授权、进度或完成判定。

首实际组件测试52/52与TS/Vite构建通过；读取真实Vite产物证明旧父作用域CSS随后覆盖新用量dt至10px，修新概览选择器作用域后再次TS/Vite通过（Vite2.29秒）。最后actual作用域SSR并按真实child-before-parent顺序核验：1280深色460侧栏为一列460px，任务行rgb24、标题rgb242、元信息和用量11px；1280宽960面板实际2列471px（另一个auto-fit零轨），无横向溢出；600浅色一列460px/scroll600，360浅色一列304px/scroll360，完整覆盖缺口状态换行可见。截图和render脚本/tmp/oviraptor-workbench-overview-visual保留，宽最终codex-after-wide.jpg、窄mist-after-360.jpg；初未编译父deep样式的预览不作为最终证据，最终实际scope已复核。无假typing、假完成或Token作为任务进度。

52相关组件测试含模式隔离、全加载汇总、覆盖缺口不假成功、未知状态HTML转义与未命名回退。最终仅CSS增量不重复未变化组件测试；未改Rust/Importer不借用旧全门禁。3路径2已有/1新，1223集合SHA7531a5ca8d67175d3d5443ab32a771e0bc118cc35805567040e6278a664f2487，1220原范围外字节保持，HEAD59be3d86d25adda5b1f975759bf256ef3b94f32b，diff --check0。before/prior Git差异、两个patch/check/apply、合并差异/codeSnapshot及测试/build日志/tmp/oviraptor-overview-reading-ui前缀保存。

该批只完成工作台概览可读性，不能当作完整UI、真实模型智能、Tauri IPC或安装态验收。Master仍待Single/Web/HTTP/工具/进程全退出、原恢复/预算对账/动态grant、全触发/角色/真实协作、聊天/逐路日志/其余UI/九旧E2E/paid删除，最后完整门禁/安装app打开/授权URL。仅repo/localhost合成组件，无真实DB/CAS/资产/安装/URL/提交；InputParser原自动审批拒绝未绕过，完整原因不可见，Goal工具旧blocked，持续工作未标complete。
