//! Upper-layer owner allocation. Native spends only `nativeCap`.
//! Pending and the host share are not available to this ledger.
//! Preset names map to the existing runtime_config modes; this file does not invent timeouts or prices.

use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub fn reserve_native_share(path: &Path, amount: u64) -> Result<(), String> {
    if !path.is_file() {
        return Ok(());
    }
    if amount == 0 {
        return Err("native_allocation_invalid".into());
    }
    let text = fs::read_to_string(path).map_err(|error| format!("native_allocation_unreadable:{error}"))?;
    let mut value: Value = serde_json::from_str(&text).map_err(|_| "native_allocation_invalid".to_string())?;
    let native_cap = value.get("nativeCap").and_then(Value::as_u64).ok_or("native_allocation_invalid")?;
    let spent = value.get("nativeSpent").and_then(Value::as_u64).unwrap_or(0);
    let host_before = value.get("hostCap").cloned();
    let pending_before = value.get("pending").cloned();
    if spent.saturating_add(amount) > native_cap {
        return Err("native_allocation_exhausted".into());
    }
    value["nativeSpent"] = json!(spent + amount);
    if value.get("hostCap") != host_before.as_ref() || value.get("pending") != pending_before.as_ref() {
        return Err("native_allocation_invalid".into());
    }
    fs::write(path, serde_json::to_string(&value).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    Ok(())
}



