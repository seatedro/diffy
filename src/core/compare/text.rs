use crate::core::compare::backends::{compare_text_builtin, compare_text_difftastic};
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::{LayoutMode, RendererKind};
use crate::core::error::Result;

pub fn compare_text(
    left_text: &str,
    right_text: &str,
    display_path: &str,
    renderer: RendererKind,
    _layout: LayoutMode,
) -> Result<CompareOutput> {
    let display_path = normalized_display_path(display_path);
    match renderer {
        RendererKind::Builtin => compare_text_builtin(left_text, right_text, &display_path),
        RendererKind::Difftastic => {
            match compare_text_difftastic(left_text, right_text, &display_path)? {
                Some(output) => Ok(output),
                None => {
                    let mut output = compare_text_builtin(left_text, right_text, &display_path)?;
                    output.used_fallback = true;
                    output.fallback_message =
                        "difftastic unavailable, fell back to built-in backend".to_owned();
                    Ok(output)
                }
            }
        }
    }
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
    use super::*;

    fn builtin(left: &str, right: &str) -> CompareOutput {
        compare_text(
            left,
            right,
            "snippet.rs",
            RendererKind::Builtin,
            LayoutMode::Unified,
        )
        .unwrap()
    }

    #[test]
    fn text_compare_modified_text_builds_compare_output() {
        let output = builtin("fn value() -> u32 { 1 }\n", "fn value() -> u32 { 2 }\n");

        assert_eq!(output.file_count(), 1);
        let file = &output.carbon.files[0];
        assert_eq!(file.path(), "snippet.rs");
        assert_eq!(file.status, carbon::FileStatus::Modified);
        assert_eq!(file.additions, 1);
        assert_eq!(file.deletions, 1);
    }

    #[test]
    fn text_compare_added_text_builds_added_file() {
        let output = builtin("", "new line\n");

        let file = &output.carbon.files[0];
        assert_eq!(file.status, carbon::FileStatus::Added);
        assert_eq!(file.additions, 1);
    }

    #[test]
    fn text_compare_deleted_text_builds_deleted_file() {
        let output = builtin("old line\n", "");

        let file = &output.carbon.files[0];
        assert_eq!(file.status, carbon::FileStatus::Deleted);
        assert_eq!(file.deletions, 1);
    }

    #[test]
    fn text_compare_equal_empty_text_has_no_files() {
        let output = builtin("", "");

        assert_eq!(output.file_count(), 0);
    }

    #[test]
    fn text_compare_preserves_no_newline_at_eof_markers() {
        let output = builtin("old", "new");

        let block = output.carbon.files[0].blocks.first().unwrap();
        assert!(block.old_no_newline_at_end);
        assert!(block.new_no_newline_at_end);
    }
}
