//! Task-external administrator decisions for signed tool inventories.
//! Approval is preparation metadata and does not itself grant task execution.
use crate::AppState;
use tauri::{AppHandle, Manager};

// Task-external signed inventory decisions. These commands do not grant execution.
static TOOL_INVENTORY_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct ToolInventoryCall;
impl Drop for ToolInventoryCall {
    fn drop(&mut self) {
        TOOL_INVENTORY_BUSY.store(false, std::sync::atomic::Ordering::Release);
    }
}
fn claim_tool_inventory_call() -> Result<ToolInventoryCall, String> {
    TOOL_INVENTORY_BUSY.compare_exchange(false, true,
        std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire)
        .map_err(|_| "tool_administrator_service_busy".to_string())?;
    Ok(ToolInventoryCall)
}
#[tauri::command]
pub async fn approve_tool_inventory(
    app: AppHandle, request: tauri::ipc::Request<'_>,
) -> Result<crate::tool_supply::administrator_service::ApprovalInventory, String> {
    let call = claim_tool_inventory_call()?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("tool_administrator_binary_frame_required".into());
    };
    tool_inventory_frame(bytes)?;
    let bytes = bytes.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _call = call;
        let (decision, manifest, package) = tool_inventory_frame(&bytes)?;
        let state = app.state::<AppState>();
        crate::tool_supply::administrator_service::approve(
            &app, &state, decision, manifest, package,
        )
    }).await.map_err(|_| "tool_administrator_service_worker_failed".to_string())?
}

type ToolInventoryFrame<'a> = (&'a [u8], &'a [u8], &'a [u8]);
fn tool_inventory_frame(bytes: &[u8]) -> Result<ToolInventoryFrame<'_>, String> {
    let invalid = || "tool_administrator_binary_frame_invalid".to_string();
    if bytes.len() < 20 || bytes.len() > 20 + 32 * 1024 + 64 * 1024 * 1024
        || &bytes[..8] != b"OVTA0001" { return Err(invalid()); }
    let sizes: Vec<usize> = bytes[8..20].as_chunks::<4>().0.iter()
        .map(|part| u32::from_le_bytes(*part) as usize).collect();
    if sizes.contains(&0) || sizes[0] > 16 * 1024
        || sizes[1] > 16 * 1024 || sizes[2] > 64 * 1024 * 1024 {
        return Err(invalid());
    }
    let manifest_start = 20 + sizes[0];
    let package_start = manifest_start + sizes[1];
    if package_start + sizes[2] != bytes.len() { return Err(invalid()); }
    Ok((&bytes[20..manifest_start], &bytes[manifest_start..package_start], &bytes[package_start..]))
}


#[tauri::command]
pub async fn revoke_tool_inventory(
    app: AppHandle, decision: Vec<u8>,
) -> Result<crate::tool_supply::administrator_service::RevocationInventory, String> {
    if decision.len() > 16 * 1024 { return Err("tool_administrator_service_input_too_large".into()); }
    let call = claim_tool_inventory_call()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _call = call;
        let state = app.state::<AppState>();
        crate::tool_supply::administrator_service::revoke(&app, &state, &decision)
    }).await.map_err(|_| "tool_administrator_service_worker_failed".to_string())?
}

#[tauri::command]
pub async fn read_tool_approval_inventory(
    app: AppHandle, approval_id: String,
) -> Result<Option<crate::tool_supply::administrator_service::ApprovalInventory>, String> {
    if approval_id.len() != 36 { return Err("tool_administrator_approval_id_invalid".into()); }
    let call = claim_tool_inventory_call()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _call = call;
        let state = app.state::<AppState>();
        crate::tool_supply::administrator_service::read(&app, &state, &approval_id)
    }).await.map_err(|_| "tool_administrator_service_worker_failed".to_string())?
}
