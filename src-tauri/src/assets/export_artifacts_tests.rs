use super::super::owned_work::test_support::Fixture;
use super::*;

fn artifact(f: &Fixture, bytes: &[u8]) -> (PathBuf, Registered) {
    let dir = project_directory(&f.exports, 9001).unwrap();
    let path = dir.join(format!("synthetic-{}.csv", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path).unwrap().write_all(bytes).unwrap();
    let registered = register(&f.path, &f.exports, 9001, &path, 1, "assets").unwrap();
    (path, registered)
}

#[test]
fn bounded_chunks_reconstruct_exact_export_bytes_with_one_fixed_hash() {
    let f = Fixture::new();
    let source = (0..400_123).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    let (_, registered) = artifact(&f, &source);
    let mut offset = 0;
    let mut bytes = Vec::new();
    loop {
        let chunk = read(
            &f.path,
            &f.exports,
            9001,
            &registered.artifact_id,
            offset,
            MAX_CHUNK,
        )
        .unwrap();
        assert_eq!(chunk.as_object().unwrap().len(), 7);
        assert_eq!(chunk["offset"], offset);
        assert_eq!(chunk["sha256"], registered.sha256);
        assert_eq!(chunk["totalBytes"], source.len());
        let part = STANDARD
            .decode(chunk["bytesBase64"].as_str().unwrap())
            .unwrap();
        assert!(part.len() <= MAX_CHUNK);
        bytes.extend(part);
        if chunk["done"] == true {
            assert_eq!(chunk["nextOffset"], source.len());
            break;
        }
        let next = chunk["nextOffset"].as_u64().unwrap();
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(bytes, source);
    assert_eq!(registered.sha256, format!("{:x}", Sha256::digest(&bytes)));
}

#[test]
fn chunk_range_unknown_receipt_and_arbitrary_files_are_rejected() {
    let f = Fixture::new();
    let (_, registered) = artifact(&f, b"abc");
    for (offset, size) in [(0, 0), (0, MAX_CHUNK + 1), (4, 1), (MAX_SAFE + 1, 1)] {
        assert!(read(
            &f.path,
            &f.exports,
            9001,
            &registered.artifact_id,
            offset,
            size
        )
        .is_err());
    }
    assert!(read(
        &f.path,
        &f.exports,
        9001,
        &uuid::Uuid::new_v4().to_string(),
        0,
        1
    )
    .is_err());
    assert!(resolve(&f.path, &f.exports, 9001, "/arbitrary.csv").is_err());
    let end = read(&f.path, &f.exports, 9001, &registered.artifact_id, 3, 1).unwrap();
    assert_eq!(end["done"], true);
    assert_eq!(end["bytesBase64"], "");
}

#[test]
fn receipt_is_bound_to_project_and_native_store_instance() {
    let f = Fixture::new();
    let (_, registered) = artifact(&f, b"owned");
    project_directory(&f.exports, 9002).unwrap();
    assert!(read(&f.path, &f.exports, 9002, &registered.artifact_id, 0, 10).is_err());
    let g = Fixture::new();
    assert!(read(&g.path, &f.exports, 9001, &registered.artifact_id, 0, 10).is_err());
    assert!(register(
        &f.path,
        &f.exports,
        9002,
        &project_directory(&f.exports, 9001)
            .unwrap()
            .join("not-owned.csv"),
        0,
        "assets"
    )
    .is_err());
}

#[test]
fn changed_same_length_export_is_refused_before_any_chunk_is_returned() {
    let f = Fixture::new();
    let (path, registered) = artifact(&f, b"before");
    fs::write(&path, b"AFTER!").unwrap();
    assert!(read(&f.path, &f.exports, 9001, &registered.artifact_id, 0, 10).is_err());
}

#[test]
fn expired_or_tampered_receipt_cannot_authorize_readback() {
    let f = Fixture::new();
    let (_, registered) = artifact(&f, b"owned");
    let dir = project_directory(&f.exports, 9001).unwrap();
    let path = receipt_path(&dir, &registered.artifact_id, false).unwrap();
    let mut receipt: Receipt = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    receipt.created_at = 1;
    receipt.expires_at = 1 + TTL;
    fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert_eq!(
        read(&f.path, &f.exports, 9001, &registered.artifact_id, 0, 10).unwrap_err(),
        "asset_artifact_expired"
    );
    receipt.project_id = 9002;
    fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert!(read(&f.path, &f.exports, 9001, &registered.artifact_id, 0, 10).is_err());
}

#[test]
#[cfg(unix)]
fn symlink_hardlink_and_replaced_exports_are_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let (path, registered) = artifact(&f, b"owned");
    let hard = path.with_extension("hard");
    fs::hard_link(&path, &hard).unwrap();
    assert!(read(&f.path, &f.exports, 9001, &registered.artifact_id, 0, 10).is_err());
    fs::remove_file(&hard).unwrap();
    let saved = path.with_extension("saved");
    fs::rename(&path, &saved).unwrap();
    symlink(&saved, &path).unwrap();
    assert!(read(&f.path, &f.exports, 9001, &registered.artifact_id, 0, 10).is_err());
    fs::remove_file(&path).unwrap();
    fs::copy(&saved, &path).unwrap();
    assert!(read(&f.path, &f.exports, 9001, &registered.artifact_id, 0, 10).is_err());
}

#[test]
fn legacy_export_dto_is_wrapped_without_exposing_private_path() {
    let f = Fixture::new();
    let dir = project_directory(&f.exports, 9001).unwrap();
    let path = dir.join("wrapped.csv");
    fs::write(&path, b"a,b\n1,2\n").unwrap();
    let wrapped = wrap_result_on(
        &f.path,
        9001,
        &f.exports,
        json!({"path":path.to_str().unwrap(),"rows":1}),
    )
    .unwrap();
    assert_eq!(wrapped["rows"], 1);
    assert!(wrapped["path"].as_str().unwrap().starts_with(PREFIX));
    assert!(!wrapped.to_string().contains(f.root.to_str().unwrap()));
    let chunk = read(
        &f.path,
        &f.exports,
        9001,
        wrapped["artifactId"].as_str().unwrap(),
        0,
        1024,
    )
    .unwrap();
    assert_eq!(
        STANDARD
            .decode(chunk["bytesBase64"].as_str().unwrap())
            .unwrap(),
        b"a,b\n1,2\n"
    );
}
