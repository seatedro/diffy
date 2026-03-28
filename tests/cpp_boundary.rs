use std::fs;
use std::path::{Path, PathBuf};

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).unwrap();
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn cpp_macros_are_confined_to_the_qt_surface_bridge() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo_root.join("src");
    let mut rs_files = Vec::new();
    collect_rs_files(&src_root, &mut rs_files);

    let mut cpp_macro_files = rs_files
        .into_iter()
        .filter_map(|path| {
            let contents = fs::read_to_string(&path).unwrap();
            contents.contains("cpp!(").then(|| {
                path.strip_prefix(repo_root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
        })
        .collect::<Vec<_>>();
    cpp_macro_files.sort();

    assert_eq!(cpp_macro_files, vec!["src/app/surface/item.rs"]);
    assert!(
        repo_root
            .join("src/app/surface/qt_raster_backend.cpp")
            .exists()
    );
    assert!(
        repo_root
            .join("src/app/surface/qt_raster_backend.hpp")
            .exists()
    );
}
