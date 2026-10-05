/// §Stage 3 item 1 — the instruction a run is handed, named without any execution
/// backend attached. Rendering, writing and auditing go through this type so a later
/// stage cannot reintroduce a backend-specific path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentInstruction {
    text: String,
}

impl AgentInstruction {
    /// The file the instruction is kept in inside a work directory.
    pub const FILE_NAME: &'static str = "agent-instruction.md";

    pub fn new(text: String) -> Self {
        Self { text }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn path_in(&self, work_dir: &Path) -> PathBuf {
        work_dir.join(Self::FILE_NAME)
    }

    pub fn write_to(&self, work_dir: &Path) -> Result<PathBuf, String> {
        let path = self.path_in(work_dir);
        fs::write(&path, &self.text).map_err(|error| format!("无法写入执行指令：{error}"))?;
        Ok(path)
    }
}

/// Resume never promotes a retired or unknown backend to Native, including old
/// rows that startup no longer mutates. A failed lookup must fail closed.
fn retired_backend_resume_refusal(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<Option<String>, String> {
    let incompatible: i64 = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 \
             AND (backend<>'native' OR status='legacy_backend_removed'))",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法核验任务后端，拒绝恢复：{error}"))?;
    if incompatible != 0 {
        return Ok(Some(
            "任务包含退役或未知后端执行，不能恢复；请发起全新的 Native 执行".to_string(),
        ));
    }
    Ok(None)
}

/// Every task uses the current Native work root. No older directory is searched.
pub const SCAN_WORK_DIRECTORY: &str = "agent-jobs";

fn neutral_scan_work_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(SCAN_WORK_DIRECTORY)
}

fn scan_work_dir_for(app_data_dir: &Path, scan_id: &str) -> PathBuf {
    neutral_scan_work_root(app_data_dir).join(scan_id)
}

/// Creates (idempotently) the working directory of one scan and returns its path.
// Legacy layout fixture only; live admission uses exclusive attempt ownership.
#[cfg(test)]
fn prepare_scan_work_dir(app_data_dir: &Path, scan_id: &str) -> Result<PathBuf, String> {
    let work = neutral_scan_work_root(app_data_dir).join(scan_id);
    fs::create_dir_all(&work).map_err(|error| format!("无法创建任务工作目录：{error}"))?;
    Ok(work)
}
