//! Builds diffy-syntax grammar packs and their signed index.
//!
//! ```text
//! syntax-pack build [LANGUAGE...] [--target TRIPLE] [--out DIR] [--sources DIR] [--work DIR]
//!     Fetch each language's pinned source (grammars.toml), check its hash,
//!     compile the parser into a shared library for TRIPLE (default: this
//!     host), and write the pack to <out>/<triple>/<language>/. With no
//!     languages, builds them all. --sources DIR reads <DIR>/<language>/
//!     instead of fetching (offline and Nix builds); the hash still applies.
//!     Packs for this host are then checked as `check` does.
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
use std::sync::mpsc;
use std::time::{Duration, Instant};

use diffy_syntax::pack::{self, PackFile, PackManifest, PackSource};
use diffy_syntax::{GrammarStore, LanguageId, LanguageStatus, StoreConfig};
use serde::Deserialize;
use serde_json::json;
use tree_sitter as ts;
use tree_sitter::StreamingIterator;

const HOST: &str = env!("SYNTAX_PACK_HOST");
const KEY_VAR: &str = "DIFFY_SYNTAX_SIGNING_KEY";
const LOCK: &str = include_str!("../grammars.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grammar {
    repo: String,
    rev: String,
    version: String,
    #[serde(default = "default_src")]
    src: String,
    symbol: Option<String>,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    highlights: Vec<String>,
    #[serde(default)]
    injections: Vec<String>,
    sha256: String,
    /// SPDX expression for the grammar and its queries.
    license: String,
    scanner: Scanner,
    /// Representative source, relative to this crate.
    sample: String,
    /// `[text, capture]`: the first occurrence of `text` in the sample must
    /// be exactly the range of a capture named `capture` or `capture.*`.
    captures: Vec<(String, String)>,
    /// Languages the injection query must name for the sample.
    #[serde(default)]
    injects: Vec<String>,
}

/// The external scanner a grammar's `src` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Scanner {
    /// `scanner.c`, compiled beside `parser.c`.
    C,
    None,
}

fn default_src() -> String {
    "src".to_owned()
}

type Lock = BTreeMap<String, Grammar>;

fn parse_lock(text: &str) -> Result<Lock, String> {
    let lock: Lock = toml::from_str(text).map_err(|e| format!("grammars.toml: {e}"))?;
    validate_lock(&lock)?;
    Ok(lock)
}

/// Checks what the build cannot: every name, alias, and extension means
/// one language (a store resolves a tag to whichever pack claims it
/// first), tags are lowercase as fence lookups are, each grammar names
/// something to check, and each expected injection reaches a pinned pack.
fn validate_lock(lock: &Lock) -> Result<(), String> {
    let mut owners: BTreeMap<&str, &str> = BTreeMap::new();
    for (language, grammar) in lock {
        let tags = std::iter::once(language)
            .chain(&grammar.aliases)
            .chain(&grammar.extensions);
        for tag in tags {
            if *tag != tag.to_ascii_lowercase() {
                return Err(format!("{language}: tag {tag:?} is not lowercase"));
            }
            if let Some(owner) = owners.insert(tag, language)
                && owner != language
            {
                return Err(format!("{tag:?} is claimed by both {owner} and {language}"));
            }
        }
        if grammar.captures.is_empty() {
            return Err(format!("{language}: no expected captures"));
        }
    }
    for (language, grammar) in lock {
        if let Some(missing) = grammar
            .injects
            .iter()
            .find(|tag| !owners.contains_key(tag.as_str()))
        {
            return Err(format!(
                "{language}: expected injection {missing:?} is no pinned language"
            ));
        }
    }
    Ok(())
}

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
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            language => options.languages.push(language.to_owned()),
        }
    }
    Ok(options)
}

fn run(args: &[String]) -> Result<(), String> {
    let lock = parse_lock(LOCK)?;
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
            for language in &options.languages {
                build(&lock, language, &options)?;
            }
            if options.target == HOST {
                check(&lock, &options)?;
            }
            Ok(())
        }
        Some("check") => check(&lock, &options()?),
        Some("index") => index(&lock, &options()?),
        Some("verify") => verify(&lock, &options()?),
        Some("remote-check") => remote_check(&lock, &options()?),
        Some("public-key") => {
            let key =
                quark_update::manifest::public_key(&signing_seed()?).map_err(|e| e.to_string())?;
            println!("{}", key.to_hex());
            Ok(())
        }
        Some("list") => {
            for (language, grammar) in &lock {
                println!(
                    "{language} {} {} {}@{}",
                    grammar.version, grammar.license, grammar.repo, grammar.rev
                );
            }
            Ok(())
        }
        _ => Err(
            "usage: syntax-pack build|check|index|verify|remote-check|public-key|list \
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
        .ok_or_else(|| format!("{language} is not in grammars.toml"))?;
    if let Some(sources) = &options.sources {
        return Ok(sources.join(language));
    }
    let dir = options
        .work
        .join("sources")
        .join(format!("{language}-{}", grammar.rev));
    if git(&dir, &["rev-parse", "HEAD"]).is_ok_and(|head| head.trim() == grammar.rev) {
        return Ok(dir);
    }
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    eprintln!("fetching {language} {} at {}", grammar.repo, grammar.rev);
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
            &grammar.repo,
            &grammar.rev,
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
/// `@<language>/<path>` names a file in another entry's checkout.
fn resolve_input(
    lock: &Lock,
    root: &Path,
    name: &str,
    options: &Options,
) -> Result<(String, PathBuf), String> {
    match name.strip_prefix('@') {
        Some(rest) => {
            let (language, path) = rest
                .split_once('/')
                .ok_or_else(|| format!("bad input {name}"))?;
            Ok((
                name.to_owned(),
                checkout(lock, language, options)?.join(path),
            ))
        }
        None => Ok((name.to_owned(), root.join(name))),
    }
}

/// Every C source and header under `src`, relative to the repository.
fn source_files(root: &Path, src: &str) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let mut stack = vec![PathBuf::from(src)];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(root.join(&dir)).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let rel = dir.join(entry.file_name());
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

fn build(lock: &Lock, language: &str, options: &Options) -> Result<(), String> {
    let grammar = lock
        .get(language)
        .ok_or_else(|| format!("{language} is not in grammars.toml"))?;
    let root = checkout(lock, language, options)?;

    let mut inputs = Vec::new();
    for name in source_files(&root, &grammar.src)?
        .iter()
        .chain(&grammar.include)
        .chain(&grammar.highlights)
        .chain(&grammar.injections)
    {
        inputs.push(resolve_input(lock, &root, name, options)?);
    }
    let hash = source_hash(&inputs)?;
    if hash != grammar.sha256 {
        return Err(format!(
            "{language}: sources hash to {hash}, grammars.toml pins {:?}",
            grammar.sha256
        ));
    }

    let src = root.join(&grammar.src);
    let parser_c = fs::read_to_string(src.join("parser.c"))
        .map_err(|e| format!("{language}: parser.c: {e}"))?;
    let abi = parser_c
        .lines()
        .find_map(|line| line.strip_prefix("#define LANGUAGE_VERSION "))
        .and_then(|v| v.trim().parse::<u32>().ok())
        .ok_or_else(|| format!("{language}: no LANGUAGE_VERSION in parser.c"))?;
    if src.join("scanner.cc").exists() {
        return Err(format!("{language}: C++ scanners are not supported"));
    }
    let has_scanner = src.join("scanner.c").exists();
    if has_scanner != (grammar.scanner == Scanner::C) {
        return Err(format!(
            "{language}: grammars.toml says scanner = {:?}, but {} scanner.c",
            grammar.scanner,
            if has_scanner {
                "src has a"
            } else {
                "src has no"
            }
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
        has_scanner,
        target,
        &objects,
        &out.join(&library_name),
    )?;

    let write_query = |name: &str, files: &[String]| -> Result<Option<PackFile>, String> {
        if files.is_empty() {
            return Ok(None);
        }
        let mut text = String::new();
        for file in files {
            let (_, path) = resolve_input(lock, &root, file, options)?;
            text.push_str(&read_text(&path)?);
            text.push('\n');
        }
        fs::write(out.join(name), &text).map_err(|e| e.to_string())?;
        Ok(Some(pack_file(&out, name)?))
    };
    let highlights = write_query("highlights.scm", &grammar.highlights)?
        .ok_or_else(|| format!("{language}: no highlight query"))?;
    let injections = write_query("injections.scm", &grammar.injections)?;

    let manifest = PackManifest {
        schema: pack::SCHEMA,
        language: language.to_owned(),
        aliases: grammar.aliases.clone(),
        extensions: grammar.extensions.clone(),
        version: grammar.version.clone(),
        target: target.clone(),
        abi,
        symbol: grammar
            .symbol
            .clone()
            .unwrap_or_else(|| format!("tree_sitter_{language}")),
        library: pack_file(&out, &library_name)?,
        highlights,
        injections,
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

/// Compiles parser.c, and scanner.c when `scanner`, in `src` into the
/// shared library `output` for `target`.
fn compile(
    src: &Path,
    scanner: bool,
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
    if scanner {
        build.file(src.join("scanner.c"));
    }
    let compiler = build.try_get_compiler().map_err(|e| e.to_string())?;
    if !compiler.is_like_msvc() {
        // parser.c marks only the language function visible, so this keeps
        // every other symbol out of the library's export table.
        build.flag("-fvisibility=hidden").flag("-std=c11");
    }
    let objs = build
        .try_compile_intermediates()
        .map_err(|e| e.to_string())?;

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
    let status = link.status().map_err(|e| format!("linker: {e}"))?;
    if !status.success() {
        return Err(format!("linking {} failed: {status}", output.display()));
    }
    Ok(())
}

// ---- Check --------------------------------------------------------------

fn check(lock: &Lock, options: &Options) -> Result<(), String> {
    for language in &options.languages {
        check_language(lock, language, &options.out.join(HOST).join(language))?;
    }
    Ok(())
}

/// Loads the pack in `dir` as the store would, then on its own: parses the
/// sample, which must parse without errors, and requires the expected
/// captures (a capture of the name that starts at some occurrence of the
/// text and covers it) and injected languages.
fn check_language(lock: &Lock, language: &str, dir: &Path) -> Result<(), String> {
    let grammar = lock
        .get(language)
        .ok_or_else(|| format!("{language} is not in grammars.toml"))?;
    let fail = |what: &dyn std::fmt::Display| format!("{language}: {what}");
    let manifest = pack::check_pack_dir(dir).map_err(|e| fail(&format!("does not load: {e}")))?;
    let ts_language = load_language(dir, &manifest).map_err(|e| fail(&e))?;
    let sample = read_text(&Path::new(env!("CARGO_MANIFEST_DIR")).join(&grammar.sample))?;

    let mut parser = ts::Parser::new();
    parser
        .set_language(&ts_language)
        .map_err(|e| fail(&e.to_string()))?;
    let tree = parser
        .parse(&sample, None)
        .ok_or_else(|| fail(&"the parser gave up"))?;
    if tree.root_node().has_error() {
        return Err(fail(&format!(
            "{} has parse errors: {}",
            grammar.sample,
            tree.root_node().to_sexp()
        )));
    }

    let highlights = ts::Query::new(
        &ts_language,
        &read_text(&dir.join(&manifest.highlights.path))?,
    )
    .map_err(|e| fail(&format!("highlight query: {e}")))?;
    let captured = captures(&highlights, &tree, &sample);
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

    let injected = match &manifest.injections {
        Some(file) => {
            let query = ts::Query::new(&ts_language, &read_text(&dir.join(&file.path))?)
                .map_err(|e| fail(&format!("injection query: {e}")))?;
            injected_languages(&query, &tree, &sample)
        }
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

/// The pack's language function, loaded directly. The library stays
/// loaded for the rest of the process, as in diffy-syntax.
fn load_language(dir: &Path, manifest: &PackManifest) -> Result<ts::Language, String> {
    let path = dir.join(&manifest.library.path);
    let mut symbol = manifest.symbol.clone().into_bytes();
    symbol.push(0);
    // SAFETY: the pack is the one just built from pinned, hashed sources
    // (or downloaded and SHA-checked by `check_pack_dir`), and its symbol
    // is the tree-sitter language function the manifest names.
    unsafe {
        let library = libloading::Library::new(&path).map_err(|e| e.to_string())?;
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
    parse_hex32(&text, &format!("${KEY_VAR}"))
}

/// Requires `languages` to be exactly the pinned set, so an index built
/// from a subset of languages can never replace the complete one.
fn check_complete<'a>(
    lock: &Lock,
    languages: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let found: BTreeSet<&str> = languages.into_iter().collect();
    let pinned: BTreeSet<&str> = lock.keys().map(String::as_str).collect();
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
            {
                file.url = Some(file_url(base, target, &language, file));
            }
        }
        packs.push(manifest);
    }
    check_complete(lock, packs.iter().map(|p| p.language.as_str()))?;
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
    check_complete(lock, index.packs.iter().map(|p| p.language.as_str()))?;
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
        let grammar = lock
            .get(language)
            .ok_or_else(|| format!("{language} is not in grammars.toml"))?;
        let id = LanguageId::from_fence(language).ok_or_else(|| format!("bad tag {language}"))?;
        let deadline = Instant::now() + REMOTE_TIMEOUT;
        loop {
            match store.status(&id) {
                LanguageStatus::Ready => break,
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
        let sample = read_text(&Path::new(env!("CARGO_MANIFEST_DIR")).join(&grammar.sample))?;
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

        let error = parse_lock(&text).unwrap_err();

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
        let lock = parse_lock(&(entry("c", "") + &entry("rust", "\"rs\""))).unwrap();
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
}
