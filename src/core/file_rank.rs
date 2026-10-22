use diffy_diff::FileStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Central,
    Source,
    Config,
    Tests,
    Docs,
    Noise,
}

impl Tier {
    pub fn worth_ranking(self) -> bool {
        self <= Self::Config
    }
}

const LOCKFILES: &[&str] = &[
    "cargo.lock",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "gemfile.lock",
    "poetry.lock",
    "pipfile.lock",
    "uv.lock",
    "composer.lock",
    "go.sum",
    "flake.lock",
    "podfile.lock",
    "pubspec.lock",
    "mix.lock",
    "packages.lock.json",
];

const NOISE_SUFFIXES: &[&str] = &[
    ".lock",
    ".min.js",
    ".min.css",
    ".map",
    ".snap",
    ".pb.go",
    "_pb2.py",
    "_pb2_grpc.py",
    ".pb.h",
    ".pb.cc",
    ".g.dart",
    ".freezed.dart",
    ".designer.cs",
];

const NOISE_DIRS: &[&str] = &[
    "vendor",
    "vendored",
    "node_modules",
    "third_party",
    "third-party",
    "dist",
    "generated",
    "__generated__",
    "__snapshots__",
    "snapshots",
    "fixtures",
    "__fixtures__",
    "testdata",
    "test_data",
    "test-data",
    ".yarn",
];

const ASSET_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "icns", "svg", "webp", "bmp", "tiff", "pdf", "woff",
    "woff2", "ttf", "otf", "eot", "mp3", "mp4", "wav", "ogg", "mov", "zip", "gz", "tar", "jar",
    "wasm", "bin",
];

const TEST_DIRS: &[&str] = &[
    "test",
    "tests",
    "__tests__",
    "__mocks__",
    "spec",
    "specs",
    "e2e",
    "benches",
    "bench",
    "benchmarks",
];

const DOC_DIRS: &[&str] = &[
    "docs",
    "doc",
    "documentation",
    ".docs",
    "examples",
    "example",
];

const DOC_EXTENSIONS: &[&str] = &["md", "mdx", "markdown", "rst", "adoc", "txt"];

const DOC_NAMES: &[&str] = &[
    "license",
    "licence",
    "changelog",
    "authors",
    "contributors",
    "notice",
    "copying",
    "readme",
    "codeowners",
];

const CONFIG_DIRS: &[&str] = &[
    ".github",
    ".gitlab",
    ".circleci",
    ".buildkite",
    ".vscode",
    ".idea",
];

const CONFIG_EXTENSIONS: &[&str] = &[
    "toml",
    "yaml",
    "yml",
    "json",
    "jsonc",
    "ini",
    "cfg",
    "conf",
    "xml",
    "plist",
    "gradle",
    "properties",
    "editorconfig",
    "nix",
];

const CONFIG_NAMES: &[&str] = &[
    "makefile",
    "dockerfile",
    "justfile",
    "rakefile",
    "gemfile",
    "procfile",
    "build.rs",
    "go.mod",
];

const CODE_EXTENSIONS: &[&str] = &[
    "rs", "go", "py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "java", "kt", "kts", "swift",
    "m", "mm", "c", "cc", "cpp", "cxx", "h", "hh", "hpp", "cs", "rb", "php", "scala", "ex", "exs",
    "erl", "hs", "ml", "fs", "lua", "dart", "zig", "vue", "svelte",
];

const ENTRY_STEMS: &[&str] = &[
    "main", "lib", "mod", "index", "app", "__init__", "__main__", "server", "root", "init",
];

const SOURCE_ROOTS: &[&str] = &["", "src", "lib", "app", "pkg", "cmd", "internal", "core"];

pub fn tier(path: &str, binary: bool) -> Tier {
    let lower = path.to_ascii_lowercase();
    let (dir, name) = lower.rsplit_once('/').unwrap_or(("", lower.as_str()));
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, ext),
        _ => (name, ""),
    };
    let in_dir = |names: &[&str]| dir.split('/').any(|segment| names.contains(&segment));
    if binary
        || LOCKFILES.contains(&name)
        || NOISE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
        || name.contains(".generated.")
        || name.contains("_generated.")
        || ASSET_EXTENSIONS.contains(&ext)
        || in_dir(NOISE_DIRS)
    {
        return Tier::Noise;
    }
    let original = path.rsplit('/').next().unwrap_or(path);
    let original = original.split('.').next().unwrap_or(original);
    if in_dir(TEST_DIRS) || is_test_name(name, stem, original) {
        return Tier::Tests;
    }
    if in_dir(DOC_DIRS) || DOC_EXTENSIONS.contains(&ext) || DOC_NAMES.contains(&stem) {
        return Tier::Docs;
    }
    if in_dir(CONFIG_DIRS)
        || CONFIG_EXTENSIONS.contains(&ext)
        || CONFIG_NAMES.contains(&name)
        || name.starts_with('.')
    {
        return Tier::Config;
    }
    let code = CODE_EXTENSIONS.contains(&ext);
    let root = dir.rsplit('/').next().unwrap_or_default();
    if code && (ENTRY_STEMS.contains(&stem) || (SOURCE_ROOTS.contains(&root) && !dir.contains('/')))
    {
        return Tier::Central;
    }
    Tier::Source
}

fn is_test_name(name: &str, stem: &str, original: &str) -> bool {
    matches!(stem, "test" | "tests" | "conftest")
        || stem.starts_with("test_")
        || ["_test", "_tests", "_spec"]
            .iter()
            .any(|s| stem.ends_with(s))
        || ["Test", "Tests", "Spec"]
            .iter()
            .any(|s| original.ends_with(s))
        || name.contains(".test.")
        || name.contains(".spec.")
}

pub fn heuristic_order(files: &[(Tier, &str)]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..files.len() as u32).collect();
    order.sort_by(|&a, &b| {
        let (ta, pa) = files[a as usize];
        let (tb, pb) = files[b as usize];
        ta.cmp(&tb).then_with(|| pa.cmp(pb))
    });
    order
}

pub fn merge_ranked(heuristic: &[u32], ranked: &[u32]) -> Vec<u32> {
    let len = heuristic.len();
    let mut placed = vec![false; len];
    let mut order = Vec::with_capacity(len);
    for &index in ranked {
        if let Some(slot) = placed.get_mut(index as usize)
            && !*slot
        {
            *slot = true;
            order.push(index);
        }
    }
    order.extend(heuristic.iter().copied().filter(|&i| !placed[i as usize]));
    order
}

pub fn status_code(status: FileStatus) -> char {
    match status {
        FileStatus::Modified => 'M',
        FileStatus::Added => 'A',
        FileStatus::Deleted => 'D',
        FileStatus::Renamed => 'R',
        FileStatus::Copied => 'C',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_obvious_low_value_files() {
        let cases = [
            ("Cargo.lock", Tier::Noise),
            ("web/package-lock.json", Tier::Noise),
            ("go.sum", Tier::Noise),
            ("assets/logo.png", Tier::Noise),
            ("vendor/pkg/a.go", Tier::Noise),
            ("src/__snapshots__/view.snap", Tier::Noise),
            ("tests/fixtures/sample.json", Tier::Noise),
            ("api/gen/service.pb.go", Tier::Noise),
            ("dist/bundle.min.js", Tier::Noise),
            ("src/schema.generated.ts", Tier::Noise),
            ("tests/flow.rs", Tier::Tests),
            ("src/parser/tests.rs", Tier::Tests),
            ("pkg/store/store_test.go", Tier::Tests),
            ("app/test_models.py", Tier::Tests),
            ("web/src/button.test.tsx", Tier::Tests),
            ("web/src/button.spec.ts", Tier::Tests),
            ("core/src/main/java/WidgetTest.java", Tier::Tests),
            ("README.md", Tier::Docs),
            ("docs/guide.html", Tier::Docs),
            ("LICENSE", Tier::Docs),
            ("CHANGELOG", Tier::Docs),
            ("Cargo.toml", Tier::Config),
            (".github/workflows/ci.yml", Tier::Config),
            (".gitignore", Tier::Config),
            ("Makefile", Tier::Config),
        ];
        for (path, expected) in cases {
            assert_eq!(tier(path, false), expected, "{path}");
        }
        assert_eq!(tier("src/engine.rs", true), Tier::Noise);
    }

    #[test]
    fn entry_points_and_shallow_sources_rank_as_central() {
        assert_eq!(tier("src/main.rs", false), Tier::Central);
        assert_eq!(tier("crates/core/src/lib.rs", false), Tier::Central);
        assert_eq!(tier("web/src/index.ts", false), Tier::Central);
        assert_eq!(tier("pkg/__init__.py", false), Tier::Central);
        assert_eq!(tier("src/engine.rs", false), Tier::Central);
        assert_eq!(tier("src/app/view/panel.rs", false), Tier::Source);
        assert_eq!(tier("scripts/release.sh", false), Tier::Source);
        assert_eq!(tier("src/app/testing.rs", false), Tier::Source);
        assert_eq!(tier("src/latest.rs", false), Tier::Central);
    }

    #[test]
    fn heuristic_order_sorts_by_tier_then_path() {
        let files = [
            (Tier::Tests, "tests/a.rs"),
            (Tier::Source, "src/b/y.rs"),
            (Tier::Noise, "Cargo.lock"),
            (Tier::Central, "src/main.rs"),
            (Tier::Source, "src/a/x.rs"),
        ];
        assert_eq!(heuristic_order(&files), [3, 4, 1, 0, 2]);
    }

    #[test]
    fn merge_keeps_ranked_first_and_fills_missing_from_heuristics() {
        let heuristic = [3, 4, 1, 0, 2];
        assert_eq!(merge_ranked(&heuristic, &[1, 3]), [1, 3, 4, 0, 2]);
        assert_eq!(merge_ranked(&heuristic, &[9, 1, 1, 4]), [1, 4, 3, 0, 2]);
        assert_eq!(merge_ranked(&heuristic, &[]), heuristic);
    }
}
