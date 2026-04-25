use carbon::{
    Block, BlockId, BlockRange, ExpansionDirection, ExpansionState, FileDiff, FileId, Hunk, HunkId,
    InlineOptions, ProjectionOptions, SourceRange, TextStore, compute_inline_diff, expand_context,
    parse_unified_patch, project_file, project_window,
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

fn many_hunk_file(hunk_count: usize) -> FileDiff {
    let line_count = hunk_count * 4 + 4;
    let old = source_text(line_count);
    let mut new_lines = old.lines().map(str::to_owned).collect::<Vec<_>>();
    for hunk_index in 0..hunk_count {
        let line_index = hunk_index * 4 + 2;
        new_lines[line_index] = format!("fn changed_{hunk_index}() {{ let value = 777; }}");
    }

    let mut new = String::new();
    for line in &new_lines {
        new.push_str(line);
        new.push('\n');
    }

    let mut file = FileDiff {
        id: FileId(2),
        old_text: Some(TextStore::from_text(old)),
        new_text: Some(TextStore::from_text(new)),
        ..FileDiff::default()
    };
    for hunk_index in 0..hunk_count {
        let line_index = (hunk_index * 4 + 2) as u32;
        let hunk_id = HunkId(hunk_index.min(u32::MAX as usize) as u32);
        let block_id = BlockId(hunk_index.min(u32::MAX as usize) as u32);
        file.add_hunk(
            Hunk::new(
                hunk_id,
                line_index + 1,
                1,
                line_index + 1,
                1,
                BlockRange::default(),
            ),
            [Block::change(
                block_id,
                SourceRange::new(line_index, 1),
                SourceRange::new(line_index, 1),
            )],
        );
    }
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
    let mut expanded = ExpansionState::default();
    expand_context(
        &file,
        &mut expanded,
        HunkId(0),
        ExpansionDirection::Above,
        u32::MAX,
    );
    expand_context(
        &file,
        &mut expanded,
        HunkId(0),
        ExpansionDirection::Below,
        u32::MAX,
    );
    let many = many_hunk_file(4_096);
    let collapsed_rows = count_project_file(
        &file,
        ProjectionOptions::default(),
        &ExpansionState::default(),
    );
    let expanded_rows = count_project_file(
        &file,
        ProjectionOptions {
            collapsed_context_threshold: 0,
            ..ProjectionOptions::default()
        },
        &expanded,
    );
    let many_rows = count_project_file(
        &many,
        ProjectionOptions {
            collapsed_context_threshold: 0,
            ..ProjectionOptions::default()
        },
        &ExpansionState::default(),
    );
    let full_file_vec_name = format!("projection/full_file_vec/{collapsed_rows}_rows");
    c.bench_function(&full_file_vec_name, |b| {
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
    let expanded_vec_name = format!("projection/full_expanded_context_vec/{expanded_rows}_rows");
    c.bench_function(&expanded_vec_name, |b| {
        b.iter(|| {
            let mut rows = Vec::new();
            project_file(
                black_box(&file),
                ProjectionOptions {
                    collapsed_context_threshold: 0,
                    ..ProjectionOptions::default()
                },
                black_box(&expanded),
                |row| rows.push(row),
            );
            black_box(rows);
        })
    });
    let expanded_count_name =
        format!("projection/full_expanded_context_count/{expanded_rows}_rows");
    c.bench_function(&expanded_count_name, |b| {
        b.iter(|| {
            let mut count = 0_usize;
            project_file(
                black_box(&file),
                ProjectionOptions {
                    collapsed_context_threshold: 0,
                    ..ProjectionOptions::default()
                },
                black_box(&expanded),
                |row| {
                    black_box(row);
                    count += 1;
                },
            );
            black_box(count);
        })
    });
    let many_vec_name = format!("projection/many_hunks_vec/{many_rows}_rows");
    c.bench_function(&many_vec_name, |b| {
        b.iter(|| {
            let mut rows = Vec::new();
            project_file(
                black_box(&many),
                ProjectionOptions {
                    collapsed_context_threshold: 0,
                    ..ProjectionOptions::default()
                },
                &ExpansionState::default(),
                |row| rows.push(row),
            );
            black_box(rows);
        })
    });
    let many_count_name = format!("projection/many_hunks_count/{many_rows}_rows");
    c.bench_function(&many_count_name, |b| {
        b.iter(|| {
            let mut count = 0_usize;
            project_file(
                black_box(&many),
                ProjectionOptions {
                    collapsed_context_threshold: 0,
                    ..ProjectionOptions::default()
                },
                &ExpansionState::default(),
                |row| {
                    black_box(row);
                    count += 1;
                },
            );
            black_box(count);
        })
    });
    c.bench_function("projection/window_vec/80_rows", |b| {
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

fn count_project_file(
    file: &FileDiff,
    options: ProjectionOptions,
    expansion: &ExpansionState,
) -> usize {
    let mut count = 0;
    project_file(file, options, expansion, |_| count += 1);
    count
}
