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

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("oviraptor-export-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn snapshot_export_publishes_only_complete_private_bytes() {
        let f = Fixture::new();
        let path = f.0.join("report.json");
        publish_with(&f.0, &path, |file| {
            file.write_all(b"first")?;
            assert!(!path.exists(), "partial bytes must not be published");
            file.write_all(b"second")
        })
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"firstsecond");
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn snapshot_export_failed_write_leaves_no_partial_report_or_temporary_file() {
        let f = Fixture::new();
        let path = f.0.join("report.json");
        assert_eq!(
            publish_with(&f.0, &path, |file| {
                file.write_all(b"partial secret")?;
                Err(std::io::Error::other("injected"))
            })
            .unwrap_err(),
            ExportError::Write
        );
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    }

    #[test]
    fn snapshot_export_collision_never_replaces_existing_file() {
        let f = Fixture::new();
        let path = f.0.join("report.json");
        fs::write(&path, b"keep").unwrap();
        assert_eq!(
            publish_with(&f.0, &path, |file| file.write_all(b"new")).unwrap_err(),
            ExportError::Publish
        );
        assert_eq!(fs::read(&path).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_export_collision_never_follows_symlink_or_uses_symlink_directory() {
        let f = Fixture::new();
        let victim = f.0.join("keep");
        let path = f.0.join("report.json");
        fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        assert_eq!(
            publish_with(&f.0, &path, |file| file.write_all(b"new")).unwrap_err(),
            ExportError::Publish
        );
        assert_eq!(fs::read(&victim).unwrap(), b"keep");
        let link = f.0.join("linked-directory");
        std::os::unix::fs::symlink(&f.0, &link).unwrap();
        assert_eq!(
            write_json(&link, Kind::Task, &serde_json::json!({})).unwrap_err(),
            ExportError::Directory
        );
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 3);
    }

    #[test]
    fn snapshot_export_concurrent_reports_get_distinct_complete_files() {
        let f = Fixture::new();
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let dir = f.0.clone();
                std::thread::spawn(move || {
                    write_json(&dir, Kind::Project, &serde_json::json!({"index": i})).unwrap()
                })
            })
            .collect();
        let paths: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_ne!(paths[0], paths[1]);
        for (i, path) in paths.iter().enumerate() {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&fs::read(path).unwrap()).unwrap()
                    ["index"],
                i
            );
        }
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 2);
    }
}
