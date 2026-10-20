//! Checked limits on input size and on per-line detail, so oversized input
//! is refused before it overflows the model's `u32` offsets, and limited
//! detail is reported rather than silently dropped.

use std::fmt;

use unicode_segmentation::GraphemeCursor;

use crate::compute::diff_texts;
use crate::inline::MAX_INLINE_LINE_BYTES;
use crate::model::DiffDocument;
use crate::patch::{PatchError, parse_unified};

/// Bytes or lines past which the model's `u32` offsets would overflow.
pub const MAX_REPRESENTABLE: u64 = u32::MAX as u64 - 1;

/// Bytes per side every default detail limit covers: a diff of two sides
/// this large gets word highlights, whole shaped lines, and syntax colors.
pub const DETAIL_SIDE_BYTES: usize = 64 << 20;

/// Size limits. The input limits default to what the model can represent;
/// the detail limits default to [`DETAIL_SIDE_BYTES`], so nothing is left
/// out below that size. An app can lower any of them, and whatever it
/// then leaves out is reported (as [`LineDetail::Prefix`],
/// `InlineDetail::Limited`, or a limited syntax status), never dropped
/// silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiffLimits {
    /// Bytes of one patch, or of one side of a text diff. Capped at
    /// [`MAX_REPRESENTABLE`] whatever it is set to.
    pub input_bytes: u64,
    /// Lines of one patch, or of one side of a text diff. Capped likewise.
    pub input_lines: u64,
    /// Longer lines get no inline diff.
    pub inline_line_bytes: usize,
    /// Longer lines shape only a prefix; see [`line_detail`]. Lines up to
    /// this long are shown whole: a viewer shapes the part of a long line
    /// in view, not a prefix.
    pub shaped_line_bytes: usize,
    /// Larger sources get plain text instead of a syntax parse.
    pub syntax_file_bytes: usize,
}

impl Default for DiffLimits {
    fn default() -> Self {
        Self {
            input_bytes: MAX_REPRESENTABLE,
            input_lines: MAX_REPRESENTABLE,
            inline_line_bytes: MAX_INLINE_LINE_BYTES,
            shaped_line_bytes: DETAIL_SIDE_BYTES,
            syntax_file_bytes: DETAIL_SIDE_BYTES,
        }
    }
}

/// Which input limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitKind {
    Bytes,
    Lines,
}

/// Why a checked ingestion failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffError {
    Patch(PatchError),
    InputTooLarge {
        kind: LimitKind,
        actual: u64,
        limit: u64,
    },
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Patch(error) => error.fmt(f),
            Self::InputTooLarge {
                kind,
                actual,
                limit,
            } => write!(f, "input has {actual} {kind:?}, over the limit of {limit}"),
        }
    }
}

impl std::error::Error for DiffError {}

impl From<PatchError> for DiffError {
    fn from(error: PatchError) -> Self {
        Self::Patch(error)
    }
}

impl DiffLimits {
    /// Refuses `text` if it has more bytes or lines than allowed. Counts
    /// lines only when the byte count could hide too many.
    pub fn check_input(&self, text: &str) -> Result<(), DiffError> {
        let bytes = text.len() as u64;
        let byte_limit = self.input_bytes.min(MAX_REPRESENTABLE);
        if bytes > byte_limit {
            return Err(DiffError::InputTooLarge {
                kind: LimitKind::Bytes,
                actual: bytes,
                limit: byte_limit,
            });
        }
        let line_limit = self.input_lines.min(MAX_REPRESENTABLE);
        if bytes > line_limit {
            let lines = text.bytes().filter(|&b| b == b'\n').count() as u64
                + u64::from(!text.is_empty() && !text.ends_with('\n'));
            if lines > line_limit {
                return Err(DiffError::InputTooLarge {
                    kind: LimitKind::Lines,
                    actual: lines,
                    limit: line_limit,
                });
            }
        }
        Ok(())
    }

    /// Whether a source of `bytes` bytes may be parsed for syntax.
    pub fn allows_syntax(&self, bytes: usize) -> bool {
        bytes <= self.syntax_file_bytes
    }
}

/// [`parse_unified`] after checking `input` against `limits`.
pub fn parse_unified_checked(input: &str, limits: &DiffLimits) -> Result<DiffDocument, DiffError> {
    limits.check_input(input)?;
    Ok(parse_unified(input)?)
}

/// [`diff_texts`] after checking both sides against `limits`.
pub fn diff_texts_checked(
    old_path: Option<&str>,
    new_path: Option<&str>,
    old: Option<&str>,
    new: Option<&str>,
    context: u32,
    limits: &DiffLimits,
) -> Result<DiffDocument, DiffError> {
    for text in [old, new].into_iter().flatten() {
        limits.check_input(text)?;
    }
    Ok(diff_texts(old_path, new_path, old, new, context))
}

/// How much of a line enters text shaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineDetail {
    Complete,
    /// Only the first `shown_bytes` of `total_bytes`, ending on a
    /// grapheme boundary; the source itself is untouched.
    Prefix {
        shown_bytes: u32,
        total_bytes: u32,
    },
}

/// The part of `line` to shape under a `max_bytes` cap: all of it, or the
/// longest prefix of whole grapheme clusters that fits. A single cluster
/// longer than the cap leaves an empty prefix.
pub fn line_detail(line: &str, max_bytes: usize) -> LineDetail {
    if line.len() <= max_bytes {
        return LineDetail::Complete;
    }
    let mut at = max_bytes;
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    let mut cursor = GraphemeCursor::new(at, line.len(), true);
    let shown = match cursor.is_boundary(line, 0) {
        Ok(true) => at,
        _ => cursor.prev_boundary(line, 0).ok().flatten().unwrap_or(0),
    };
    LineDetail::Prefix {
        shown_bytes: shown as u32,
        total_bytes: line.len() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::{DiffError, DiffLimits, LimitKind, LineDetail, line_detail, parse_unified_checked};

    #[test]
    fn oversized_input_is_refused_before_parsing() {
        let patch = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        let limits = |input_bytes, input_lines| DiffLimits {
            input_bytes,
            input_lines,
            ..DiffLimits::default()
        };
        let too_large = |kind, actual, limit| {
            Some(DiffError::InputTooLarge {
                kind,
                actual,
                limit,
            })
        };
        let cases = [
            (limits(30, 100), too_large(LimitKind::Bytes, 34, 30)),
            (limits(100, 4), too_large(LimitKind::Lines, 5, 4)),
            // Limits above what the model represents are capped, not obeyed.
            (limits(u64::MAX, u64::MAX), None),
        ];
        for (limits, expected) in cases {
            assert_eq!(parse_unified_checked(patch, &limits).err(), expected);
        }
    }

    #[test]
    fn an_overlong_line_shapes_a_grapheme_safe_prefix() {
        let mib = "x".repeat(1 << 20);
        let accent_across_cap = format!("{}e\u{301}tail", "a".repeat(9));
        let combining = format!("e{}", "\u{301}".repeat(100));
        let cases = [
            ("short line", "short", 10, LineDetail::Complete),
            (
                "one mebibyte",
                mib.as_str(),
                4_096,
                LineDetail::Prefix {
                    shown_bytes: 4_096,
                    total_bytes: 1 << 20,
                },
            ),
            // The cap falls inside `e` + accent at bytes 9..12.
            (
                "accent across the cap",
                accent_across_cap.as_str(),
                11,
                LineDetail::Prefix {
                    shown_bytes: 9,
                    total_bytes: 16,
                },
            ),
            // One cluster longer than the cap: nothing whole fits.
            (
                "long combining sequence",
                combining.as_str(),
                50,
                LineDetail::Prefix {
                    shown_bytes: 0,
                    total_bytes: 201,
                },
            ),
        ];
        for (name, line, cap, expected) in cases {
            assert_eq!(line_detail(line, cap), expected, "{name}");
        }
    }
}
