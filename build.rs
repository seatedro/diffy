#[cfg(feature = "qt")]
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");

    let qt_include_path =
        std::env::var("DEP_QT_INCLUDE_PATH").expect("DEP_QT_INCLUDE_PATH must be set");

    let mut config = cpp_build::Config::new();
    for flag in std::env::var("DEP_QT_COMPILE_FLAGS")
        .expect("DEP_QT_COMPILE_FLAGS must be set")
        .split_terminator(';')
    {
        config.flag(flag);
    }

    config
        .include(&qt_include_path)
        .include(format!("{qt_include_path}/QtCore"))
        .build("src/lib.rs");
}

#[cfg(not(feature = "qt"))]
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
