// The tool's own target is the host it compiles packs on.
fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=SYNTAX_PACK_HOST={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
