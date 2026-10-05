use super::*;
use std::fs;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("oviraptor-manifest-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn manifest_snapshot_hash_and_payload_use_one_read_even_when_source_changes() {
    let f = Fixture::new();
    let path = f.file("run.json", br#"{"id":"old"}"#);
    let original = fs::read(&path).unwrap();
    let mut reads = 0;
    let (manifest, payloads) = build_with_reader(
        &f.0,
        &f.0,
        std::slice::from_ref(&path),
        &Limits::default(),
        |p, _| {
            reads += 1;
            let bytes = fs::read(p)?;
            fs::write(p, br#"{"id":"new"}"#)?;
            Ok(bytes)
        },
    )
    .unwrap();
    assert_eq!(manifest.files[0].content_hash, sha256_hex(&payloads[0].1));
    assert_eq!(payloads[0].1, original);
    assert_eq!(reads, 1);
    assert_ne!(fs::read(path).unwrap(), original);
}

#[test]
fn manifest_snapshot_growth_after_metadata_cannot_exceed_file_budget() {
    let f = Fixture::new();
    let path = f.file("run.json", b"x");
    let limits = Limits {
        file_bytes: 4,
        ..Limits::default()
    };
    let result = build_with_reader(&f.0, &f.0, &[path], &limits, |p, cap| {
        fs::write(p, b"12345")?;
        read_bounded_regular(p, cap)
    });
    assert!(result
        .unwrap_err()
        .iter()
        .any(|d| d.code == "file_bytes_limit"));
}

#[test]
fn manifest_snapshot_growth_cannot_exceed_aggregate_budget() {
    let f = Fixture::new();
    let paths = [f.file("run.json", b"x"), f.file("coverage.json", b"x")];
    let limits = Limits {
        file_bytes: 8,
        bundle_bytes: 6,
        ..Limits::default()
    };
    let result = build_with_reader(&f.0, &f.0, &paths, &limits, |p, cap| {
        fs::write(p, b"1234")?;
        read_bounded_regular(p, cap)
    });
    assert!(result
        .unwrap_err()
        .iter()
        .any(|d| d.code == "bundle_bytes_limit"));
}

#[test]
fn manifest_snapshot_file_count_rejected_before_reading_payloads() {
    let f = Fixture::new();
    let paths = [f.file("run.json", b"a"), f.file("coverage.json", b"b")];
    let limits = Limits {
        bundle_files: 1,
        ..Limits::default()
    };
    let mut reads = 0;
    let result = build_with_reader(&f.0, &f.0, &paths, &limits, |p, _| {
        reads += 1;
        fs::read(p)
    });
    assert!(result
        .unwrap_err()
        .iter()
        .any(|d| d.code == "bundle_files_limit"));
    assert_eq!(
        reads, 0,
        "file count must be checked before allocations or reads"
    );
}

#[test]
fn manifest_snapshot_duplicate_normalized_paths_are_rejected() {
    let f = Fixture::new();
    let path = f.file("run.json", b"{}");
    let paths = [path, f.0.join(".").join("run.json")];
    let result = build(&f.0, &f.0, &paths, &Limits::default());
    assert!(result
        .unwrap_err()
        .iter()
        .any(|d| d.code == "duplicate_manifest_path"));
}

#[test]
fn manifest_snapshot_read_error_never_becomes_an_empty_artifact() {
    let f = Fixture::new();
    let paths = [f.file("run.json", b"{}")];
    let result = build_with_reader(&f.0, &f.0, &paths, &Limits::default(), |p, cap| {
        fs::remove_file(p)?;
        read_bounded_regular(p, cap)
    });
    assert!(result.unwrap_err().iter().any(|d| d.code == "unreadable"));
}

#[test]
fn manifest_snapshot_bound_is_on_read_bytes_not_only_metadata() {
    let f = Fixture::new();
    let path = f.file("large.bin", &[b'x'; 1024]);
    assert_eq!(read_bounded_regular(&path, 7).unwrap().len(), 8);
    assert_eq!(read_bounded_regular(&path, 0).unwrap().len(), 1);
    let empty = f.file("empty.bin", b"");
    assert!(read_bounded_regular(&empty, 0).unwrap().is_empty());
}

#[test]
fn manifest_snapshot_canonical_paths_preserve_identity_without_control_delimiters() {
    let root = Path::new("/manifest-root");
    assert!(normalize_relative(root, &root.join("line\nfeed.json")).is_none());
    assert!(normalize_relative(root, &root.join("tab\tname.json")).is_none());
    assert_eq!(
        normalize_relative(root, &root.join("cafe\u{301}.json")),
        normalize_relative(root, &root.join("caf\u{e9}.json"))
    );
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let name = std::ffi::OsString::from_vec(vec![0xff, b'.', b'j']);
        assert!(normalize_relative(root, &root.join(name)).is_none());
    }
}

#[cfg(unix)]
#[test]
fn manifest_snapshot_replacement_symlink_and_fifo_are_not_read() {
    use std::os::unix::ffi::OsStrExt;
    let f = Fixture::new();
    let path = f.file("run.json", b"{}");
    let outside = f.file("unselected.json", b"not-authorized");
    let result = build_with_reader(
        &f.0,
        &f.0,
        std::slice::from_ref(&path),
        &Limits::default(),
        |p, cap| {
            fs::remove_file(p)?;
            std::os::unix::fs::symlink(&outside, p)?;
            read_bounded_regular(p, cap)
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"not-authorized");
    let fifo = f.0.join("pipe.json");
    let raw = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // A test-owned FIFO must be rejected before a blocking open.
    assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
    assert!(read_bounded_regular(&fifo, 1024).is_err());
    assert!(read_bounded_regular(&f.0, 1024).is_err());
}
