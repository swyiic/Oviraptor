//! Publish complete, private files without replacing an existing key or CAS object.
//! A same-directory hard link is atomic and fails if another importer won the race.
use std::io::Write;
use std::path::{Path, PathBuf};

struct PendingFile(PathBuf);

impl Drop for PendingFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(super) fn publish_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("object path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let pending = PendingFile(parent.join(format!(".import-{}.tmp", uuid::Uuid::new_v4())));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&pending.0)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    match std::fs::hard_link(&pending.0, path) {
        Ok(()) => {
            #[cfg(unix)]
            std::fs::File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn read_regular(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    if !std::fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "key/object must be a regular file, not a link or device",
        ));
    }
    std::fs::read(path)
}
