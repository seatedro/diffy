use carbon::{
    Block, BlockId, BlockRange, ExpansionState, FileDiff, FileId, Hunk, HunkId, InlineOptions,
    ProjectionOptions, SourceRange, TextStore, compute_inline_diff, parse_unified_patch,
    project_file, project_window,
};
use criterion::{BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main};

fn source_text(line_count: usize) -> String {
    let mut text = String::with_capacity(line_count * 32);
    for index in 0..line_count {
        text.push_str("fn item_");
        text.push_str(&index.to_string());
        text.push_str("() { let value = ");
        text.push_str(&(index % 97).to_string());
        text.push_str("; }\n");
    }
    text
}

fn minified_text(byte_len: usize) -> String {
    let mut text = String::with_capacity(byte_len);
    while text.len() < byte_len {
        text.push_str("let a=1;let b=2;let c=a+b;");
    }
    text.truncate(byte_len);
    text
}

fn large_file() -> FileDiff {
    let old = source_text(20_000);
    let mut new = old.clone();
    new = new.replace(
        "fn item_10000() { let value = 6; }",
        "fn item_10000() { let value = 777; }",
    );
    let mut file = FileDiff {
        id: FileId(1),
        old_text: Some(TextStore::from_text(old)),
        new_text: Some(TextStore::from_text(new)),
        ..FileDiff::default()
    };
    file.add_hunk(
        Hunk::new(HunkId(0), 10_001, 1, 10_001, 1, BlockRange::default()),
        [Block::change(
            BlockId(0),
            SourceRange::new(10_000, 1),
            SourceRange::new(10_000, 1),
        )],
    );
    file
}

fn large_patch(file_count: usize, lines_per_file: usize) -> String {
    let mut patch = String::new();
    for file_index in 0..file_count {
        patch.push_str("diff --git a/file");
        patch.push_str(&file_index.to_string());
        patch.push_str(".rs b/file");
        patch.push_str(&file_index.to_string());
        patch.push_str(".rs\nindex 1111111..2222222 100644\n--- a/file");
        patch.push_str(&file_index.to_string());
        patch.push_str(".rs\n+++ b/file");
        patch.push_str(&file_index.to_string());
        patch.push_str(".rs\n@@ -1,");
        patch.push_str(&lines_per_file.to_string());
        patch.push_str(" +1,");
        patch.push_str(&lines_per_file.to_string());
        patch.push_str(" @@\n");
        for line in 0..lines_per_file {
            if line == lines_per_file / 2 {
                patch.push_str("-let value = 1;\n+let value = 2;\n");
            } else {
                patch.push_str(" let value = 0;\n");
            }
        }
    }
    patch
}

fn bench_text_store(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_store");
    for (name, text) in [
        ("large_lf", source_text(50_000)),
        ("large_crlf", source_text(50_000).replace('\n', "\r\n")),
        ("minified", minified_text(1_500_000)),
    ] {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_function(name, |b| {
            b.iter_batched(
                || text.clone().into_bytes(),
                |bytes| black_box(TextStore::from_bytes(bytes)),
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn bench_projection(c: &mut Criterion) {
    let file = large_file();
    c.bench_function("projection/full_file", |b| {
        b.iter(|| {
            let mut rows = Vec::new();
            project_file(
                black_box(&file),
                ProjectionOptions::default(),
                &ExpansionState::default(),
                |row| rows.push(row),
            );
            black_box(rows);
        })
    });
    c.bench_function("projection/window", |b| {
        b.iter(|| {
            let mut rows = Vec::new();
            project_window(
                black_box(&file),
                ProjectionOptions::default(),
                &ExpansionState::default(),
                carbon::ProjectionWindow {
                    start: 9_990,
                    len: 80,
                },
                |row| rows.push(row),
            );
            black_box(rows);
        })
    });
}

fn bench_patch(c: &mut Criterion) {
    let patch = large_patch(64, 64);
    c.bench_function("patch/parse_git_unified", |b| {
        b.iter(|| black_box(parse_unified_patch(black_box(&patch)).unwrap()))
    });
}

fn bench_inline(c: &mut Criterion) {
    let old = "let alpha_value = compute_expensive_result(old_input, shared_context);";
    let new = "let beta_value = compute_expensive_result(new_input, shared_context);";
    c.bench_function("inline/word", |b| {
        b.iter(|| black_box(compute_inline_diff(old, new, InlineOptions::default())))
    });
}

criterion_group!(
    benches,
    bench_text_store,
    bench_projection,
    bench_patch,
    bench_inline
);
criterion_main!(benches);
