//! Native Assets operator adapter. Reuses the original domain owner bodies.
//! Database/export paths are trusted bootstrap capabilities, never RPC fields.
use crate::commands::ownership;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

fn input<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "asset_service_arguments_invalid".into())
}
fn output(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "asset_service_result_invalid".into())
}
fn same_project(expected: i64, provided: i64) -> Result<(), String> {
    if expected <= 0 || expected > 9_007_199_254_740_991 || expected != provided {
        return Err("asset_service_project_scope_mismatch".into());
    }
    Ok(())
}
macro_rules! args {
    ($name:ident {$($field:ident : $kind:ty),* $(,)?}) => {
        #[derive(Deserialize)]
        #[serde(rename_all="camelCase",deny_unknown_fields)]
        struct $name { $($field:$kind),* }
    };
}

args!(GetAssetOwnershipProfileArgs { project_id: i64 });
args!(SaveAssetOwnershipProfileArgs {
    input: crate::asset_models::AssetOwnershipProfile,
    expected_version: i64
});
args!(GetAssetOwnershipSummaryArgs { project_id: i64 });
args!(AssessAssetOwnershipArgs { project_id: i64 });
args!(UpdateAssetOwnershipArgs { input: crate::asset_models::AssetOwnershipDecisionInput, expected_project_id: i64, expected_decisions: Vec<ownership::AssetOwnershipDecisionExpectation> });
args!(GetProjectOwnershipBootstrapArgs { project_id: i64 });
args!(PreviewProjectOwnershipSuggestionsArgs { project_id: i64 });
args!(SaveProjectOwnershipDirectoryArgs {
    input: ownership::ProjectOwnershipDirectorySave
});
args!(StartOwnershipAnalysisArgs { project_id: i64, mode: Option<String> });
args!(CancelOwnershipAnalysisArgs {
    project_id: i64,
    run_id: i64
});
args!(ReadOwnershipAnalysisArgs { project_id: i64 });
args!(PageOwnershipGroupsArgs {
    input: ownership::OwnershipGroupPageInput
});
args!(PageOwnershipGroupMembersArgs {
    input: ownership::OwnershipMemberPageInput
});
args!(PreviewOwnershipGroupDecisionArgs {
    input: ownership::OwnershipGroupDecisionRequest
});
args!(ApplyOwnershipGroupDecisionArgs {
    input: ownership::OwnershipGroupDecisionApply
});
args!(ReadOwnershipOperationArgs {
    project_id: i64,
    operation_id: String
});
args!(PreviewOwnershipGroupRevertArgs {
    input: ownership::OwnershipRevertRequest
});
args!(ApplyOwnershipGroupRevertArgs {
    input: ownership::OwnershipRevertApply
});
args!(RevokeOwnershipGroupAuthorizationArgs {
    input: ownership::OwnershipRevokeRequest
});
args!(ConfirmAssetOwnershipArgs { input: ownership::OwnershipConfirmInput, expected_project_id: i64, expected_decisions: Vec<ownership::AssetOwnershipDecisionExpectation> });
args!(AuthorizeAssetOwnershipArgs { input: ownership::OwnershipAuthorizeInput, expected_project_id: i64, expected_decisions: Vec<ownership::AssetOwnershipDecisionExpectation> });

pub(crate) fn project_commands() -> &'static [&'static str] {
    &[
        "get_asset_ownership_profile",
        "save_asset_ownership_profile",
        "get_asset_ownership_summary",
        "assess_asset_ownership",
        "update_asset_ownership",
        "get_project_ownership_bootstrap",
        "preview_project_ownership_suggestions",
        "start_ownership_analysis",
        "cancel_ownership_analysis",
        "read_ownership_analysis",
        "page_ownership_groups",
        "page_ownership_group_members",
        "preview_ownership_group_decision",
        "apply_ownership_group_decision",
        "read_ownership_operation",
        "preview_ownership_group_revert",
        "apply_ownership_group_revert",
        "revoke_ownership_group_authorization",
        "confirm_asset_ownership",
        "authorize_asset_ownership",
    ]
}
pub(crate) fn application_commands() -> &'static [&'static str] {
    &["save_project_ownership_directory"]
}

/// Project-bound manual UI actions. Never registered as model tools.
pub(crate) fn dispatch_project(
    db_path: &Path,
    project: i64,
    command: &str,
    arguments: Value,
) -> Result<Value, String> {
    ownership::with_background_scope(db_path, || {
        dispatch_project_scoped(db_path, project, command, arguments)
    })
}

fn dispatch_project_scoped(
    db_path: &Path,
    project: i64,
    command: &str,
    arguments: Value,
) -> Result<Value, String> {
    let _admission = super::owned_work::admission()?;
    same_project(project, project)?;
    match command {
        "get_asset_ownership_profile" => {
            let a: GetAssetOwnershipProfileArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(ownership::get_asset_ownership_profile_at(
                db_path,
                a.project_id,
            )?)
        }
        "save_asset_ownership_profile" => {
            let a: SaveAssetOwnershipProfileArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            output(ownership::save_asset_ownership_profile_at(
                db_path,
                a.input,
                a.expected_version,
            )?)
        }
        "get_asset_ownership_summary" => {
            let a: GetAssetOwnershipSummaryArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(ownership::get_asset_ownership_summary_at(
                db_path,
                a.project_id,
            )?)
        }
        "assess_asset_ownership" => {
            let a: AssessAssetOwnershipArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(ownership::assess_asset_ownership_at(db_path, a.project_id)?)
        }
        "update_asset_ownership" => {
            let a: UpdateAssetOwnershipArgs = input(arguments)?;
            same_project(project, a.expected_project_id)?;
            for item in &a.input.selections {
                same_project(project, item.project_id)?;
            }
            output(ownership::update_asset_ownership_at(
                db_path,
                a.input,
                a.expected_project_id,
                a.expected_decisions,
            )?)
        }
        "get_project_ownership_bootstrap" => {
            let a: GetProjectOwnershipBootstrapArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(ownership::get_project_ownership_bootstrap_at(
                db_path,
                a.project_id,
            )?)
        }
        "preview_project_ownership_suggestions" => {
            let a: PreviewProjectOwnershipSuggestionsArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(ownership::preview_project_ownership_suggestions_at(
                db_path,
                a.project_id,
            )?)
        }
        "start_ownership_analysis" => {
            let a: StartOwnershipAnalysisArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Ownership)?;
            let result = ownership::start_ownership_analysis_at(db_path, a.project_id, a.mode)?;
            super::owned_work::record(
                db_path,
                super::owned_work::Kind::OwnershipAnalysis,
                project,
                &result.id.to_string(),
            )?;
            output(result)
        }
        "cancel_ownership_analysis" => {
            let a: CancelOwnershipAnalysisArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            output(ownership::cancel_ownership_analysis_at(
                db_path,
                a.project_id,
                a.run_id,
            )?)
        }
        "read_ownership_analysis" => {
            let a: ReadOwnershipAnalysisArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Ownership)?;
            output(ownership::read_ownership_analysis_at(
                db_path,
                a.project_id,
            )?)
        }
        "page_ownership_groups" => {
            let a: PageOwnershipGroupsArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            output(ownership::page_ownership_groups_at(db_path, a.input)?)
        }
        "page_ownership_group_members" => {
            let a: PageOwnershipGroupMembersArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            output(ownership::page_ownership_group_members_at(
                db_path, a.input,
            )?)
        }
        "preview_ownership_group_decision" => {
            let a: PreviewOwnershipGroupDecisionArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            output(ownership::preview_ownership_group_decision_at(
                db_path, a.input,
            )?)
        }
        "apply_ownership_group_decision" => {
            let a: ApplyOwnershipGroupDecisionArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Ownership)?;
            super::owned_work::require_owned_if_exists(
                db_path,
                super::owned_work::Kind::OwnershipOperation,
                &a.input.operation_id,
            )?;
            let result = ownership::apply_ownership_group_decision_at(db_path, a.input)?;
            super::owned_work::record(
                db_path,
                super::owned_work::Kind::OwnershipOperation,
                project,
                &result.operation_id,
            )?;
            output(result)
        }
        "read_ownership_operation" => {
            let a: ReadOwnershipOperationArgs = input(arguments)?;
            same_project(project, a.project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Ownership)?;
            output(ownership::read_ownership_operation_at(
                db_path,
                a.project_id,
                a.operation_id,
            )?)
        }
        "preview_ownership_group_revert" => {
            let a: PreviewOwnershipGroupRevertArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            output(ownership::preview_ownership_group_revert_at(
                db_path, a.input,
            )?)
        }
        "apply_ownership_group_revert" => {
            let a: ApplyOwnershipGroupRevertArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Ownership)?;
            super::owned_work::require_owned_if_exists(
                db_path,
                super::owned_work::Kind::OwnershipOperation,
                &a.input.operation_id,
            )?;
            let result = ownership::apply_ownership_group_revert_at(db_path, a.input)?;
            super::owned_work::record(
                db_path,
                super::owned_work::Kind::OwnershipOperation,
                project,
                &result.operation_id,
            )?;
            output(result)
        }
        "revoke_ownership_group_authorization" => {
            let a: RevokeOwnershipGroupAuthorizationArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            super::owned_work::guard_recovery(db_path, super::owned_work::Family::Ownership)?;
            super::owned_work::require_owned_if_exists(
                db_path,
                super::owned_work::Kind::OwnershipOperation,
                &a.input.operation_id,
            )?;
            let result = ownership::revoke_ownership_group_authorization_at(db_path, a.input)?;
            super::owned_work::record(
                db_path,
                super::owned_work::Kind::OwnershipOperation,
                project,
                &result.operation_id,
            )?;
            output(result)
        }
        "confirm_asset_ownership" => {
            let a: ConfirmAssetOwnershipArgs = input(arguments)?;
            same_project(project, a.expected_project_id)?;
            for item in &a.input.selections {
                same_project(project, item.project_id)?;
            }
            output(ownership::confirm_asset_ownership_at(
                db_path,
                a.input,
                a.expected_project_id,
                a.expected_decisions,
            )?)
        }
        "authorize_asset_ownership" => {
            let a: AuthorizeAssetOwnershipArgs = input(arguments)?;
            same_project(project, a.expected_project_id)?;
            for item in &a.input.selections {
                same_project(project, item.project_id)?;
            }
            output(ownership::authorize_asset_ownership_at(
                db_path,
                a.input,
                a.expected_project_id,
                a.expected_decisions,
            )?)
        }
        _ => Err("asset_service_ownership_method_unavailable".into()),
    }
}

/// Global organization facts change: requires explicit application-wide operator permission.
pub(crate) fn dispatch_application(
    db_path: &Path,
    project: i64,
    command: &str,
    arguments: Value,
) -> Result<Value, String> {
    same_project(project, project)?;
    match command {
        "save_project_ownership_directory" => {
            let a: SaveProjectOwnershipDirectoryArgs = input(arguments)?;
            same_project(project, a.input.project_id)?;
            output(ownership::save_project_ownership_directory_at(
                db_path, a.input,
            )?)
        }
        _ => Err("asset_service_ownership_method_unavailable".into()),
    }
}

/// Includes actual owner threads even after the domain row became terminal.
pub(crate) fn status_on(db_path: &Path) -> Result<Value, String> {
    super::owned_work::status_on(
        db_path,
        super::owned_work::Family::Ownership,
        ownership::active_background_workers(db_path)?,
    )
}

#[cfg(test)]
#[path = "ownership_service_tests.rs"]
mod tests;
