//! Publish complete JSON reports without replacing existing files. Export names
//! never include task identifiers, project names, or other imported data.
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) enum Kind {
    Task,
    Project,
    SourceReview,
    SourceReviewSarif,
    SourceReviewBundle,
}
impl Kind {
    fn prefix(&self) -> &'static str {
        match self {
            Self::Task => "sentinel-task",
            Self::Project => "sentinel-project",
            Self::SourceReview | Self::SourceReviewSarif => "source-review",
            Self::SourceReviewBundle => "source-review-bundle",
        }
    }
    fn extension(&self) -> &'static str {
        match self {
            Self::SourceReviewSarif => "sarif",
            _ => "json",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExportError {
    Encoding,
    Directory,
    File,
    Write,
    Publish,
    DurabilityUnknown,
}
impl ExportError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Encoding => "encoding_failed",
            Self::Directory => "directory_failed",
            Self::File => "file_failed",
            Self::Write => "write_failed",
            Self::Publish => "publish_failed",
            Self::DurabilityUnknown => "durability_unknown",
        }
    }
}

struct Pending(Option<PathBuf>);
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn write_json(
    directory: &Path,
    kind: Kind,
    value: &serde_json::Value,
) -> Result<String, ExportError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| ExportError::Encoding)?;
    fs::create_dir_all(directory).map_err(|_| ExportError::Directory)?;
    if !fs::symlink_metadata(directory)
        .map_err(|_| ExportError::Directory)?
        .is_dir()
    {
        return Err(ExportError::Directory);
    }
    let path = directory.join(format!(
        "{}-{}.{}",
        kind.prefix(),
        uuid::Uuid::new_v4(),
        kind.extension()
    ));
    publish_with(directory, &path, |file| file.write_all(&bytes))?;
    Ok(path.to_string_lossy().into_owned())
}

fn publish_with(
    directory: &Path,
    path: &Path,
    fill: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> Result<(), ExportError> {
    if path.parent() != Some(directory) {
        return Err(ExportError::File);
    }
    let pending_path = directory.join(format!(".snapshot-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Arm cleanup only after exclusive creation succeeds: never delete a
    // pre-existing file, even on the astronomically unlikely UUID collision.
    let mut file = options.open(&pending_path).map_err(|_| ExportError::File)?;
    let mut pending = Pending(Some(pending_path.clone()));
    let written = fill(&mut file).and_then(|()| file.sync_all());
    drop(file);
    written.map_err(|_| ExportError::Write)?;
    // Same-directory hard link publishes complete bytes atomically and refuses
    // both regular-file and symlink collisions. No replacing rename fallback.
    fs::hard_link(&pending_path, path).map_err(|_| ExportError::Publish)?;
    fs::remove_file(&pending_path).map_err(|_| ExportError::DurabilityUnknown)?;
    pending.0 = None;
    // A failure after publication must not delete the complete final report or
    // claim confirmed success. An interrupted export may require user inspection.
    #[cfg(unix)]
    File::open(directory)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| ExportError::DurabilityUnknown)?;
    Ok(())
}

