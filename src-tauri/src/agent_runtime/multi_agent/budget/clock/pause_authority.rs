//! Finite business columns for atomic, already paid Source pause publication.
//! Original failure projection only; no new work, spend, asset writes or delete.
pub(super) fn allows(table: &str, column: &str) -> bool {
    match table {
        "sentinel_scans" => matches!(column, "status" | "current_checkpoint" | "updated_at"),
        "sentinel_scan_attempts" => matches!(
            column,
            "status"
                | "stage"
                | "checkpoint"
                | "stop_reason"
                | "finished_at"
                | "updated_at"
                | "llm_requests_delta"
                | "input_tokens_delta"
                | "output_tokens_delta"
                | "cached_tokens_delta"
                | "total_tokens_delta"
        ),
        "native_scan_branches" => matches!(
            column,
            "status" | "checkpoint" | "report_json" | "updated_at"
        ),
        "sentinel_targets" => matches!(column, "status" | "last_attempt_number"),
        _ => false,
    }
}
