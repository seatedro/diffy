use std::fs;
use std::path::Path;

#[test]
fn repo_is_native_only() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));

    assert!(collect_files(&repo_root.join("src/app")).is_empty());
    assert!(collect_files(&repo_root.join("qml")).is_empty());
    assert!(!repo_root.join("build.rs").exists());

    let cargo_toml = fs::read_to_string(repo_root.join("Cargo.toml")).unwrap();
    assert!(!cargo_toml.contains("qmetaobject"));
    assert!(!cargo_toml.contains("qttypes"));
    assert!(!cargo_toml.contains("cpp_build"));
    assert!(!cargo_toml.contains("qtquick"));

    let main_rs = fs::read_to_string(repo_root.join("src/main.rs")).unwrap();
    assert!(!main_rs.contains("QmlEngine"));
    assert!(!main_rs.contains("qml_register_type"));
}

fn collect_files(root: &Path) -> Vec<std::path::PathBuf> {
    if !root.exists() {
        return Vec::new();
    }

    let mut files = Vec::new();
    let entries = fs::read_dir(root).unwrap();
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}
