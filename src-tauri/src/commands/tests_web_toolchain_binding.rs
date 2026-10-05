fn toolchain_test_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!("oviraptor-toolchain-binding-{}",Uuid::new_v4()));
    fs::create_dir(&path).unwrap();
    fs::canonicalize(path).unwrap()
}

#[test]
fn web_toolchain_streaming_digest_preserves_sha256_schema_and_byte_budget() {
    let root = toolchain_test_directory();
    let candidate = root.join("digest-input");
    let mut inputs = vec![Vec::new(), b"abc".to_vec()];
    // SHA-256 padding/block boundaries and the inventory's 64 KiB read boundary.
    for size in [1, 55, 56, 63, 64, 65, 65_535, 65_536, 65_537, 131_089] {
        inputs.push((0..size).map(|index| (index % 256) as u8).collect());
    }
    for bytes in inputs {
        fs::write(&candidate, &bytes).unwrap();
        let expected = format!("{:x}", Sha256::digest(&bytes));
        if bytes.is_empty() {
            assert_eq!(expected, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        } else if bytes == b"abc" {
            assert_eq!(expected, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        }
        for surplus in [0, 17] {
            let mut remaining = bytes.len() as u64 + surplus;
            let binding = web_tool_candidate_binding(&candidate, &mut remaining).unwrap();
            assert_eq!(binding, serde_json::json!({
                "path": candidate.to_str().unwrap(), "state": "present",
                "resolvedPath": candidate.to_str().unwrap(),
                "stamp": web_tool_file_stamp(&fs::metadata(&candidate).unwrap()).unwrap(),
                "sha256": expected,
            }));
            assert_eq!(remaining, surplus);
        }
        if !bytes.is_empty() {
            let mut remaining = bytes.len() as u64 - 1;
            assert_eq!(web_tool_candidate_binding(&candidate, &mut remaining).unwrap_err(),
                "web_binding_tool_unsafe_or_oversized");
            assert_eq!(remaining, bytes.len() as u64 - 1);
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_toolchain_inventory_detects_install_edit_remove_without_executing() {
    let root = toolchain_test_directory();
    let candidate = root.join("node");
    let candidates = vec![candidate.clone()];
    let missing = web_toolchain_candidates_binding(&candidates,&[]).unwrap();
    assert_eq!(missing["nodeCandidates"][0]["state"],"missing");
    // This executable fixture must be read, never invoked by inventory.
    fs::write(&candidate,format!("#!/bin/sh\ntouch '{}'\n",root.join("executed").display())).unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&candidate,fs::Permissions::from_mode(0o700)).unwrap();
    }
    let installed = web_toolchain_candidates_binding(&candidates,&[]).unwrap();
    assert_ne!(installed,missing);
    assert_eq!(installed,web_toolchain_candidates_binding(&candidates,&[]).unwrap());
    assert!(!root.join("executed").exists());
    let mut content = fs::read(&candidate).unwrap();
    content[0] = b'x';
    fs::write(&candidate,&content).unwrap();
    let edited = web_toolchain_candidates_binding(&candidates,&[]).unwrap();
    assert_ne!(edited["nodeCandidates"][0]["sha256"],installed["nodeCandidates"][0]["sha256"]);
    fs::remove_file(candidate).unwrap();
    assert_eq!(web_toolchain_candidates_binding(&candidates,&[]).unwrap(),missing);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_toolchain_enumeration_keeps_missing_overrides_and_stable_deduplication() {
    let root = toolchain_test_directory();
    let path = std::env::join_paths([&root,&root]).unwrap();
    let override_path = root.join("override-node");
    let candidates = helper_node_candidates(&path,Some(override_path.clone().into_os_string()));
    assert_eq!(candidates[0],override_path);
    assert_eq!(candidates.iter().filter(|p| **p == root.join(if cfg!(windows) {"node.exe"} else {"node"})).count(),1);
    #[cfg(not(windows))] {
        assert!(candidates.contains(&PathBuf::from("/opt/homebrew/bin/node")));
        assert!(candidates.contains(&PathBuf::from("/usr/local/bin/node")));
        assert!(candidates.contains(&PathBuf::from("/usr/bin/node")));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_toolchain_browser_locations_match_shared_platform_inventory() {
    let shared: JsonValue = serde_json::from_str(include_str!("../../resources/config/browser-locations.json")).unwrap();
    for platform in ["darwin","linux"] {
        let expected: Vec<PathBuf> = shared[platform].as_array().unwrap().iter()
            .map(|item| PathBuf::from(item.as_str().unwrap())).collect();
        assert_eq!(web_browser_candidates(platform, |_| None).unwrap(),expected);
        let override_path = expected[0].clone();
        let actual = web_browser_candidates(platform,|key| (key=="OVIRAPTOR_BROWSER_EXECUTABLE")
            .then(|| override_path.clone().into_os_string())).unwrap();
        assert_eq!(actual,expected);
    }
    let windows = web_browser_candidates("win32",|key| match key {
        "PROGRAMFILES" | "PROGRAMFILES(X86)" => Some(OsString::from("C:/Program Files")),
        "LOCALAPPDATA" => Some(OsString::from("C:/User/Local")),
        _ => None,
    }).unwrap();
    assert_eq!(windows.len(),6);
    assert_eq!(windows[0],PathBuf::from("C:/Program Files").join("Google/Chrome/Application/chrome.exe"));
    assert_eq!(windows[3],PathBuf::from("C:/User/Local").join("Google/Chrome/Application/chrome.exe"));
    assert!(web_browser_candidates("win32", |_| None).unwrap().is_empty());
}

#[test]
fn web_toolchain_refuses_relative_special_oversized_and_excessive_inputs() {
    let root = toolchain_test_directory();
    for path in [PathBuf::from("relative-node"),root.clone()] {
        assert!(web_toolchain_candidates_binding(&[path],&[]).is_err());
    }
    let oversized = root.join("oversized");
    File::create(&oversized).unwrap().set_len(WEB_TOOLCHAIN_FILE_LIMIT+1).unwrap();
    assert_eq!(web_toolchain_candidates_binding(&[oversized],&[]).unwrap_err(),"web_binding_tool_unsafe_or_oversized");
    assert_eq!(web_toolchain_candidates_binding(&vec![root.join("missing");257],&[]).unwrap_err(),"web_binding_too_many_tool_candidates");
    let ordinary = root.join("small");
    fs::write(&ordinary,b"small").unwrap();
    assert!(web_tool_candidate_binding(&ordinary,&mut 4).is_err());
    #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;
        let fifo = root.join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(),0o600) },0);
        assert!(web_toolchain_candidates_binding(&[fifo],&[]).is_err());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn web_toolchain_symlink_retarget_and_leaf_replacement_change_binding() {
    use std::os::unix::fs::symlink;
    let root = toolchain_test_directory();
    let one = root.join("one");
    let two = root.join("two");
    let link = root.join("node");
    fs::write(&one,b"same bytes").unwrap();
    fs::write(&two,b"same bytes").unwrap();
    symlink(&one,&link).unwrap();
    let candidates = vec![link.clone()];
    let before = web_toolchain_candidates_binding(&candidates,&[]).unwrap();
    fs::remove_file(&link).unwrap();
    symlink(&two,&link).unwrap();
    let retargeted = web_toolchain_candidates_binding(&candidates,&[]).unwrap();
    assert_ne!(before,retargeted);
    fs::rename(&one,&two).unwrap();
    assert_ne!(retargeted,web_toolchain_candidates_binding(&candidates,&[]).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_toolchain_changes_invalidate_private_dispatch_receipt_without_a_claim() {
    let (root,_,mut connection) = web_start_fixture();
    let candidate = fs::canonicalize(&root).unwrap().join("test-node");
    fs::write(&candidate,b"first runtime").unwrap();
    let candidates = vec![candidate.clone()];
    let mut descriptor = binding_test_runtime();
    descriptor["toolchain"] = web_toolchain_candidates_binding(&candidates,&[]).unwrap();
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut startup = prepare_binding_test_startup(&transaction,&root);
    register_web_dispatch_binding_in(&transaction,&startup,&descriptor).unwrap();
    startup.files.preserve = true;
    transaction.commit().unwrap();
    assert!(binding_test_verify(&mut connection,&startup.files.path,&descriptor).is_ok());
    fs::write(&candidate,b"other runtime").unwrap();
    descriptor["toolchain"] = web_toolchain_candidates_binding(&candidates,&[]).unwrap();
    assert!(binding_test_verify(&mut connection,&startup.files.path,&descriptor).is_err());
    let claim: String = connection.query_row("SELECT claim_id FROM native_branch_dispatches WHERE scan_id='start-test'",[],|r|r.get(0)).unwrap();
    assert!(claim.is_empty());
    let status: String = connection.query_row("SELECT status FROM sentinel_scans WHERE id='start-test'",[],|r|r.get(0)).unwrap();
    assert_eq!(status,"scanning");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
