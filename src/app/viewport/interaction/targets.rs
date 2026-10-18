use diffy_ui::DiffAnchor;
use diffy_ui::diff_view::decorator::HunkSource;

use crate::contracts::shared::{FileRevision, HunkTarget, Side, SourceTarget};

pub fn source_target(anchor: &DiffAnchor) -> SourceTarget {
    SourceTarget {
        file: anchor.file,
        revision: FileRevision(anchor.revision.0),
        side: anchor.side,
        lines: anchor.lines.clone(),
    }
}

pub fn hunk_target(hunk: &HunkSource) -> HunkTarget {
    let (side, lines) = if hunk.new.is_empty() {
        (Side::Old, hunk.old.clone())
    } else {
        (Side::New, hunk.new.clone())
    };
    HunkTarget {
        source: SourceTarget {
            file: hunk.file.id,
            revision: FileRevision(hunk.file.revision.0),
            side,
            lines,
        },
        hunk: hunk.hunk,
    }
}
