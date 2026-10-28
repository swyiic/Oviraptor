use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

const RETAINED_ROWS: i64 = 20_000;
const PAGE_LIMIT: i64 = 300;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    pub id: i64,
    pub stage: String,
    pub stream: String,
    pub message: String,
    pub time: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    pub rows: Vec<Row>,
    pub more: bool,
    pub older_rows: i64,
    pub earliest_id: i64,
}

pub(crate) struct Journal {
    connection: Connection,
}

impl Journal {
    pub fn open(path: &Path) -> Result<Self, String> {
        Ok(Self {
            connection: crate::db::open(path)?,
        })
    }

    pub fn append(&self, stage: &str, stream: &str, message: &str) -> Result<i64, String> {
        if !matches!(stream, "status" | "stdout" | "stderr" | "success" | "error") {
            return Err("安装输出流类型无效".into());
        }
        let safe_stage: String = stage.chars().take(120).collect();
        let safe_message = crate::log_display::line(message);
        self.connection
            .execute(
                "INSERT INTO environment_install_logs(stage,stream,message) VALUES(?1,?2,?3)",
                params![safe_stage, stream, safe_message],
            )
            .map_err(|_| "无法持久记录安装输出".to_string())?;
        let id = self.connection.last_insert_rowid();
        // A bounded journal avoids unbounded growth on noisy installers. A
        // missed range is detectable from the first retained ID on replay.
        if id % 256 == 0 {
            self.connection
                .execute(
                    "DELETE FROM environment_install_logs WHERE id<=?1",
                    [id.saturating_sub(RETAINED_ROWS)],
                )
                .map_err(|_| "无法维护安装输出保留窗口".to_string())?;
        }
        Ok(id)
    }
}

pub(crate) fn page(
    connection: &Connection,
    after_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Page, String> {
    let after = after_id.unwrap_or(0);
    let count = limit.unwrap_or(PAGE_LIMIT);
    if after < 0 || !(1..=PAGE_LIMIT).contains(&count) {
        return Err("安装日志游标或页大小无效".into());
    }
    let earliest_id: i64 = connection
        .query_row(
            "SELECT COALESCE(MIN(id),0) FROM environment_install_logs",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "无法读取安装日志".to_string())?;
    let initial = after_id.is_none();
    let query = if initial {
        "SELECT id,stage,stream,message,created_at FROM environment_install_logs ORDER BY id DESC LIMIT ?1"
    } else {
        "SELECT id,stage,stream,message,created_at FROM environment_install_logs WHERE id>?2 ORDER BY id ASC LIMIT ?1"
    };
    let arguments = if initial {
        vec![count + 1]
    } else {
        vec![count + 1, after]
    };
    let mut rows: Vec<Row> = connection
        .prepare(query)
        .map_err(|_| "无法读取安装日志".to_string())?
        .query_map(rusqlite::params_from_iter(arguments), |row| {
            Ok(Row {
                id: row.get(0)?,
                stage: row.get(1)?,
                stream: row.get(2)?,
                message: row.get(3)?,
                time: row.get(4)?,
            })
        })
        .map_err(|_| "无法读取安装日志".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "无法读取安装日志".to_string())?;
    let more = rows.len() > count as usize;
    if more {
        rows.pop();
    }
    let older_rows = if initial {
        if let Some(last) = rows.last() {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM environment_install_logs WHERE id<?1",
                    [last.id],
                    |row| row.get(0),
                )
                .map_err(|_| "无法读取安装日志".to_string())?
        } else {
            0
        }
    } else {
        0
    };
    if initial {
        rows.reverse();
    }
    Ok(Page {
        rows,
        more: more && !initial,
        older_rows,
        earliest_id,
    })
}

