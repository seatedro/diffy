use std::path::{Path, PathBuf};

use crate::engine::{self, Engine};
use crate::render::render;

const OLD: &str = "<<<< old";
const NEW: &str = "<<<< new";
const EXPECT: &str = "<<<< expect";

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures"))]
    dir: PathBuf,
    #[arg(long, value_delimiter = ',', default_value = "difftastic,native")]
    engines: Vec<Engine>,
    #[arg(long)]
    bless: Option<Engine>,
    #[arg(long, requires = "bless")]
    force: bool,
    filter: Option<String>,
}

struct Fixture {
    path: PathBuf,
    name: String,
    old: String,
    new: String,
    expect: Vec<String>,
}

pub fn run(args: Args) -> Result<(), String> {
    let fixtures = load(&args.dir, args.filter.as_deref())?;
    if let Some(engine) = args.bless {
        let mut blessed = 0;
        for fixture in &fixtures {
            if !args.force && fixture.expect.iter().any(|e| !e.trim().is_empty()) {
                continue;
            }
            blessed += 1;
            let actual = output(engine, fixture)?;
            let text = format!(
                "{OLD}\n{}{NEW}\n{}{EXPECT}\n{actual}",
                fixture.old, fixture.new
            );
            std::fs::write(&fixture.path, text).map_err(|error| error.to_string())?;
        }
        println!("blessed {blessed} fixtures with {}", engine.name());
        return Ok(());
    }
    let mut failed = 0;
    for &engine in &args.engines {
        let mut passed = 0;
        for fixture in &fixtures {
            let actual = output(engine, fixture)?;
            if fixture
                .expect
                .iter()
                .any(|e| e.trim_end() == actual.trim_end())
            {
                passed += 1;
                continue;
            }
            failed += 1;
            println!("FAIL {} {}", engine.name(), fixture.name);
            for expect in &fixture.expect {
                println!("expected:\n{}", expect.trim_end());
            }
            println!("actual:\n{}\n", actual.trim_end());
        }
        println!(
            "{}: {passed}/{} fixtures match",
            engine.name(),
            fixtures.len()
        );
    }
    if failed > 0 {
        return Err(format!("{failed} fixture mismatches"));
    }
    Ok(())
}

fn output(engine: Engine, fixture: &Fixture) -> Result<String, String> {
    let run = engine::run(
        engine,
        &fixture.name,
        fixture.old.as_bytes(),
        fixture.new.as_bytes(),
    )?;
    Ok(render(&run.doc, run.fallback.as_deref()))
}

fn load(dir: &Path, filter: Option<&str>) -> Result<Vec<Fixture>, String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .filter(|path| filter.is_none_or(|f| path.to_string_lossy().contains(f)))
        .collect();
    paths.sort();
    paths.into_iter().map(|path| parse(&path)).collect()
}

fn parse(path: &Path) -> Result<Fixture, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned();
    let mut sections = vec![String::new(), String::new()];
    let mut current = None;
    for line in text.lines() {
        match line {
            OLD => current = Some(0),
            NEW => current = Some(1),
            EXPECT => {
                sections.push(String::new());
                current = Some(sections.len() - 1);
            }
            _ => {
                let Some(index) = current else {
                    return Err(format!("{}: text before {OLD}", path.display()));
                };
                sections[index].push_str(line);
                sections[index].push('\n');
            }
        }
    }
    let expect = sections.split_off(2);
    let [old, new] = <[String; 2]>::try_from(sections).unwrap_or_default();
    Ok(Fixture {
        path: path.to_owned(),
        name,
        old,
        new,
        expect,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_engine_matches_every_fixture() {
        if engine::grammar("x.rs").is_none() {
            return;
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let fixtures = load(&dir, None).unwrap();
        let failing: Vec<&str> = fixtures
            .iter()
            .filter(|fixture| {
                let actual = output(Engine::Native, fixture).unwrap();
                !fixture
                    .expect
                    .iter()
                    .any(|e| e.trim_end() == actual.trim_end())
            })
            .map(|fixture| fixture.name.as_str())
            .collect();
        assert!(failing.is_empty(), "fixtures read differently: {failing:?}");
    }
}
