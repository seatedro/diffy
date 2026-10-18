use std::path::PathBuf;

use diffy_syntax::testing::legacy as fixture;
use diffy_syntax::{GrammarStore, LanguageId, LegacyPacks, StoreConfig};
use phosphor::LanguageId as PhosphorLanguage;

const SAMPLES: &[(PhosphorLanguage, &str)] = &[
    (
        PhosphorLanguage::Bash,
        "greet() {\n  local name=\"$1\"\n  echo \"hi $name\" # note\n}\n",
    ),
    (
        PhosphorLanguage::C,
        "#include <stdio.h>\nstatic int add(int a, int b) { return a + b; }\n",
    ),
    (
        PhosphorLanguage::Cpp,
        "namespace demo {\nclass Box { public: int size() const { return 1; } };\n}\n",
    ),
    (
        PhosphorLanguage::Go,
        "package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Println(42) }\n",
    ),
    (
        PhosphorLanguage::JavaScript,
        "const add = (a, b) => a + b;\nexport default add;\n",
    ),
    (
        PhosphorLanguage::Json,
        "{\"name\": \"demo\", \"count\": 3, \"ok\": true}\n",
    ),
    (
        PhosphorLanguage::Nix,
        "{ pkgs ? import <nixpkgs> {} }:\npkgs.mkShell { buildInputs = [ pkgs.git ]; }\n",
    ),
    (
        PhosphorLanguage::Python,
        "def add(a, b):\n    return a + b  # sum\n",
    ),
    (
        PhosphorLanguage::Rust,
        "fn main() {\n    let total: u32 = 1 + 2;\n    if total > 2 { return; }\n}\n",
    ),
    (
        PhosphorLanguage::Toml,
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    ),
    (
        PhosphorLanguage::TypeScript,
        "interface Item { id: number }\nexport function first(items: Item[]): Item | undefined { return items[0]; }\n",
    ),
    (
        PhosphorLanguage::TypeScriptTsx,
        "export const View = () => <div className=\"row\">{1}</div>;\n",
    ),
    (
        PhosphorLanguage::Zig,
        "const std = @import(\"std\");\npub fn main() void {\n    std.debug.print(\"hi\", .{});\n}\n",
    ),
];

const NATIVE: &[(&str, &str)] = &[
    ("bash", "bash"),
    ("c", "c"),
    ("cpp", "cpp"),
    ("go", "go"),
    ("javascript", "javascript"),
    ("json", "json"),
    ("python", "python"),
    ("rust", "rust"),
    ("toml", "toml"),
    ("typescript", "typescript"),
    ("tsx", "tsx"),
];

type Span = (usize, usize, String);

fn render(spans: &[Span], source: &str) -> String {
    spans
        .iter()
        .map(|(start, end, kind)| format!("{kind}:{}", &source[*start..*end]))
        .collect::<Vec<_>>()
        .join(" ")
}

fn inside(inner: &Span, outer: &Span) -> bool {
    outer.0 <= inner.0 && inner.1 <= outer.1
}

fn refines(phosphor: &[Span], legacy: &[Span]) -> bool {
    legacy
        .iter()
        .all(|span| phosphor.iter().any(|outer| inside(span, outer)))
        && phosphor
            .iter()
            .all(|outer| legacy.iter().any(|span| inside(span, outer)))
}

fn legacy_store() -> Option<(PathBuf, GrammarStore, Option<tempfile::TempDir>)> {
    if let (Some(dir), Some(key)) = (
        std::env::var_os("DIFFY_LEGACY_PARITY_STORAGE"),
        std::env::var("DIFFY_LEGACY_PARITY_KEY").ok(),
    ) {
        let key = diffy_syntax::PublicKey::from_hex(&key).ok()?;
        let dir = PathBuf::from(dir);
        let store = GrammarStore::new(
            StoreConfig::new().legacy_packs(LegacyPacks::new(dir.clone(), &[key])),
        );
        return Some((dir, store, None));
    }
    let packs = fixture::native_packs(NATIVE)?;
    let temp = tempfile::tempdir().ok()?;
    fixture::install_signed(temp.path(), &packs)?;
    let store = fixture::cached_store(temp.path())?;
    Some((temp.path().to_path_buf(), store, Some(temp)))
}

#[test]
fn legacy_grammars_highlight_like_phosphor() {
    let Some((storage, store, _temp)) = legacy_store() else {
        return;
    };
    phosphor::pack::override_storage_dir(storage);
    let highlighter = phosphor::Highlighter::new();
    let (mut exact, mut refined, mut diverged) = (Vec::new(), Vec::new(), Vec::new());
    for (language, source) in SAMPLES {
        if !highlighter.is_parser_available(*language) {
            eprintln!("skipping {language}: no legacy pack in the parity storage");
            continue;
        }
        let phosphor: Vec<Span> = highlighter
            .highlight_language(*language, source)
            .unwrap_or_default()
            .into_iter()
            .filter(|span| span.kind != phosphor::HighlightKind::Normal)
            .map(|span| {
                let start = span.offset as usize;
                let kind = format!("{:?}", span.kind).to_ascii_lowercase();
                (start, start + span.length as usize, kind)
            })
            .collect();
        let tag = LanguageId::from_fence(language.name()).unwrap();
        let legacy: Vec<Span> = diffy_syntax::highlight(&store, &tag, source)
            .into_iter()
            .map(|span| {
                let range = span.range();
                (range.start, range.end, span.kind.name().to_owned())
            })
            .collect();
        if phosphor == legacy {
            exact.push(language.name());
        } else if refines(&phosphor, &legacy) {
            refined.push(language.name());
        } else {
            diverged.push(format!(
                "{language}\n  phosphor: {}\n  legacy:   {}",
                render(&phosphor, source),
                render(&legacy, source)
            ));
        }
    }
    eprintln!(
        "exact: {}; nested captures refined: {}",
        exact.join(", "),
        refined.join(", ")
    );
    assert!(diverged.is_empty(), "{}", diverged.join("\n"));
}
