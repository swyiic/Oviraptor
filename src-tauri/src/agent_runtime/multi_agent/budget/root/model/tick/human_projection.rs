//! Select only display metadata from the already verified original paid basis.
//! No current directive lookup, authority, repair, lease acquisition or write.
use serde_json::{json, Value};
fn name(value: &Value) -> Result<&str, String> {
    value
        .as_str()
        .filter(|s| {
            !s.trim().is_empty() && s.chars().count() <= 200 && !s.chars().any(char::is_control)
        })
        .ok_or_else(|| "root_tick_human_projection_invalid".into())
}
pub(super) fn from_basis(basis: &Value, target: &str) -> Result<Option<Value>, String> {
    let frame = &basis["changedFact"];
    if basis["phase"] != "human-directive" && frame["kind"] != "human-directive" {
        return Ok(None);
    }
    let invalid = "root_tick_human_projection_invalid";
    if basis["phase"] != "human-directive"
        || frame["kind"] != "human-directive"
        || basis["permittedSuggestion"] != "assess:human_directive"
    {
        return Err(invalid.into());
    }
    let semantic = &frame["semantic"];
    let revision = semantic["revision"]
        .as_i64()
        .filter(|n| *n > 0 && *n <= 9_007_199_254_740_991)
        .ok_or(invalid)?;
    let hash = semantic["draftHash"]
        .as_str()
        .filter(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
        .ok_or(invalid)?;
    if semantic["targetKey"] != target {
        return Err(invalid.into());
    }
    Ok(Some(
        json!({"schemaVersion":1,"directiveId":name(&semantic["directiveId"])? ,"draftId":name(&semantic["draftId"])? ,"revision":revision,"draftHash":hash,"confirmationReceiptId":name(&semantic["confirmationReceiptId"])? ,"threadKey":name(&semantic["threadKey"])? ,"targetKey":target}),
    ))
}
