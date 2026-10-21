//! Builds diffy-syntax grammar packs and their signed index.
//!
//! ```text
//! syntax-pack build [LANGUAGE...] [--target TRIPLE] [--out DIR] [--sources DIR] [--work DIR] [--jobs N]
//!     Fetch each language's pinned source (grammars.toml, registry.toml),
//!     check its hash, compile the parser into a shared library for TRIPLE
//!     (default: this host), and write the pack to <out>/<triple>/<language>/.
//!     With no languages, builds them all. --sources DIR reads
//!     <DIR>/<language>/ instead of fetching (offline and Nix builds); the
//!     hash still applies. Packs for this host are then checked as `check`
//!     does. A registry language that fails is left out and recorded in
//!     <out>/<triple>/skipped.json; a grammars.toml language fails the build.
//!     Grammars marked `generate` need tree-sitter CLI 0.26.8 ($TREE_SITTER).
//! syntax-pack registry --registry FILE [--extensions FILE] [--jobs N]
//!     Pin every grammar in nvim-treesitter's registry.json that is not in
//!     grammars.toml, build and check each for this host, and rewrite
//!     registry.toml. Extensions come from extensions.toml.
//! syntax-pack check [LANGUAGE...] [--out DIR]
//!     Load each built pack for this host, parse its sample from samples/,
//!     and require the captures and injected languages grammars.toml lists.
//!     Packs for another target need a syntax-pack built for that target
//!     (`cargo run --target x86_64-apple-darwin` runs under Rosetta).
//! syntax-pack index [--target TRIPLE] [--out DIR] [--base-url URL]
//!     Sign <out>/<triple>/index.json listing every pack under
//!     <out>/<triple>/, with the Ed25519 seed in $DIFFY_SYNTAX_SIGNING_KEY
//!     (64 hex digits; make one with
//!     `cargo run -p quark-update --example manifest_tool keygen`). Every
//!     pinned language must be present. With --base-url, each file gets the
//!     immutable URL <URL>/<triple>/<language>/<sha256>/<file>.
//! syntax-pack verify --key HEX --base-url URL [--target TRIPLE] [--out DIR]
//!     Check <out>/<triple>/index.json before publishing it: it verifies
//!     against HEX, lists every pinned language, its files match, and each
//!     URL is the immutable one under URL. Prints `<file>\t<url>\t<sha256>`
//!     for each file, the list a publisher uploads.
//! syntax-pack remote-check --index URL --key HEX [LANGUAGE...] [--work DIR]
//!     Download this host's packs from a published index through
//!     diffy-syntax's own downloader and highlight each sample.
//! syntax-pack public-key
//!     Print the public key for $DIFFY_SYNTAX_SIGNING_KEY, for the app.
//! syntax-pack list
//!     Print the pinned languages.
//! ```
//!
//! `--out` defaults to `<workspace>/target/syntax-packs`, which quark's
//! tests and examples read; `--work` (source checkouts and objects) to
//! `<workspace>/target/syntax-pack-work`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use diffy_syntax::pack::{self, PackFile, PackManifest, PackSource};
use diffy_syntax::{GrammarStore, LanguageId, LanguageStatus, StoreConfig};
use serde_json::json;
use tree_sitter as ts;
use tree_sitter::StreamingIterator;

mod lock;
mod query;
mod registry;

use lock::{Lock, Scanner, Source};

const HOST: &str = env!("SYNTAX_PACK_HOST");
const KEY_VAR: &str = "DIFFY_SYNTAX_SIGNING_KEY";
const TREE_SITTER_CLI: &str = "0.26.8";
const TREE_SITTER_VAR: &str = "TREE_SITTER";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("syntax-pack: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    languages: Vec<String>,
    target: String,
    out: PathBuf,
    work: PathBuf,
    sources: Option<PathBuf>,
    base_url: Option<String>,
    key: Option<String>,
    index_url: Option<String>,
    registry: Option<PathBuf>,
    extensions: Option<PathBuf>,
    jobs: usize,
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        languages: Vec::new(),
        target: HOST.to_owned(),
        out: workspace().join("target/syntax-packs"),
        work: workspace().join("target/syntax-pack-work"),
        sources: None,
        base_url: None,
        key: None,
        index_url: None,
        registry: None,
        extensions: None,
        jobs: std::thread::available_parallelism().map_or(4, |n| n.get()),
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--target" => options.target = value()?,
            "--out" => options.out = value()?.into(),
            "--work" => options.work = value()?.into(),
            "--sources" => options.sources = Some(value()?.into()),
            "--base-url" => options.base_url = Some(value()?.trim_end_matches('/').to_owned()),
            "--key" => options.key = Some(value()?),
            "--index" => options.index_url = Some(value()?),
            "--registry" => options.registry = Some(value()?.into()),
            "--extensions" => options.extensions = Some(value()?.into()),
            "--jobs" => {
                options.jobs = value()?
                    .parse()
                    .map_err(|_| "--jobs needs a number".to_owned())?
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            language => options.languages.push(language.to_owned()),
        }
    }
    Ok(options)
}

fn run(args: &[String]) -> Result<(), String> {
    let lock = lock::load()?;
    let options = || -> Result<Options, String> {
        let mut options = parse_options(&args[1..])?;
        if options.languages.is_empty() {
            options.languages = lock.keys().cloned().collect();
        }
        Ok(options)
    };
    match args.first().map(String::as_str) {
        Some("build") => {
            let options = options()?;
            let mut skipped = BTreeMap::new();
            let results = registry::par_map(&options.languages, options.jobs, |language| {
                build(&lock, language, &options).and_then(|()| {
                    if options.target == HOST {
                        check_language(&lock, language, &options)
                    } else {
                        Ok(())
                    }
                })
            });
            for (language, built) in options.languages.iter().zip(results) {
                match built {
                    Ok(()) => {}
                    Err(error) if !lock.is_core(language) => {
                        eprintln!("skipping {language}: {error}");
                        let _ =
                            fs::remove_dir_all(options.out.join(&options.target).join(language));
                        skipped.insert(language.clone(), error);
                    }
                    Err(error) => return Err(error),
                }
            }
            write_skipped(&options, &skipped)
        }
        Some("check") => check(&lock, &options()?),
        Some("index") => index(&lock, &options()?),
        Some("verify") => verify(&lock, &options()?),
        Some("remote-check") => remote_check(&lock, &options()?),
        Some("registry") => registry::generate(&lock, &parse_options(&args[1..])?),
        Some("public-key") => {
            let key =
                quark_update::manifest::public_key(&signing_seed()?).map_err(|e| e.to_string())?;
            println!("{}", key.to_hex());
            Ok(())
        }
        Some("list") => {
            for (language, grammar) in &lock.grammars {
                println!(
                    "{language} {} {} {}@{}",
                    grammar.version,
                    grammar.license.as_deref().unwrap_or("unknown"),
                    grammar.repo,
                    grammar.rev
                );
            }
            Ok(())
        }
        _ => Err(
            "usage: syntax-pack build|check|index|verify|remote-check|registry|public-key|list \
             (see src/main.rs)"
                .to_owned(),
        ),
    }
}

// ---- Sources ------------------------------------------------------------

/// The checkout of `language`'s pinned source.
fn checkout(lock: &Lock, language: &str, options: &Options) -> Result<PathBuf, String> {
    let grammar = lock
        .get(language)
        .ok_or_else(|| format!("{language} is not pinned"))?;
    if let Some(sources) = &options.sources {
        return Ok(sources.join(language));
    }
    let source = Source {
        repo: grammar.repo.clone(),
        rev: grammar.rev.clone(),
    };
    fetch(language, &source, options)
}

fn queries_checkout(lock: &Lock, name: &str, options: &Options) -> Result<PathBuf, String> {
    if !lock.fragments.contains_key(name)
        && lock
            .get(name)
            .is_some_and(|grammar| grammar.queries.is_none())
    {
        return checkout(lock, name, options);
    }
    let source = lock
        .queries_of(name)
        .ok_or_else(|| format!("{name} has no pinned queries"))?;
    if let Some(sources) = &options.sources {
        return Ok(sources.join(format!("{name}-queries")));
    }
    fetch(&format!("{name}-queries"), &source, options)
}

fn fetch(key: &str, source: &Source, options: &Options) -> Result<PathBuf, String> {
    static FETCHING: Mutex<BTreeMap<String, Arc<Mutex<()>>>> = Mutex::new(BTreeMap::new());
    let slot = FETCHING
        .lock()
        .map_err(|_| "fetch lock poisoned".to_owned())?
        .entry(key.to_owned())
        .or_default()
        .clone();
    let _held = slot.lock().map_err(|_| "fetch lock poisoned".to_owned())?;
    let dir = options
        .work
        .join("sources")
        .join(format!("{key}-{}", source.rev));
    if git(&dir, &["rev-parse", "HEAD"]).is_ok_and(|head| head.trim() == source.rev) {
        return Ok(dir);
    }
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    eprintln!("fetching {key} {} at {}", source.repo, source.rev);
    git(&dir, &["init", "--quiet"])?;
    // Windows runners default to core.autocrlf=true; keep the bytes as
    // committed (the hash also normalizes, for --sources trees).
    git(&dir, &["config", "core.autocrlf", "false"])?;
    git(
        &dir,
        &[
            "fetch",
            "--quiet",
            "--depth",
            "1",
            &source.repo,
            &source.rev,
        ],
    )?;
    git(&dir, &["checkout", "--quiet", "--detach", "FETCH_HEAD"])?;
    Ok(dir)
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A file the build reads: its hashed name and its path on disk.
fn resolve_input(
    lock: &Lock,
    language: &str,
    root: &Path,
    name: &str,
    options: &Options,
) -> Result<(String, PathBuf), String> {
    let path = if let Some(path) = name.strip_prefix("q:") {
        queries_checkout(lock, language, options)?.join(path)
    } else if let Some(rest) = name.strip_prefix('@') {
        let (other, path) = rest
            .split_once('/')
            .ok_or_else(|| format!("bad input {name}"))?;
        match other.strip_suffix(":q") {
            Some(other) => queries_checkout(lock, other, options)?.join(path),
            None => checkout(lock, other, options)?.join(path),
        }
    } else {
        root.join(name)
    };
    Ok((name.to_owned(), path))
}

/// Every C source and header under `src`, relative to the repository.
fn source_files(root: &Path, src: &str, generate: bool) -> Result<Vec<String>, String> {
    let generated = |rel: &Path| {
        rel == Path::new(src).join("parser.c")
            || rel.starts_with(Path::new(src).join("tree_sitter"))
    };
    let mut files = Vec::new();
    if generate {
        files.push(format!("{src}/grammar.json"));
    }
    let mut stack = vec![PathBuf::from(src)];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(root.join(&dir)).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let rel = dir.join(entry.file_name());
            if generate && generated(&rel) {
                continue;
            }
            if entry.path().is_dir() {
                stack.push(rel);
            } else if rel
                .extension()
                .is_some_and(|ext| ext == "c" || ext == "h" || ext == "cc")
            {
                files.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(files)
}

fn sha256(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `bytes` with CRLF line endings as LF, so a checkout whose line endings
/// were converted on Windows hashes and ships the same bytes as any other.
/// Every input is text: C sources, headers, and queries.
fn normalize_eol(bytes: Vec<u8>) -> Vec<u8> {
    if !bytes.contains(&b'\r') {
        return bytes;
    }
    let mut out = Vec::with_capacity(bytes.len());
    for (i, &byte) in bytes.iter().enumerate() {
        if !(byte == b'\r' && bytes.get(i + 1) == Some(&b'\n')) {
            out.push(byte);
        }
    }
    out
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    String::from_utf8(normalize_eol(bytes)).map_err(|e| format!("{}: {e}", path.display()))
}

/// SHA-256 over `<name>\0<sha256>\n` for every input, sorted by name, each
/// with LF line endings.
fn source_hash(inputs: &[(String, PathBuf)]) -> Result<String, String> {
    let mut lines = Vec::new();
    for (name, path) in inputs {
        let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        lines.push(format!("{name}\0{}\n", sha256(&normalize_eol(bytes))));
    }
    lines.sort();
    Ok(sha256(lines.concat().as_bytes()))
}

// ---- Build --------------------------------------------------------------

fn inputs(
    lock: &Lock,
    language: &str,
    options: &Options,
) -> Result<Vec<(String, PathBuf)>, String> {
    let grammar = lock
        .get(language)
        .ok_or_else(|| format!("{language} is not pinned"))?;
    let root = checkout(lock, language, options)?;
    let mut inputs = Vec::new();
    for name in source_files(&root, &grammar.src, grammar.generate)?
        .iter()
        .chain(&grammar.include)
        .chain(&grammar.highlights)
        .chain(&grammar.injections)
        .chain(&grammar.locals)
    {
        inputs.push(resolve_input(lock, language, &root, name, options)?);
    }
    Ok(inputs)
}

fn build(lock: &Lock, language: &str, options: &Options) -> Result<(), String> {
    let grammar = lock
        .get(language)
        .ok_or_else(|| format!("{language} is not pinned"))?;
    let root = checkout(lock, language, options)?;
    let hash = source_hash(&inputs(lock, language, options)?)?;
    if hash != grammar.sha256 {
        return Err(format!(
            "{language}: sources hash to {hash}, the lock pins {:?}",
            grammar.sha256
        ));
    }

    let src = if grammar.generate {
        generate_parser(language, &root.join(&grammar.src), options)?
    } else {
        root.join(&grammar.src)
    };
    let parser_c = fs::read_to_string(src.join("parser.c"))
        .map_err(|e| format!("{language}: parser.c: {e}"))?;
    let abi = parser_c
        .lines()
        .find_map(|line| line.strip_prefix("#define LANGUAGE_VERSION "))
        .and_then(|v| v.trim().parse::<u32>().ok())
        .ok_or_else(|| format!("{language}: no LANGUAGE_VERSION in parser.c"))?;
    let found = if src.join("scanner.c").exists() {
        Scanner::C
    } else if src.join("scanner.cc").exists() {
        Scanner::Cc
    } else {
        Scanner::None
    };
    if found != grammar.scanner {
        return Err(format!(
            "{language}: pinned scanner = {:?}, but src has {found:?}",
            grammar.scanner
        ));
    }

    let target = &options.target;
    let out = options.out.join(target).join(language);
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let library_name = match pack::library_suffix(target) {
        ".dll" => format!("tree-sitter-{language}.dll"),
        suffix => format!("libtree-sitter-{language}{suffix}"),
    };
    let objects = options.work.join("objects").join(target).join(language);
    eprintln!("compiling {language} for {target}");
    compile(
        &src,
        grammar.scanner,
        target,
        &objects,
        &out.join(&library_name),
    )?;

    let symbol = grammar
        .symbol
        .clone()
        .unwrap_or_else(|| format!("tree_sitter_{language}"));
    let ts_language = (target == HOST)
        .then(|| load_library(&out.join(&library_name), &symbol))
        .transpose()
        .map_err(|e| format!("{language}: {e}"))?;
    let write_query = |name: &str, files: &[String]| -> Result<Option<PackFile>, String> {
        if files.is_empty() {
            return Ok(None);
        }
        let mut text = String::new();
        for file in files {
            let (_, path) = resolve_input(lock, language, &root, file, options)?;
            text.push_str(&read_text(&path)?);
            text.push('\n');
        }
        if let Some(ts_language) = &ts_language {
            let adapted =
                query::adapt(ts_language, &text).map_err(|e| format!("{language}: {name}: {e}"))?;
            if adapted.removed > 0 {
                eprintln!(
                    "{language}: {name}: dropped {} incompatible patterns",
                    adapted.removed
                );
            }
            text = adapted.text;
        }
        fs::write(out.join(name), &text).map_err(|e| e.to_string())?;
        Ok(Some(pack_file(&out, name)?))
    };
    let highlights = write_query("highlights.scm", &grammar.highlights)?
        .ok_or_else(|| format!("{language}: no highlight query"))?;
    let injections = write_query("injections.scm", &grammar.injections)?;
    let locals = write_query("locals.scm", &grammar.locals)?;
    let semantic = workspace()
        .join("crates/diffy-semantic/queries")
        .join(language)
        .join("semantic.scm");
    let semantic = if semantic.is_file() {
        let text = read_text(&semantic)?;
        if let Some(ts_language) = &ts_language {
            ts::Query::new(ts_language, &text)
                .map_err(|e| format!("{language}: semantic.scm: {e}"))?;
        }
        fs::write(out.join("semantic.scm"), &text).map_err(|e| e.to_string())?;
        Some(pack_file(&out, "semantic.scm")?)
    } else {
        None
    };

    let manifest = PackManifest {
        schema: pack::SCHEMA,
        language: language.to_owned(),
        aliases: grammar.aliases.clone(),
        extensions: grammar.extensions.clone(),
        version: grammar.version.clone(),
        target: target.clone(),
        abi,
        symbol,
        library: pack_file(&out, &library_name)?,
        highlights,
        injections,
        locals,
        semantic,
        source: Some(PackSource {
            repo: grammar.repo.clone(),
            rev: grammar.rev.clone(),
            sha256: hash,
        }),
    };
    manifest
        .validate(target)
        .map_err(|e| format!("{language}: {e}"))?;
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(out.join(pack::MANIFEST), json + "\n").map_err(|e| e.to_string())?;
    println!(
        "{} ({} bytes)",
        out.join(&library_name).display(),
        manifest.library.size
    );
    Ok(())
}

fn pack_file(dir: &Path, name: &str) -> Result<PackFile, String> {
    let bytes = fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
    Ok(PackFile {
        path: name.to_owned(),
        sha256: sha256(&bytes),
        size: bytes.len() as u64,
        url: None,
    })
}

fn generate_parser(language: &str, src: &Path, options: &Options) -> Result<PathBuf, String> {
    let cli = std::env::var_os(TREE_SITTER_VAR).unwrap_or_else(|| "tree-sitter".into());
    let version = Command::new(&cli).arg("--version").output().map_err(|e| {
        format!("{language}: needs tree-sitter CLI {TREE_SITTER_CLI} (${TREE_SITTER_VAR}): {e}")
    })?;
    let version = String::from_utf8_lossy(&version.stdout);
    if version.split_whitespace().nth(1) != Some(TREE_SITTER_CLI) {
        return Err(format!(
            "{language}: needs tree-sitter CLI {TREE_SITTER_CLI}, found {}",
            version.trim()
        ));
    }
    let out = options.work.join("generated").join(language);
    let _ = fs::remove_dir_all(&out);
    copy_dir(src, &out)?;
    eprintln!("generating {language}");
    let status = Command::new(&cli)
        .args(["generate", "--abi", "15", "-o"])
        .arg(&out)
        .arg(out.join("grammar.json"))
        .status()
        .map_err(|e| format!("{language}: tree-sitter generate: {e}"))?;
    if !status.success() {
        return Err(format!("{language}: tree-sitter generate failed: {status}"));
    }
    Ok(out)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        }
    }
    Ok(())
}

/// Compiles parser.c and its scanner in `src` into the shared library
/// `output` for `target`.
fn compile(
    src: &Path,
    scanner: Scanner,
    target: &str,
    objects: &Path,
    output: &Path,
) -> Result<(), String> {
    let _ = fs::remove_dir_all(objects);
    fs::create_dir_all(objects).map_err(|e| e.to_string())?;
    let mut build = cc::Build::new();
    build
        .cargo_metadata(false)
        .cargo_warnings(false)
        .warnings(false)
        .target(target)
        .host(HOST)
        .opt_level(2)
        .debug(false)
        .out_dir(objects)
        .include(src)
        .file(src.join("parser.c"));
    if scanner == Scanner::C {
        build.file(src.join("scanner.c"));
    }
    let compiler = build.try_get_compiler().map_err(|e| e.to_string())?;
    let parser = fs::read(src.join("parser.c")).map_err(|e| format!("parser.c: {e}"))?;
    let marks_export = parser.windows(9).any(|w| w == b"TS_PUBLIC");
    if !compiler.is_like_msvc() {
        build.flag("-std=c11");
        // Newer parser.c marks only the language function visible, so this
        // keeps every other symbol out of the export table; older ones mark
        // nothing and would export nothing.
        if marks_export {
            build.flag("-fvisibility=hidden");
        }
    }
    let mut objs = build
        .try_compile_intermediates()
        .map_err(|e| e.to_string())?;
    if scanner == Scanner::Cc {
        let mut cxx = cc::Build::new();
        cxx.cargo_metadata(false)
            .cargo_warnings(false)
            .warnings(false)
            .cpp(true)
            .target(target)
            .host(HOST)
            .opt_level(2)
            .debug(false)
            .out_dir(objects)
            .include(src)
            .file(src.join("scanner.cc"));
        if !compiler.is_like_msvc() {
            cxx.flag("-fvisibility=hidden");
        }
        objs.extend(cxx.try_compile_intermediates().map_err(|e| e.to_string())?);
    }

    let mut link = if compiler.is_like_msvc() {
        let mut link = cc::windows_registry::find_tool(target, "link.exe")
            .ok_or("link.exe not found; run from a Visual Studio developer shell")?
            .to_command();
        link.args(["/NOLOGO", "/DLL"])
            .arg(format!("/OUT:{}", output.display()))
            .arg(format!("/IMPLIB:{}", objects.join("parser.lib").display()));
        link
    } else {
        let mut link = compiler.to_command();
        if target.contains("apple") {
            link.args(["-dynamiclib", "-Wl,-dead_strip", "-Wl,-x"]);
        } else {
            link.args(["-shared", "-s"]);
        }
        link.arg("-o").arg(output);
        link
    };
    link.args(&objs);
    if scanner == Scanner::Cc && !compiler.is_like_msvc() {
        link.arg(if target.contains("apple") {
            "-lc++"
        } else {
            "-lstdc++"
        });
    }
    let status = link.status().map_err(|e| format!("linker: {e}"))?;
    if !status.success() {
        return Err(format!("linking {} failed: {status}", output.display()));
    }
    Ok(())
}

// ---- Check --------------------------------------------------------------

fn check(lock: &Lock, options: &Options) -> Result<(), String> {
    let skipped = read_skipped(&options.out.join(HOST))?;
    for language in &options.languages {
        if !skipped.contains_key(language) {
            check_language(lock, language, options)?;
        }
    }
    Ok(())
}

fn read_sample(lock: &Lock, language: &str, options: &Options) -> Result<Option<String>, String> {
    let Some(sample) = lock.get(language).and_then(|g| g.sample.as_deref()) else {
        return Ok(None);
    };
    let path = match sample.strip_prefix("q:") {
        Some(path) => queries_checkout(lock, language, options)?.join(path),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join(sample),
    };
    read_text(&path).map(Some)
}

/// Loads the pack for this host as the store would, then on its own: its
/// queries must compile. A core language's sample must parse without
/// errors and yield the expected captures (a capture of the name that
/// starts at some occurrence of the text and covers it) and injected
/// languages; a registry language's sample must yield some capture.
fn check_language(lock: &Lock, language: &str, options: &Options) -> Result<(), String> {
    let grammar = lock
        .get(language)
        .ok_or_else(|| format!("{language} is not pinned"))?;
    let dir = options.out.join(HOST).join(language);
    let fail = |what: &dyn std::fmt::Display| format!("{language}: {what}");
    let manifest = pack::check_pack_dir(&dir).map_err(|e| fail(&format!("does not load: {e}")))?;
    let ts_language = load_language(&dir, &manifest).map_err(|e| fail(&e))?;
    let highlights = ts::Query::new(
        &ts_language,
        &read_text(&dir.join(&manifest.highlights.path))?,
    )
    .map_err(|e| fail(&format!("highlight query: {e}")))?;
    let injections = match &manifest.injections {
        Some(file) => Some(
            ts::Query::new(&ts_language, &read_text(&dir.join(&file.path))?)
                .map_err(|e| fail(&format!("injection query: {e}")))?,
        ),
        None => None,
    };
    for file in [&manifest.locals, &manifest.semantic].into_iter().flatten() {
        ts::Query::new(&ts_language, &read_text(&dir.join(&file.path))?)
            .map_err(|e| fail(&format!("{}: {e}", file.path)))?;
    }
    let Some(sample) = read_sample(lock, language, options)? else {
        println!("{language}: loads, queries compile");
        return Ok(());
    };

    let mut parser = ts::Parser::new();
    parser
        .set_language(&ts_language)
        .map_err(|e| fail(&e.to_string()))?;
    let tree = parser
        .parse(&sample, None)
        .ok_or_else(|| fail(&"the parser gave up"))?;
    let captured = captures(&highlights, &tree, &sample);
    if !lock.is_core(language) {
        if captured.is_empty() {
            return Err(fail(&"the sample highlights nothing"));
        }
        println!(
            "{language}: loads, {} captures in its sample",
            captured.len()
        );
        return Ok(());
    }
    if tree.root_node().has_error() {
        return Err(fail(&format!(
            "{} has parse errors: {}",
            grammar.sample.as_deref().unwrap_or_default(),
            tree.root_node().to_sexp()
        )));
    }
    for (text, capture) in &grammar.captures {
        let matches = |name: &&str| {
            name.strip_prefix(capture.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        };
        // The names of captures that start at an occurrence and cover it.
        let at = |start: usize| -> Vec<&str> {
            captured
                .iter()
                .filter(|(r, _)| r.start == start && r.end >= start + text.len())
                .map(|(_, name)| *name)
                .collect()
        };
        let occurrences: Vec<usize> = sample
            .match_indices(text.as_str())
            .map(|(i, _)| i)
            .collect();
        if !occurrences
            .iter()
            .any(|&start| at(start).iter().any(matches))
        {
            let found: Vec<_> = occurrences
                .iter()
                .map(|&start| (start, at(start)))
                .collect();
            return Err(fail(&format!(
                "expected @{capture} starting at {text:?}; captures at each occurrence: {found:?}"
            )));
        }
    }

    let injected = match &injections {
        Some(query) => injected_languages(query, &tree, &sample),
        None => BTreeSet::new(),
    };
    if let Some(missing) = grammar.injects.iter().find(|l| !injected.contains(*l)) {
        return Err(fail(&format!(
            "the sample should inject {missing:?}, found {injected:?}"
        )));
    }
    println!(
        "{language}: {} captures and {} injections check",
        grammar.captures.len(),
        grammar.injects.len()
    );
    Ok(())
}

const SKIPPED: &str = "skipped.json";

fn read_skipped(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    match fs::read(dir.join(SKIPPED)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{SKIPPED}: {e}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(format!("{SKIPPED}: {error}")),
    }
}

fn write_skipped(options: &Options, skipped: &BTreeMap<String, String>) -> Result<(), String> {
    let dir = options.out.join(&options.target);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(skipped).map_err(|e| e.to_string())?;
    fs::write(dir.join(SKIPPED), json + "\n").map_err(|e| e.to_string())?;
    if !skipped.is_empty() {
        eprintln!("{} languages skipped for {}", skipped.len(), options.target);
    }
    Ok(())
}

/// The pack's language function, loaded directly. The library stays
/// loaded for the rest of the process, as in diffy-syntax.
fn load_language(dir: &Path, manifest: &PackManifest) -> Result<ts::Language, String> {
    load_library(&dir.join(&manifest.library.path), &manifest.symbol)
}

fn load_library(path: &Path, symbol: &str) -> Result<ts::Language, String> {
    let mut symbol = symbol.to_owned().into_bytes();
    symbol.push(0);
    // SAFETY: the pack is the one just built from pinned, hashed sources
    // (or downloaded and SHA-checked by `check_pack_dir`), and its symbol
    // is the tree-sitter language function the manifest names.
    unsafe {
        let library = libloading::Library::new(path).map_err(|e| e.to_string())?;
        let library: &'static libloading::Library = Box::leak(Box::new(library));
        let function = library
            .get::<unsafe extern "C" fn() -> *const ()>(&symbol)
            .map_err(|e| e.to_string())?;
        Ok(ts::Language::new(
            tree_sitter_language::LanguageFn::from_raw(*function),
        ))
    }
}

/// Every capture's byte range and name.
fn captures<'q>(
    query: &'q ts::Query,
    tree: &ts::Tree,
    source: &str,
) -> Vec<(std::ops::Range<usize>, &'q str)> {
    let mut cursor = ts::QueryCursor::new();
    let mut found = Vec::new();
    let mut captures = cursor.captures(query, tree.root_node(), source.as_bytes());
    while let Some((query_match, index)) = captures.next() {
        let capture = query_match.captures[*index];
        found.push((
            capture.node.byte_range(),
            query.capture_names()[capture.index as usize],
        ));
    }
    found
}

/// The languages an injection query names over `source`, through an
/// `@injection.language` capture or `(#set! injection.language ...)`.
fn injected_languages(query: &ts::Query, tree: &ts::Tree, source: &str) -> BTreeSet<String> {
    let capture = query.capture_index_for_name("injection.language");
    let mut cursor = ts::QueryCursor::new();
    let mut found = BTreeSet::new();
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        for c in query_match.captures {
            if Some(c.index) == capture {
                found.insert(source[c.node.byte_range()].to_ascii_lowercase());
            }
        }
        for property in query.property_settings(query_match.pattern_index) {
            if &*property.key == "injection.language"
                && let Some(value) = &property.value
            {
                found.insert(value.to_string());
            }
        }
    }
    found
}

// ---- Index --------------------------------------------------------------

fn parse_hex32(text: &str, what: &str) -> Result<[u8; 32], String> {
    let text = text.trim();
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2).unwrap_or("x"), 16))
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{what} is not hex"))?;
    bytes
        .try_into()
        .map_err(|_| format!("{what} is not 32 bytes"))
}

fn signing_seed() -> Result<[u8; 32], String> {
    let text = std::env::var(KEY_VAR).map_err(|_| format!("${KEY_VAR} is not set"))?;
    if text.contains("OPENSSH PRIVATE KEY") {
        return openssh_seed(&text).map_err(|e| format!("${KEY_VAR}: {e}"));
    }
    parse_hex32(&text, &format!("${KEY_VAR}"))
}

fn openssh_seed(text: &str) -> Result<[u8; 32], String> {
    let body: String = text
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .flat_map(|line| line.trim().chars())
        .collect();
    let bytes = base64_decode(&body)?;
    let mut reader = SshReader { bytes: &bytes };
    if reader.take(15)? != b"openssh-key-v1\0" {
        return Err("not an OpenSSH private key".to_owned());
    }
    let (cipher, kdf) = (reader.string()?, reader.string()?);
    reader.string()?;
    if cipher != b"none" || kdf != b"none" || reader.u32()? != 1 {
        return Err("the key must be a single unencrypted key".to_owned());
    }
    reader.string()?;
    let mut private = SshReader {
        bytes: reader.string()?,
    };
    if private.u32()? != private.u32()? {
        return Err("the key's check integers differ".to_owned());
    }
    if private.string()? != b"ssh-ed25519" {
        return Err("the key is not Ed25519".to_owned());
    }
    private.string()?;
    let secret = private.string()?;
    secret
        .get(..32)
        .and_then(|seed| seed.try_into().ok())
        .ok_or_else(|| "the Ed25519 secret is too short".to_owned())
}

struct SshReader<'a> {
    bytes: &'a [u8],
}

impl<'a> SshReader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        if self.bytes.len() < len {
            return Err("the key is truncated".to_owned());
        }
        let (head, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn string(&mut self) -> Result<&'a [u8], String> {
        let len = self.u32()? as usize;
        self.take(len)
    }
}

fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let value = |c: u8| -> Result<u32, String> {
        Ok(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("{:?} is not base64", c as char)),
        } as u32)
    };
    let digits = text.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let mut word = 0;
        for (i, &c) in chunk.iter().enumerate() {
            word |= value(c)? << (18 - 6 * i);
        }
        out.extend_from_slice(&word.to_be_bytes()[1..chunk.len()]);
    }
    Ok(out)
}

fn check_complete<'a>(
    lock: &Lock,
    skipped: &BTreeMap<String, String>,
    languages: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let found: BTreeSet<&str> = languages.into_iter().collect();
    let pinned: BTreeSet<&str> = lock
        .keys()
        .map(String::as_str)
        .filter(|language| lock.is_core(language) || !skipped.contains_key(*language))
        .collect();
    let missing: Vec<&str> = pinned.difference(&found).copied().collect();
    let extra: Vec<&str> = found.difference(&pinned).copied().collect();
    if !missing.is_empty() || !extra.is_empty() {
        return Err(format!(
            "an index lists every pinned language and nothing else; missing {missing:?}, \
             not pinned {extra:?}"
        ));
    }
    Ok(())
}

/// The immutable URL a file is published at: a changed grammar or query
/// changes its SHA-256 and so its URL, and a cached copy of an old file
/// can never be taken for the new one.
fn file_url(base: &str, target: &str, language: &str, file: &PackFile) -> String {
    format!("{base}/{target}/{language}/{}/{}", file.sha256, file.path)
}

fn https_base(options: &Options) -> Result<Option<&str>, String> {
    match options.base_url.as_deref() {
        Some(base) if !base.starts_with("https://") => {
            Err(format!("--base-url {base} is not an https URL"))
        }
        base => Ok(base),
    }
}

fn index(lock: &Lock, options: &Options) -> Result<(), String> {
    write_index(lock, options, &signing_seed()?)
}

fn write_index(lock: &Lock, options: &Options, seed: &[u8; 32]) -> Result<(), String> {
    let base = https_base(options)?;
    let target = &options.target;
    let dir = options.out.join(target);
    let mut packs = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for pack_dir in dirs {
        let path = pack_dir.join(pack::MANIFEST);
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut manifest: PackManifest =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        manifest
            .validate(target)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        // The index is the only thing signed, so check the files it vouches
        // for are the ones being published.
        pack::verify_files(&pack_dir, &manifest).map_err(|e| e.to_string())?;
        if let Some(base) = base {
            let language = manifest.language.clone();
            for file in [&mut manifest.library, &mut manifest.highlights]
                .into_iter()
                .chain(manifest.injections.as_mut())
                .chain(manifest.locals.as_mut())
                .chain(manifest.semantic.as_mut())
            {
                file.url = Some(file_url(base, target, &language, file));
            }
        }
        packs.push(manifest);
    }
    check_complete(
        lock,
        &read_skipped(&dir)?,
        packs.iter().map(|p| p.language.as_str()),
    )?;
    let payload = json!({
        "schema": pack::SCHEMA,
        "target": target,
        "packs": packs,
    });
    let document = quark_update::manifest::sign(seed, &payload).map_err(|e| e.to_string())?;
    let key = quark_update::manifest::public_key(seed).map_err(|e| e.to_string())?;
    let key = parse_hex32(&key.to_hex(), "the public key")?;
    pack::verify_index(document.as_bytes(), &[key], target)
        .map_err(|e| format!("the signed index does not verify: {e}"))?;
    let path = dir.join("index.json");
    fs::write(&path, document + "\n").map_err(|e| e.to_string())?;
    println!("{} ({} packs)", path.display(), packs.len());
    Ok(())
}

fn verify(lock: &Lock, options: &Options) -> Result<(), String> {
    let key = parse_hex32(options.key.as_deref().ok_or("verify needs --key")?, "--key")?;
    let base = https_base(options)?.ok_or("verify needs --base-url")?;
    let target = &options.target;
    let dir = options.out.join(target);
    let path = dir.join("index.json");
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let index = pack::verify_index(&bytes, &[key], target)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some((language, error)) = index.rejected.first() {
        return Err(format!(
            "{}: {language} is rejected: {error}",
            path.display()
        ));
    }
    check_complete(
        lock,
        &read_skipped(&dir)?,
        index.packs.iter().map(|p| p.language.as_str()),
    )?;
    for manifest in &index.packs {
        let pack_dir = dir.join(&manifest.language);
        pack::verify_files(&pack_dir, manifest).map_err(|e| e.to_string())?;
        for file in manifest.files() {
            let expected = file_url(base, target, &manifest.language, file);
            if file.url.as_deref() != Some(expected.as_str()) {
                return Err(format!(
                    "{}: {} has URL {:?}, expected {expected}",
                    manifest.language, file.path, file.url
                ));
            }
            println!(
                "{}\t{expected}\t{}",
                pack_dir.join(&file.path).display(),
                file.sha256
            );
        }
    }
    Ok(())
}

/// How long one language may take to download before remote-check fails.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(120);

fn remote_check(lock: &Lock, options: &Options) -> Result<(), String> {
    let url = options
        .index_url
        .as_deref()
        .ok_or("remote-check needs --index")?;
    let key = diffy_syntax::PublicKey::from_hex(
        options.key.as_deref().ok_or("remote-check needs --key")?,
    )
    .map_err(|e| format!("--key: {e}"))?;
    // A fresh cache, so every pack comes over the network.
    let cache = options.work.join("remote-check");
    let _ = fs::remove_dir_all(&cache);
    let downloads = diffy_syntax::Downloads::new("syntax-pack", url, &[key]).cache_dir(&cache);
    let store = GrammarStore::new(StoreConfig::new().downloads(downloads));
    let (tx, rx) = mpsc::channel();
    store.subscribe(move || tx.send(()).is_ok());
    for language in &options.languages {
        if lock.get(language).is_none() {
            return Err(format!("{language} is not pinned"));
        }
        let id = LanguageId::from_fence(language).ok_or_else(|| format!("bad tag {language}"))?;
        let deadline = Instant::now() + REMOTE_TIMEOUT;
        loop {
            match store.status(&id) {
                LanguageStatus::Ready => break,
                LanguageStatus::Unavailable if !lock.is_core(language) => {
                    eprintln!("warning {language}: not in the index; skipped on this target?");
                    break;
                }
                LanguageStatus::Unavailable => {
                    return Err(format!(
                        "{language}: not available from {}",
                        url.replace("{target}", pack::TARGET)
                    ));
                }
                LanguageStatus::Pending => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if rx.recv_timeout(left).is_err() {
                        return Err(format!(
                            "{language}: still downloading after {REMOTE_TIMEOUT:?}"
                        ));
                    }
                }
            }
        }
        if store.status(&id) != LanguageStatus::Ready {
            continue;
        }
        let Some(sample) = read_sample(lock, language, options)? else {
            println!("{language}: downloaded");
            continue;
        };
        let spans = diffy_syntax::highlight(&store, &id, &sample);
        if spans.is_empty() {
            return Err(format!(
                "{language}: the downloaded pack highlights nothing"
            ));
        }
        println!("{language}: downloaded, {} spans", spans.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(language: &str, aliases: &str) -> String {
        format!(
            "[{language}]\nrepo = \"r\"\nrev = \"0\"\nversion = \"1\"\naliases = [{aliases}]\n\
             highlights = [\"h.scm\"]\nsha256 = \"0\"\nlicense = \"MIT\"\nscanner = \"none\"\n\
             sample = \"s\"\ncaptures = [[\"a\", \"b\"]]\n"
        )
    }

    // Catches two packs claiming one fence tag, which a store would
    // resolve to whichever it saw first.
    #[test]
    fn an_alias_claimed_by_two_languages_is_rejected() {
        let text = entry("c", "\"h\"") + &entry("cpp", "\"h\", \"cc\"");

        let error = lock::parse(&text, "").err().unwrap();

        assert_eq!(error, "\"h\" is claimed by both c and cpp");
    }

    // Catches a Windows checkout with converted line endings failing the
    // pinned hash.
    #[test]
    fn crlf_sources_hash_like_lf_sources() {
        let dir = tempfile::tempdir().unwrap();
        let (lf, crlf) = (dir.path().join("lf.c"), dir.path().join("crlf.c"));
        fs::write(&lf, "int a;\nint b;\n").unwrap();
        fs::write(&crlf, "int a;\r\nint b;\r\n").unwrap();

        let hash = |path: &Path| source_hash(&[("src/parser.c".to_owned(), path.to_owned())]);

        assert_eq!(hash(&crlf), hash(&lf));
    }

    // Catches a subset build (one language dispatched alone) producing an
    // index that would replace the complete public one.
    #[test]
    fn an_index_missing_a_pinned_language_is_refused() {
        let lock = lock::parse(&(entry("c", "") + &entry("rust", "\"rs\"")), "").unwrap();
        let out = tempfile::tempdir().unwrap();
        let dir = out.path().join(HOST).join("rust");
        fs::create_dir_all(&dir).unwrap();
        let library = format!("libtree-sitter-rust{}", pack::library_suffix(HOST));
        fs::write(dir.join(&library), "library").unwrap();
        fs::write(dir.join("highlights.scm"), "(identifier) @variable").unwrap();
        let manifest = PackManifest {
            schema: pack::SCHEMA,
            language: "rust".to_owned(),
            aliases: Vec::new(),
            extensions: Vec::new(),
            version: "1".to_owned(),
            target: HOST.to_owned(),
            abi: *pack::supported_abi().end(),
            symbol: "tree_sitter_rust".to_owned(),
            library: pack_file(&dir, &library).unwrap(),
            highlights: pack_file(&dir, "highlights.scm").unwrap(),
            injections: None,
            locals: None,
            semantic: None,
            source: None,
        };
        fs::write(
            dir.join(pack::MANIFEST),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let mut options =
            parse_options(&["--out".to_owned(), out.path().display().to_string()]).unwrap();
        options.target = HOST.to_owned();

        let error = write_index(&lock, &options, &[7; 32]).unwrap_err();

        assert_eq!(
            error,
            "an index lists every pinned language and nothing else; missing [\"c\"], \
             not pinned []"
        );
        assert!(!out.path().join(HOST).join("index.json").exists());
    }

    #[test]
    fn an_openssh_key_yields_its_ed25519_seed() {
        let seed = [7u8; 32];
        let public = parse_hex32(
            &quark_update::manifest::public_key(&seed).unwrap().to_hex(),
            "key",
        )
        .unwrap();
        let string = |bytes: &[u8]| [&(bytes.len() as u32).to_be_bytes()[..], bytes].concat();
        let public_blob = [string(b"ssh-ed25519"), string(&public)].concat();
        let private = [
            vec![1, 2, 3, 4, 1, 2, 3, 4],
            string(b"ssh-ed25519"),
            string(&public),
            string(&[&seed[..], &public[..]].concat()),
            string(b""),
        ]
        .concat();
        let blob = [
            b"openssh-key-v1\0".to_vec(),
            string(b"none"),
            string(b"none"),
            string(b""),
            1u32.to_be_bytes().to_vec(),
            string(&public_blob),
            string(&private),
        ]
        .concat();
        let key = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
            base64_encode(&blob)
        );

        assert_eq!(openssh_seed(&key).unwrap(), seed);
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let word = chunk
                .iter()
                .enumerate()
                .fold(0u32, |word, (i, &b)| word | (b as u32) << (16 - 8 * i));
            for i in 0..=chunk.len() {
                out.push(DIGITS[(word >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        while !out.len().is_multiple_of(4) {
            out.push('=');
        }
        out
    }

    #[test]
    fn a_skipped_registry_language_may_be_missing_but_a_core_one_may_not() {
        let mut lock = lock::parse(&entry("c", ""), "").unwrap();
        lock.grammars
            .insert("zig".to_owned(), lock.get("c").unwrap().clone());
        let skipped = BTreeMap::from([("zig".to_owned(), "no parser.c".to_owned())]);

        assert!(check_complete(&lock, &skipped, ["c"]).is_ok());
        assert!(check_complete(&lock, &BTreeMap::new(), ["c"]).is_err());
        let core_skipped = BTreeMap::from([("c".to_owned(), "broken".to_owned())]);
        assert!(check_complete(&lock, &core_skipped, ["zig"]).is_err());
    }
}
