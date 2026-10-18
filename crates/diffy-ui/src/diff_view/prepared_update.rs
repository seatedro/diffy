use diffy_diff::{DiffDocument, FileStatus, Side};

use super::prepared::FileFact;

/// Facts about `file` its line rows do not show. See [`FileFact`]. A
/// missing final newline is a fact only where it differs between the sides
/// (or the file was added or deleted), since an unchanged one is not part
/// of the change.
pub(crate) fn file_facts(doc: &DiffDocument, file: u32) -> Vec<FileFact> {
    let facts = doc.facts(file);
    let mut out = Vec::new();
    if facts.binary {
        out.push(FileFact::Binary);
    }
    if let Some((old, new)) = facts.mode_change {
        out.push(FileFact::ModeChange { old, new });
    }
    match facts.status {
        FileStatus::Renamed if !facts.has_hunks && !facts.binary => {
            out.push(FileFact::RenameOnly);
        }
        FileStatus::Copied if !facts.has_hunks && !facts.binary => out.push(FileFact::CopyOnly),
        _ => {}
    }
    let eof = [facts.old_missing_newline, facts.new_missing_newline];
    for side in [Side::Old, Side::New] {
        let differs = match facts.status {
            FileStatus::Added => side == Side::New,
            FileStatus::Deleted => side == Side::Old,
            _ => eof[0] != eof[1],
        };
        if eof[side as usize] && differs {
            out.push(FileFact::NoNewlineAtEof(side));
        }
    }
    out
}
