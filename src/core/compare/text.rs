use crate::core::compare::git;
use crate::core::compare::output::CompareOutput;
use crate::core::compare::service::SEMANTIC_UNAVAILABLE;
use crate::core::compare::spec::RendererKind;
use crate::core::error::Result;

pub fn compare_text(
    left_text: &str,
    right_text: &str,
    display_path: &str,
    renderer: RendererKind,
) -> Result<CompareOutput> {
    let display_path = normalized_display_path(display_path);
    if renderer == RendererKind::Difftastic {
        #[cfg(feature = "difftastic")]
        return crate::core::compare::semantic::compare_text(left_text, right_text, &display_path);
    }
    let mut output = git::compare_text(left_text, right_text, &display_path)?;
    if renderer == RendererKind::Difftastic {
        output.used_fallback = true;
        output.fallback_message = SEMANTIC_UNAVAILABLE.to_owned();
    }
    Ok(output)
}

fn normalized_display_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "text.txt".to_owned()
    } else {
        trimmed.replace('\\', "/")
    }
}

#[cfg(test)]
mod tests {
    use diffy_diff::{FileStatus, Side};

    use super::*;

    fn builtin(left: &str, right: &str) -> CompareOutput {
        compare_text(left, right, "snippet.rs", RendererKind::Builtin).unwrap()
    }

    #[test]
    fn text_compare_reports_status_and_stats() {
        let cases = [
            (
                "fn value() -> u32 { 1 }\n",
                "fn value() -> u32 { 2 }\n",
                FileStatus::Modified,
                (1, 1),
            ),
            ("", "new line\n", FileStatus::Added, (1, 0)),
            ("old line\n", "", FileStatus::Deleted, (0, 1)),
        ];
        for (left, right, status, stats) in cases {
            let output = builtin(left, right);
            assert_eq!(output.file_count(), 1);
            let entry = output.entry(0).unwrap();
            assert_eq!(entry.path(), "snippet.rs");
            assert_eq!(entry.status(), status);
            assert_eq!(entry.stats.known(), Some(stats));
        }
        assert_eq!(builtin("", "").file_count(), 0);
    }

    #[test]
    fn text_compare_keeps_full_sources_and_missing_newlines() {
        let output = builtin("old", "new");
        let doc = output.document(0).unwrap();
        let facts = doc.facts(0);
        assert!(facts.old_missing_newline && facts.new_missing_newline);
        assert_eq!(doc.text(0, Side::New).as_str(), "new");
        assert_eq!(
            compare_text("a", "b", "  ", RendererKind::Builtin)
                .unwrap()
                .entry(0)
                .unwrap()
                .path(),
            "text.txt"
        );
    }

    #[cfg(feature = "difftastic")]
    #[test]
    fn semantic_text_compare_keeps_change_intensities() {
        let output = compare_text(
            "fn value() -> u32 {\n    1\n}\n",
            "fn value() -> u32 {\n    2\n}\n",
            "snippet.rs",
            RendererKind::Difftastic,
        )
        .unwrap();
        assert!(!output.used_fallback);
        let doc = output.document(0).unwrap();
        assert_eq!(doc.stats(0).known(), Some((1, 1)));
        let novel = doc.semantic_spans(0, Side::New, 1);
        assert!(!novel.is_empty(), "difftastic marks the changed literal");
        assert!(novel.iter().all(|span| span.end as usize <= "    2".len()));
    }
}
