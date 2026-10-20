// Packs are built per target triple, and Rust has no runtime constant for
// the full triple (`std::env::consts` lacks the vendor and ABI parts that
// tell gnu from musl, or msvc from gnu), so record the one this build is for.
fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=DIFFY_SYNTAX_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
