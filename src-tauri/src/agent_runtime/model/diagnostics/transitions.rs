//! Allowed saved SDK prefixes, never inferred model or role completion.
use rusqlite::Connection;
pub(super) fn validate(
    db: &Connection,
    owner: &str,
    stage: &str,
    cost: &str,
    terminal: &str,
) -> Result<(), String> {
    let prior = prefix(db, owner)?;
    check(&prior, stage, cost, terminal)
}
pub(super) fn verify_prefix(db: &Connection, owner: &str) -> Result<(), String> {
    prefix(db, owner).map(|_| ())
}
fn prefix(db: &Connection, owner: &str) -> Result<Vec<(String, String, String)>, String> {
    let prior=db.prepare("SELECT stage,cost_phase,terminal_state FROM native_sdk_log_rows WHERE owner_id=?1 ORDER BY ordinal")
  .map_err(|_|"native_sdk_log_stage_invalid")?.query_map([owner],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))
  .map_err(|_|"native_sdk_log_stage_invalid")?.collect::<Result<Vec<_>,_>>().map_err(|_|"native_sdk_log_stage_invalid")?;
    let mut prefix = Vec::new();
    for row in prior {
        check(&prefix, &row.0, &row.1, &row.2)?;
        prefix.push(row);
    }
    Ok(prefix)
}
fn check(
    prior: &[(String, String, String)],
    stage: &str,
    cost: &str,
    terminal: &str,
) -> Result<(), String> {
    let last = prior.last().map(|r| r.0.as_str());
    let phase = prior
        .iter()
        .find(|r| r.0 == "cost_saved")
        .map(|r| r.1.as_str());
    let valid = match stage {
        "prepared" => prior.is_empty() && cost.is_empty() && terminal.is_empty(),
        "sent" => last == Some("prepared") && cost.is_empty() && terminal.is_empty(),
        "response_received" => last == Some("sent") && cost.is_empty() && terminal.is_empty(),
        "cost_saved" => {
            terminal.is_empty()
                && match cost {
                    "received" => last == Some("response_received"),
                    "uncertain" => matches!(last, Some("sent" | "response_received")),
                    "unsent" => last == Some("prepared"),
                    _ => false,
                }
        }
        "validated" => {
            last == Some("cost_saved")
                && phase == Some("received")
                && cost.is_empty()
                && terminal.is_empty()
        }
        "terminal" => match terminal {
            "returned" => {
                last == Some("validated") && phase == Some("received") && cost == "received"
            }
            "withheld" => {
                last == Some("cost_saved") && phase == Some("received") && cost == "received"
            }
            "uncertain" => {
                last == Some("cost_saved") && phase == Some("uncertain") && cost == "uncertain"
            }
            "unsent" => last == Some("cost_saved") && phase == Some("unsent") && cost == "unsent",
            "failed" => {
                matches!(last, Some("prepared" | "sent" | "response_received"))
                    && phase.is_none()
                    && cost.is_empty()
            }
            _ => false,
        },
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("native_sdk_log_stage_invalid".into())
    }
}
