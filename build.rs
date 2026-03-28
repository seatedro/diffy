#[cfg(feature = "qt")]
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=src/app/surface/qt_raster_backend.cpp");
    println!("cargo:rerun-if-changed=src/app/surface/qt_raster_backend.hpp");

    let qt_include_path =
        std::env::var("DEP_QT_INCLUDE_PATH").expect("DEP_QT_INCLUDE_PATH must be set");
    let qt_compile_flags = std::env::var("DEP_QT_COMPILE_FLAGS")
        .expect("DEP_QT_COMPILE_FLAGS must be set")
        .split_terminator(';')
        .filter(|flag| !flag.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    let mut config = cpp_build::Config::new();
    for flag in &qt_compile_flags {
        config.flag(flag);
    }

    config
        .include(&qt_include_path)
        .include(format!("{qt_include_path}/QtCore"))
        .include(format!("{qt_include_path}/QtGui"))
        .include(format!("{qt_include_path}/QtQuick"))
        .include("src")
        .build("src/lib.rs");

    let mut cc_build = cc::Build::new();
    cc_build.cpp(true);
    for flag in &qt_compile_flags {
        cc_build.flag(flag);
    }
    cc_build
        .include(&qt_include_path)
        .include(format!("{qt_include_path}/QtCore"))
        .include(format!("{qt_include_path}/QtGui"))
        .include(format!("{qt_include_path}/QtQuick"))
        .include("src")
        .file("src/app/surface/qt_raster_backend.cpp")
        .compile("diffy_qt_raster_backend");
}

#[cfg(not(feature = "qt"))]
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
