use gix::diff::blob::{
    Algorithm, Diff, InternedInput, UnifiedDiff, diff_with_slider_heuristics,
    platform::resource::ByteLinesWithoutTerminator,
    unified_diff::{ConsumeBinaryHunk, ContextSize},
};

use crate::core::compare::CompareOutput;
use crate::core::compare::stats::CompareFileSummary;
use crate::core::error::{DiffyError, Result};

pub const BLANK_DIFF_PATH: &str = "Untitled.txt";
const BLANK_DIFF_CONTEXT_LINES: u32 = 3;

pub(crate) fn output_from_text_pair(old: &str, new: &str) -> Result<CompareOutput> {
    let mut raw_diff = raw_patch_header();
    raw_diff.push_str(&render_gix_unified_hunks(
        old.as_bytes(),
        new.as_bytes(),
        BLANK_DIFF_CONTEXT_LINES,
    )?);

    let mut document = carbon::parse_unified_patch(&raw_diff)
        .map_err(|error| DiffyError::Parse(error.to_string()))?;
    let Some(file) = document.files.first_mut() else {
        return Err(DiffyError::Parse("blank diff produced no file".to_owned()));
    };
    file.is_partial = false;
    let (additions, deletions) = gix_line_stats(old.as_bytes(), new.as_bytes());
    file.additions = additions;
    file.deletions = deletions;

    Ok(CompareOutput {
        file_summaries: document
            .files
            .iter()
            .map(CompareFileSummary::from_file)
            .collect(),
        carbon: document,
        raw_diff,
        used_fallback: false,
        fallback_message: String::new(),
    })
}

pub(super) fn gix_line_stats(old_content: &[u8], new_content: &[u8]) -> (u32, u32) {
    let input = InternedInput::new(
        ByteLinesWithoutTerminator::new(old_content),
        ByteLinesWithoutTerminator::new(new_content),
    );
    let diff = Diff::compute(Algorithm::Histogram, &input);
    (diff.count_additions(), diff.count_removals())
}

pub(super) fn render_gix_unified_hunks(
    old_content: &[u8],
    new_content: &[u8],
    context_lines: u32,
) -> Result<String> {
    let input = InternedInput::new(
        ByteLinesWithoutTerminator::new(old_content),
        ByteLinesWithoutTerminator::new(new_content),
    );
    let diff = diff_with_slider_heuristics(Algorithm::Histogram, &input);
    UnifiedDiff::new(
        &diff,
        &input,
        ConsumeBinaryHunk::new(String::new(), "\n"),
        ContextSize::symmetrical(context_lines),
    )
    .consume()
    .map_err(|error| DiffyError::General(format!("Text diff render failed: {error}")))
}

fn raw_patch_header() -> String {
    format!(
        "diff --git a/{path} b/{path}\nindex 0000000000000000000000000000000000000000..0000000000000000000000000000000000000000 100644\n--- a/{path}\n+++ b/{path}\n",
        path = BLANK_DIFF_PATH,
    )
}
