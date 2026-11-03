use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The directory `syntax-pack bundle --out` wrote.
const BUNDLE_VAR: &str = "DIFFY_SYNTAX_BUNDLE";
/// First line of `<bundle>/<target>/bundle.tsv`.
const HEADER: &str = "# diffy-syntax bundle 1";
const QUERIES: [&str; 5] = ["highlights", "injections", "locals", "semantic", "tags"];

// Packs are built per target triple, and Rust has no runtime constant for
// the full triple (`std::env::consts` lacks the vendor and ABI parts that
// tell gnu from musl, or msvc from gnu), so record the one this build is for.
fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=DIFFY_SYNTAX_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_BUNDLED").is_some()
        && let Err(error) = bundle(&target)
    {
        panic!("{BUNDLE_VAR}: {error}");
    }
}

/// Writes `$OUT_DIR/bundled.rs` for `src/bundled.rs` and links the bundle.
fn bundle(target: &str) -> Result<(), String> {
    println!("cargo:rerun-if-env-changed={BUNDLE_VAR}");
    let out_dir = std::env::var_os("OUT_DIR").ok_or("cargo did not set OUT_DIR")?;
    let code = match std::env::var_os(BUNDLE_VAR).filter(|value| !value.is_empty()) {
        None => {
            println!(
                "cargo:warning={BUNDLE_VAR} is not set, so the `bundled` feature compiles no \
                 grammar in; see crates/diffy-syntax/PACKS.md"
            );
            "pub(crate) static GRAMMARS: &[Bundled] = &[];\n".to_owned()
        }
        Some(root) => {
            let root = PathBuf::from(root);
            if !root.is_absolute() {
                return Err(format!("{} is not an absolute path", root.display()));
            }
            generate(&root.join(target), target)?
        }
    };
    let out = Path::new(&out_dir).join("bundled.rs");
    std::fs::write(&out, code).map_err(|e| format!("{}: {e}", out.display()))
}

/// The table for the bundle in `dir`. `bundle.tsv` columns: language,
/// symbol, ABI, library, aliases, extensions, source SHA-256 (lists
/// comma-separated, `-` when empty). Queries: `queries/<language>/<name>.scm`.
fn generate(dir: &Path, target: &str) -> Result<String, String> {
    let manifest = dir.join("bundle.tsv");
    let text = std::fs::read_to_string(&manifest).map_err(|e| {
        format!(
            "{}: {e}; write it with `syntax-pack bundle --target {target}`",
            manifest.display()
        )
    })?;
    println!("cargo:rerun-if-changed={}", manifest.display());
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(format!("{} is not a version 1 bundle", manifest.display()));
    }
    let lib_dir = dir.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    let mut externs = String::new();
    let mut table = String::new();
    let lines = lines.filter(|line| !line.is_empty() && !line.starts_with('#'));
    for (index, line) in lines.enumerate() {
        let bad = |what: &str| format!("{}: {what} in {line:?}", manifest.display());
        let fields: Vec<&str> = line.split('\t').collect();
        let [language, symbol, abi, library, aliases, extensions, source] = fields[..] else {
            return Err(bad("not 7 tab-separated fields"));
        };
        let list = |field: &str| -> Result<Vec<String>, String> {
            if field == "-" {
                return Ok(Vec::new());
            }
            field
                .split(',')
                .map(|tag| {
                    is_tag(tag)
                        .then(|| format!("{tag:?}"))
                        .ok_or_else(|| bad(&format!("bad tag {tag:?}")))
                })
                .collect()
        };
        let (aliases, extensions) = (list(aliases)?, list(extensions)?);
        if !is_tag(language) {
            return Err(bad("bad language"));
        }
        if !is_c_identifier(symbol) {
            return Err(bad("bad symbol"));
        }
        let abi: u32 = abi.parse().map_err(|_| bad("bad ABI"))?;
        if library.is_empty()
            || !library
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(bad("bad library name"));
        }
        if source.len() != 64 || !source.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad("bad source hash"));
        }
        let archive = if target.contains("msvc") {
            lib_dir.join(format!("{library}.lib"))
        } else {
            lib_dir.join(format!("lib{library}.a"))
        };
        if !archive.is_file() {
            return Err(format!("{} is missing", archive.display()));
        }
        println!("cargo:rerun-if-changed={}", archive.display());
        println!("cargo:rustc-link-lib=static={library}");

        // Only the link name comes from the bundle; the Rust name is ours.
        let _ = writeln!(
            externs,
            "    #[link_name = {symbol:?}]\n    fn grammar_{index}() -> *const ();"
        );
        let mut queries = String::new();
        for name in QUERIES {
            let path = dir
                .join("queries")
                .join(language)
                .join(format!("{name}.scm"));
            let value = if path.is_file() {
                println!("cargo:rerun-if-changed={}", path.display());
                let path = path
                    .to_str()
                    .ok_or_else(|| format!("{} is not UTF-8", path.display()))?;
                if name == "highlights" {
                    format!("include_str!({path:?})")
                } else {
                    format!("Some(include_str!({path:?}))")
                }
            } else if name == "highlights" {
                return Err(format!("{} is missing", path.display()));
            } else {
                "None".to_owned()
            };
            let _ = writeln!(queries, "        {name}: {value},");
        }
        let _ = write!(
            table,
            "    Bundled {{\n        name: {language:?},\n        aliases: &[{}],\n        \
             extensions: &[{}],\n        abi: {abi},\n        language: grammar_{index},\n        \
             source: {source:?},\n{queries}    }},\n",
            aliases.join(", "),
            extensions.join(", "),
        );
    }
    Ok(format!(
        "// Generated by build.rs from {:?}.\nunsafe extern \"C\" {{\n{externs}}}\n\n\
         pub(crate) static GRAMMARS: &[Bundled] = &[\n{table}];\n",
        manifest
    ))
}

/// A fence tag or extension, as `LanguageId::from_fence` accepts them.
fn is_tag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'#' | b'.' | b'_' | b'-'))
}

fn is_c_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
