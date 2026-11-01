#[allow(dead_code)]
#[path = "src/scanning/fingerprint_logic.rs"]
mod fingerprint_logic;
#[path = "build/fingerprint_resources.rs"]
mod fingerprint_resources;

fn main() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let directory = root.join("resources/fingerprint-packs");
    println!("cargo:rerun-if-changed={}", directory.display());
    println!("cargo:rerun-if-changed=build/fingerprint_resources.rs");
    println!("cargo:rerun-if-changed=src/scanning/fingerprint_logic.rs");
    let count = fingerprint_resources::verify(&directory)
        .unwrap_or_else(|error| panic!("Bundled fingerprint validation failed: {error}"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let packs = manifest["packs"].as_array().unwrap();
    let mut indexed = std::collections::HashSet::new();
    let mut generated = String::from(
        "pub(crate) const BUNDLED_FINGERPRINT_PACKS: &[(&str, &str, &str, usize, &str)] = &[\n",
    );
    for pack in packs {
        let file = pack["file"].as_str().unwrap();
        assert!(
            std::path::Path::new(file).components().count() == 1
                && file.ends_with(".fingerprints.json"),
            "Indexed packs must be flat *.fingerprints.json files"
        );
        indexed.insert(file.to_string());
        let path = directory.join(file).canonicalize().unwrap();
        generated.push_str(&format!(
            "({:?}, include_str!({:?}), {:?}, {}, {:?}),\n",
            file,
            path.to_str().unwrap(),
            pack["sha256"].as_str().unwrap(),
            pack["count"].as_u64().unwrap(),
            pack["ruleSha256"].as_str().unwrap()
        ));
    }
    // The packaging glob cannot silently collect an unindexed rule/CVE file.
    for entry in std::fs::read_dir(&directory).unwrap() {
        let entry = entry.unwrap();
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.ends_with(".fingerprints.json") {
            assert!(
                indexed.contains(&file),
                "Unindexed fingerprint resource: {file}"
            );
        }
    }
    generated.push_str("];\n");
    let output = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap())
        .join("fingerprint_pack_resources.rs");
    if std::fs::read_to_string(&output).ok().as_deref() != Some(&generated) {
        std::fs::write(output, generated).unwrap();
    }
    println!("cargo:warning=Verified {count} bundled fingerprint defaults (no database access)");
    tauri_build::build()
}
