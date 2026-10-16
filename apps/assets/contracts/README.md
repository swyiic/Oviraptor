# Assets 完整业务迁移合同

盘点基线 `5642317df4206a4d277f4d53b433de53e1a9d488`，旧桌面 `294` 个注册 IPC 中，本模块三个 facade 共 `102` 次调用、`84` 个去重合同。另 `startup_status` / `test_fofa_api` 是两个相关边界，共 86 个相关入口。这个数量不是迁移完成率。

当前独立应用的库存三个工具和 unavailable 的快照入口不代表旧 Assets 已迁移。旧 Vue/Tauri 仍是完整兼容路径；新页面将复用 Assets owner 的成熟视图，经应用自己的有限 API port 连接真实 native owner。Host/common 不保存业务规则或资产表。

## 迁移批次与验收

1. 合同台账与构建边界：精确列出旧 IPC/DTO/owner，独立应用声明 Vue/Vite 构建依赖，包内带静态产物，运行时不依赖旧桌面或 Host 业务。
2. 库存、目标、规则与导出：保留完整 AssetQuery/AssetPage/Asset/summary/memberTotal/extra；筛选与响应代表锚、人工状态、回收/恢复、已看标记、预览令牌与导出使用原 Rust owner。真实失败、unknown、stale 不能伪装为空库存。
3. 采集任务：full/collect/reprobe、配置只读元数据、targets、同 runId resume/cancel、日志/变化、collectionSummary/retention/历史探测语义；任务属于应用进程，关闭页面不取消任务。
4. 归属与暴露面：21 个 ownership 合同与 40 个 exposure 合同逐项接线，保留所有预览、版本、operationId、证据、复测与手工授权区别。共享目录/旧全局规则仅人工管理 action；不进入模型项目工具目录。
5. 真实安装包验收：独立 build/package/process、原生 owner 同一 DTO 回归、真实 sandbox 页面操作，以及私有 DB 的 Host→native project 映射和跨应用显式交接。没有执行过的能力不能记为通过。

## 必须保留的边界

- `startup_status` 会将 queued/running/cancel_requested 标为 interrupted；不能成为页面轮询的只读状态。
- `dashboard_stats` 是 Shell 跨业务聚合；项目生命周期、运行配置以及 FOFA 账户测试不因被页面消费而成为资产库存查询。
- 全局 add_content_rule/get_asset_rule_catalog 与共享组织目录不等于当前项目库存写权限；模型工具、人工 operator action 分开。
- 人工有效性、归属事实、显式授权和扫描授权互不等价。归属回滚不恢复授权。
- 源观察时间、接收时间与 probe_checked_at 分开；未知不补当前时间，skipped 不当已验证。
- 原 summary 忽略 decisionView，filtered total 与 display groups/raw members 是不同计数；responseFingerprint 必须连同代表资产 ID 和项目精确展开。
- Host projectId 与 native projectId 通过可信映射，不假定私有 DB ID 一致。资产→扫描需精确目标与来源 identity/hash/version 的移交 receipt。
- 旧文件路径是 native 内部合同；页面取回导出应使用受控 artifact 引用，不获得任意文件读写。
- sandbox 页面不能访问 Host token/HTTP；opaque-origin storage 与 confirm/prompt 使用应用内端口，不放宽 iframe 权限。

## 完整旧 IPC 合同表

| IPC | 实际 Rust owner | 操作 | 返回 DTO | 主要调用 |
|---|---|---|---|---|
| `acknowledge_interrupted_run` | `commands::assets` | 写入/执行/生成文件 | `void` | `assetApi.acknowledgeInterruptedRun` |
| `import_targets` | `commands::assets` | 写入/执行/生成文件 | `number` | `assetApi.importTargets` |
| `list_targets` | `commands::assets` | 读取/预览 | `Target[]` | `assetApi.listTargets` |
| `remove_target` | `commands::assets` | 写入/执行/生成文件 | `void` | `assetApi.removeTarget` |
| `list_assets` | `commands::assets` | 读取/预览 | `AssetPage` | `assetApi.listAssets` |
| `mark_asset_entry_opened` | `commands::assets` | 写入/执行/生成文件 | `string` | `assetApi.markAssetEntryOpened` |
| `add_content_rule` | `commands::assets` | 写入/执行/生成文件 | `ContentRuleApplyResult` | `assetApi.addContentRule` |
| `preview_page_rule` | `commands::page_isolation` | 读取/预览 | `PageIsolationPreview` | `assetApi.previewPageRule` |
| `apply_page_rule` | `commands::page_isolation` | 写入/执行/生成文件 | `{ ruleId: number; isolated: number; protected: number; backupPath: string }` | `assetApi.applyPageRule` |
| `list_page_rules` | `commands::page_isolation` | 读取/预览 | `PageIsolationRule[]` | `assetApi.listPageRules` |
| `get_asset_rule_catalog` | `commands::asset_rule_catalog` | 读取/预览 | `AssetRuleCatalog` | `assetApi.getAssetRuleCatalog` |
| `disable_page_rule` | `commands::page_isolation` | 写入/执行/生成文件 | `{ restored: number; retained: number; backupPath: string }` | `assetApi.disablePageRule` |
| `update_decision` | `commands::assets` | 写入/执行/生成文件 | `void` | `assetApi.updateDecision` |
| `update_asset_decisions` | `commands::assets` | 写入/执行/生成文件 | `number` | `assetApi.updateAssetDecisions` |
| `get_asset_ownership_profile` | `commands::ownership` | 读取/预览 | `AssetOwnershipProfile` | `assetApi.getAssetOwnershipProfile` |
| `save_asset_ownership_profile` | `commands::ownership` | 写入/执行/生成文件 | `AssetOwnershipProfile` | `assetApi.saveAssetOwnershipProfile` |
| `get_asset_ownership_summary` | `commands::ownership` | 读取/预览 | `AssetOwnershipSummary` | `assetApi.getAssetOwnershipSummary` |
| `assess_asset_ownership` | `commands::ownership` | 写入/执行/生成文件 | `AssetOwnershipAssessmentResult` | `assetApi.assessAssetOwnership` |
| `update_asset_ownership` | `commands::ownership` | 写入/执行/生成文件 | `number` | `assetApi.updateAssetOwnership` |
| `resume_exposure_scan` | `commands::exposure` | 写入/执行/生成文件 | `number` | `exposureApi.resumeExposureScan` |
| `page_exposure_pending_discoveries` | `commands::exposure` | 读取/预览 | `ExposurePage<ExposurePending>` | `exposureApi.pageExposurePendingDiscoveries` |
| `read_exposure_evidence` | `commands::exposure` | 读取/预览 | `ExposureEvidence` | `exposureApi.readExposureEvidence` |
| `exposure_summary` | `commands::exposure` | 读取/预览 | `ExposureSummary` | `exposureApi.exposureSummary` |
| `list_exposure_findings` | `commands::exposure` | 读取/预览 | `ExposureFinding[]` | `exposureApi.listExposureFindings` |
| `save_exposure_finding` | `commands::exposure` | 写入/执行/生成文件 | `number` | `exposureApi.saveExposureFinding` |
| `start_exposure_scan` | `commands::exposure` | 写入/执行/生成文件 | `number` | `exposureApi.startExposureScan` |
| `list_exposure_runs` | `commands::exposure` | 读取/预览 | `ExposureRun[]` | `exposureApi.listExposureRuns` |
| `list_exposure_source_results` | `commands::exposure` | 读取/预览 | `ExposureSourceResult[]` | `exposureApi.listExposureSourceResults` |
| `cancel_exposure_scan` | `commands::exposure` | 写入/执行/生成文件 | `void` | `exposureApi.cancelExposureScan` |
| `export_exposure_findings` | `commands::exposure` | 写入/执行/生成文件 | `{ path: string; rows: number }` | `exposureApi.exportExposureFindings` |
| `reclassify_exposure_finding` | `commands::exposure` | 写入/执行/生成文件 | `unknown` | `exposureApi.reclassifyExposureFinding` |
| `exposure_scope_summary` | `commands::exposure` | 读取/预览 | `ExposureScopeSummary` | `exposureApi.exposureScopeSummary` |
| `page_exposure_findings` | `commands::exposure` | 读取/预览 | `ExposurePage<ExposureFinding> & { dispositions?: Record<string, unknown> }` | `exposureApi.pageExposureFindings` |
| `page_exposure_runs` | `commands::exposure` | 读取/预览 | `ExposurePage<ExposureRun>` | `exposureApi.pageExposureRuns` |
| `list_exposure_source_results_for_project` | `commands::exposure` | 读取/预览 | `ExposureSourceResult[]` | `exposureApi.listExposureSourceResultsForProject` |
| `read_exposure_finding` | `commands::exposure` | 读取/预览 | `ExposureFinding` | `exposureApi.readExposureFinding` |
| `exposure_known_limits` | `commands::exposure` | 读取/预览 | `unknown[]` | `exposureApi.exposureKnownLimits` |
| `soft_delete_assets` | `commands::assets` | 写入/执行/生成文件 | `void` | `assetApi.softDeleteAssets` |
| `soft_delete_asset_selections` | `commands::assets` | 写入/执行/生成文件 | `number` | `assetApi.softDeleteAssetSelections` |
| `list_runs` | `commands::assets` | 读取/预览 | `JobRun[]` | `assetApi.listRuns` |
| `list_logs` | `commands::assets` | 读取/预览 | `LogEntry[]` | `assetApi.listLogs` |
| `list_asset_events` | `commands::assets` | 读取/预览 | `AssetEvent[]` | `assetApi.listEvents` |
| `start_job` | `jobs` | 写入/执行/生成文件 | `number` | `assetApi.startJob` |
| `resume_job` | `jobs` | 写入/执行/生成文件 | `number` | `assetApi.resumeJob` |
| `cancel_job` | `jobs` | 写入/执行/生成文件 | `void` | `assetApi.cancelJob` |
| `export_assets` | `commands::asset_export` | 写入/执行/生成文件 | `{ path: string; rows: number }` | `assetApi.exportAssets` |
| `get_project_ownership_bootstrap` | `commands::ownership` | 读取/预览 | `ProjectOwnershipBootstrap` | `ownershipDirectoryApi.getProjectOwnershipBootstrap` |
| `preview_project_ownership_suggestions` | `commands::ownership` | 读取/预览 | `OwnershipSuggestionPreview` | `ownershipDirectoryApi.previewProjectOwnershipSuggestions` |
| `save_project_ownership_directory` | `commands::ownership` | 写入/执行/生成文件 | `ProjectOwnershipDirectorySaveResult` | `ownershipDirectoryApi.saveProjectOwnershipDirectory` |
| `start_ownership_analysis` | `commands::ownership` | 写入/执行/生成文件 | `OwnershipAnalysisRun` | `ownershipReviewApi.startOwnershipAnalysis` |
| `read_ownership_analysis` | `commands::ownership` | 读取/预览 | `OwnershipAnalysisOverview` | `ownershipReviewApi.readOwnershipAnalysis` |
| `cancel_ownership_analysis` | `commands::ownership` | 写入/执行/生成文件 | `OwnershipAnalysisRun` | `ownershipReviewApi.cancelOwnershipAnalysis` |
| `page_ownership_groups` | `commands::ownership` | 读取/预览 | `OwnershipGroupPage` | `ownershipReviewApi.pageOwnershipGroups` |
| `page_ownership_group_members` | `commands::ownership` | 读取/预览 | `OwnershipMemberPage` | `ownershipReviewApi.pageOwnershipGroupMembers` |
| `preview_ownership_group_decision` | `commands::ownership` | 读取/预览 | `OwnershipGroupDecisionPreview` | `ownershipReviewApi.previewOwnershipGroupDecision` |
| `apply_ownership_group_decision` | `commands::ownership` | 写入/执行/生成文件 | `OwnershipGroupDecisionResult` | `ownershipReviewApi.applyOwnershipGroupDecision` |
| `read_ownership_operation` | `commands::ownership` | 读取/预览 | `OwnershipOperationView` | `ownershipReviewApi.readOwnershipOperation` |
| `preview_ownership_group_revert` | `commands::ownership` | 读取/预览 | `OwnershipRevertPreview` | `ownershipReviewApi.previewOwnershipGroupRevert` |
| `apply_ownership_group_revert` | `commands::ownership` | 写入/执行/生成文件 | `OwnershipGroupDecisionResult` | `ownershipReviewApi.applyOwnershipGroupRevert` |
| `revoke_ownership_group_authorization` | `commands::ownership` | 写入/执行/生成文件 | `OwnershipRevokeResult` | `ownershipReviewApi.revokeOwnershipGroupAuthorization` |
| `confirm_asset_ownership` | `commands::ownership` | 写入/执行/生成文件 | `OwnershipConfirmResult` | `ownershipAuthorizationApi.confirmAssetOwnership` |
| `authorize_asset_ownership` | `commands::ownership` | 写入/执行/生成文件 | `number` | `ownershipAuthorizationApi.authorizeAssetOwnership` |
| `get_exposure_settings` | `commands::exposure` | 读取/预览 | `ExposureSettingsView` | `exposureApi.getExposureSettings` |
| `list_exposure_setting_versions` | `commands::exposure` | 读取/预览 | `ExposureSettingVersionPage` | `exposureApi.listExposureSettingVersions` |
| `save_exposure_settings` | `commands::exposure` | 写入/执行/生成文件 | `number` | `exposureApi.saveExposureSettings` |
| `page_exposure_clusters` | `commands::exposure` | 读取/预览 | `ExposurePage<ExposureCluster>` | `exposureApi.pageExposureClusters` |
| `page_exposure_cluster_members` | `commands::exposure` | 读取/预览 | `ExposurePage<ExposureClusterMember>` | `exposureApi.pageExposureClusterMembers` |
| `exposure_review_taxonomy` | `commands::exposure` | 读取/预览 | `ExposureTaxonomyRow[]` | `exposureApi.exposureReviewTaxonomy` |
| `list_exposure_review_trail` | `commands::exposure` | 读取/预览 | `ExposureReviewEvent[]` | `exposureApi.listExposureReviewTrail` |
| `read_exposure_retest_outcome` | `commands::exposure` | 读取/预览 | `ExposureRetestOutcome` | `exposureApi.readExposureRetestOutcome` |
| `verify_exposure_export` | `commands::exposure` | 读取/预览 | `JsonLike` | `exposureApi.verifyExposureExport` |
| `page_exposure_remediations` | `commands::exposure` | 读取/预览 | `ExposurePage<ExposureRemediation>` | `exposureApi.pageExposureRemediations` |
| `review_exposure_finding_scoped` | `commands::exposure` | 写入/执行/生成文件 | `number` | `exposureApi.reviewExposureFindingScoped` |
| `save_exposure_remediation` | `commands::exposure` | 写入/执行/生成文件 | `number` | `exposureApi.saveExposureRemediation` |
| `start_exposure_retest` | `commands::exposure` | 写入/执行/生成文件 | `number` | `exposureApi.startExposureRetest` |
| `mark_exposure_remediation` | `commands::exposure` | 写入/执行/生成文件 | `ExposureRetestOutcome` | `exposureApi.markExposureRemediation` |
| `import_exposure_documents` | `commands::exposure` | 写入/执行/生成文件 | `unknown` | `exposureApi.importExposureDocuments` |
| `grade_exposure_intel` | `commands::exposure` | 写入/执行/生成文件 | `unknown` | `exposureApi.gradeExposureIntel` |
| `page_exposure_incidents` | `commands::exposure` | 读取/预览 | `unknown` | `exposureIncidentApi.pageExposureIncidents` |
| `read_exposure_incident` | `commands::exposure` | 读取/预览 | `unknown` | `exposureIncidentApi.readExposureIncident` |
| `page_exposure_incident_locations` | `commands::exposure` | 读取/预览 | `unknown` | `exposureIncidentApi.pageExposureIncidentLocations` |
| `act_on_exposure_incident` | `commands::exposure` | 写入/执行/生成文件 | `unknown` | `exposureIncidentApi.actOnExposureIncident` |
| `exposure_coverage_drilldown` | `commands::exposure` | 读取/预览 | `unknown` | `exposureIncidentApi.exposureCoverageDrilldown` |
| `propose_exposure_incidents` | `commands::exposure` | 写入/执行/生成文件 | `unknown` | `exposureIncidentApi.proposeExposureIncidents` |

机器清单 [legacy-ipc-contracts.json](legacy-ipc-contracts.json) 保留每个 facade 重复消费者、源文件与行号、输入 SHA，以及真实 handler owner。

复核命令（开发期需 TypeScript parser；不打包进运行时）：

```sh
node apps/assets/scripts/audit-legacy-contracts.mjs --check
```
