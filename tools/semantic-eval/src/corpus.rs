use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::capture::{self, RECORD_PREFIX};
use crate::check::{self, Metrics};
use crate::engine::{self, Engine};
use crate::report;

const PACK_EXTENSIONS: &[&str] = &[
    "bash", "c", "cc", "cjs", "cpp", "cts", "cxx", "go", "h", "hh", "hpp", "js", "json", "jsx",
    "mjs", "mts", "nix", "py", "rs", "sh", "toml", "ts", "tsx",
];
const BINARY_SCAN_BYTES: usize = 8000;
const PROGRESS_STRIDE: usize = 50;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long, default_value = "HEAD")]
    rev: String,
    #[arg(long, default_value_t = 200)]
    commits: usize,
    #[arg(long, value_delimiter = ',')]
    ext: Vec<String>,
    #[arg(long, value_delimiter = ',', default_value = "difftastic,native,line")]
    engines: Vec<Engine>,
    #[arg(long)]
    max_files: Option<usize>,
    #[arg(long)]
    phases: bool,
    #[arg(long)]
    jsonl: Option<PathBuf>,
    #[arg(long, default_value_t = 10)]
    slowest: usize,
    #[arg(long, hide = true)]
    emit_records: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub engine: Engine,
    pub commit: String,
    pub path: String,
    pub ext: String,
    pub old_bytes: u64,
    pub new_bytes: u64,
    pub micros: u64,
    pub cold: bool,
    pub structural: bool,
    pub fallback: Option<String>,
    pub error: Option<String>,
    pub metrics: Metrics,
    pub violations: Vec<String>,
    #[serde(default)]
    pub phases: Vec<(String, f64)>,
    #[serde(default)]
    pub renames: u32,
    #[serde(default)]
    pub moves: u32,
    #[serde(default)]
    pub reflows: u32,
}

pub fn run(args: Args) -> Result<(), String> {
    let records = if args.phases && !args.emit_records {
        collect_with_phases()?
    } else {
        let mut records = Vec::new();
        collect(&args, |record| {
            if args.emit_records {
                let json = serde_json::to_string(&record).unwrap_or_default();
                eprintln!("{RECORD_PREFIX}{json}");
            } else {
                records.push(record);
            }
        })?;
        records
    };
    if args.emit_records {
        return Ok(());
    }
    if let Some(path) = &args.jsonl {
        write_jsonl(path, &records)?;
    }
    report::print(&records, args.slowest);
    let broken = records.iter().filter(|r| !r.violations.is_empty()).count();
    if broken > 0 {
        return Err(format!("{broken} files break an invariant"));
    }
    Ok(())
}

fn collect(args: &Args, mut emit: impl FnMut(Record)) -> Result<(), String> {
    let commits = commits(&args.repo, &args.rev, args.commits)?;
    let mut blobs = Blobs::spawn(&args.repo)?;
    let mut warm = HashSet::new();
    let mut files = 0usize;
    for (index, (commit, parent)) in commits.iter().enumerate() {
        for (old_path, new_path) in changes(&args.repo, parent, commit)? {
            let ext = extension(&new_path);
            let wanted = if args.ext.is_empty() {
                PACK_EXTENSIONS.contains(&ext.as_str())
            } else {
                args.ext.contains(&ext)
            };
            if !wanted || args.max_files.is_some_and(|max| files >= max) {
                continue;
            }
            let (Some(old), Some(new)) = (
                blobs.read(parent, &old_path)?,
                blobs.read(commit, &new_path)?,
            ) else {
                continue;
            };
            if old == new || binary(&old) || binary(&new) {
                continue;
            }
            files += 1;
            if files.is_multiple_of(PROGRESS_STRIDE) {
                eprintln!("{files} files, commit {}/{}", index + 1, commits.len());
            }
            for &engine in &args.engines {
                let cold = warm.insert((engine, ext.clone()));
                emit(measure(engine, commit, &new_path, &ext, &old, &new, cold));
            }
        }
    }
    Ok(())
}

fn measure(
    engine: Engine,
    commit: &str,
    path: &str,
    ext: &str,
    old: &[u8],
    new: &[u8],
    cold: bool,
) -> Record {
    let mut record = Record {
        engine,
        commit: commit.to_owned(),
        path: path.to_owned(),
        ext: ext.to_owned(),
        old_bytes: old.len() as u64,
        new_bytes: new.len() as u64,
        micros: 0,
        cold,
        structural: false,
        fallback: None,
        error: None,
        metrics: Metrics::default(),
        violations: Vec::new(),
        phases: Vec::new(),
        renames: 0,
        moves: 0,
        reflows: 0,
    };
    match engine::run(engine, path, old, new) {
        Ok(run) => {
            let (metrics, violations) = check::check(&run.doc);
            record.micros = run.micros;
            record.structural = run.doc.file_count() > 0 && run.doc.prefers_structural(0);
            record.fallback = run.fallback;
            record.metrics = metrics;
            record.violations = violations;
            if let Some(semantics) = run.doc.semantics(0) {
                record.renames = semantics.renames().len() as u32;
                record.moves = semantics.moves().len() as u32;
            }
            record.reflows = reflows(&run.doc);
        }
        Err(error) => record.error = Some(error),
    }
    record
}

fn reflows(doc: &diffy_diff::DiffDocument) -> u32 {
    use diffy_diff::{Comparison, ComparisonOptions, Expansion, Mode, Projection, RowKind};
    if doc.file_count() == 0 || !doc.prefers_structural(0) {
        return 0;
    }
    let comparison = Comparison::new(doc, ComparisonOptions::default());
    let p = Projection::with_comparison(doc, Mode::Unified, &Expansion::new(doc), &comparison);
    (0..p.len())
        .filter(|&r| p.reflow(r).is_some() && p.kind[r as usize] == RowKind::Reflowed)
        .count() as u32
}

fn collect_with_phases() -> Result<Vec<Record>, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let args = std::env::args_os()
        .skip(1)
        .filter(|arg| arg != "--phases")
        .chain([std::ffi::OsString::from("--emit-records")]);
    let mut child = Command::new(exe)
        .args(args)
        .env("DFT_TIMINGS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stderr = child.stderr.take().ok_or("child has no stderr")?;
    let mut records = Vec::new();
    let mut phases = Vec::new();
    for line in BufReader::new(stderr).lines() {
        let line = line.map_err(|error| error.to_string())?;
        if let Some(phase) = capture::parse_phase(&line) {
            phases.push(phase);
        } else if let Some(json) = line.strip_prefix(RECORD_PREFIX) {
            let mut record: Record = serde_json::from_str(json).map_err(|e| e.to_string())?;
            record.phases = std::mem::take(&mut phases);
            records.push(record);
        } else {
            eprintln!("{line}");
        }
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("measuring child exited with {status}"));
    }
    Ok(records)
}

fn write_jsonl(path: &Path, records: &[Record]) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut out = BufWriter::new(file);
    for record in records {
        serde_json::to_writer(&mut out, record).map_err(|error| error.to_string())?;
        out.write_all(b"\n").map_err(|error| error.to_string())?;
    }
    out.flush().map_err(|error| error.to_string())
}

fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_SCAN_BYTES)].contains(&0)
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(output.stdout)
}

fn commits(repo: &Path, rev: &str, count: usize) -> Result<Vec<(String, String)>, String> {
    let limit = format!("-n{count}");
    let out = git(
        repo,
        &[
            "log",
            "--first-parent",
            "--no-merges",
            &limit,
            "--format=%H %P",
            rev,
        ],
    )?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
        })
        .collect())
}

fn changes(repo: &Path, parent: &str, commit: &str) -> Result<Vec<(String, String)>, String> {
    let out = git(repo, &["diff", "--name-status", "-z", "-M", parent, commit])?;
    let text = String::from_utf8_lossy(&out);
    let mut fields = text.split('\0').filter(|field| !field.is_empty());
    let mut pairs = Vec::new();
    while let Some(status) = fields.next() {
        match status.as_bytes().first() {
            Some(b'R' | b'C') => {
                let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                    break;
                };
                pairs.push((old.to_owned(), new.to_owned()));
            }
            Some(b'M') => {
                if let Some(path) = fields.next() {
                    pairs.push((path.to_owned(), path.to_owned()));
                }
            }
            _ => _ = fields.next(),
        }
    }
    pairs.retain(|(old, new)| !old.contains('\n') && !new.contains('\n'));
    Ok(pairs)
}

pub struct Blobs {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Blobs {
    pub fn spawn(repo: &Path) -> Result<Self, String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?;
        let input = child.stdin.take().ok_or("cat-file has no stdin")?;
        let output = BufReader::new(child.stdout.take().ok_or("cat-file has no stdout")?);
        Ok(Self {
            child,
            input,
            output,
        })
    }

    pub fn read(&mut self, rev: &str, path: &str) -> Result<Option<Vec<u8>>, String> {
        let io = |error: std::io::Error| error.to_string();
        writeln!(self.input, "{rev}:{path}").map_err(io)?;
        self.input.flush().map_err(io)?;
        let mut header = String::new();
        self.output.read_line(&mut header).map_err(io)?;
        let mut parts = header.split_whitespace();
        let (Some(_), Some(kind), Some(size)) = (parts.next(), parts.next(), parts.next()) else {
            return Ok(None);
        };
        let size: usize = size
            .parse()
            .map_err(|_| format!("bad cat-file header {header:?}"))?;
        let mut bytes = vec![0; size + 1];
        self.output.read_exact(&mut bytes).map_err(io)?;
        bytes.truncate(size);
        Ok((kind == "blob").then_some(bytes))
    }
}

impl Drop for Blobs {
    fn drop(&mut self) {
        _ = self.child.kill();
        _ = self.child.wait();
    }
}
