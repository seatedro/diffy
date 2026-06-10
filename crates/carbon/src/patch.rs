use crate::model::{
    Block, BlockId, BlockKind, BlockRange, DiffDocument, DiffSide, FileDiff, FileId, FileMode,
    FileStatus, Hunk, HunkId, ObjectId, SourceRange,
};
use crate::text::{TextStore, u32_to_usize_saturating, usize_to_u32_saturating};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    Empty,
    InvalidHunkHeader(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatchError::Empty => formatter.write_str("patch contains no file diffs"),
            PatchError::InvalidHunkHeader(header) => {
                write!(formatter, "invalid hunk header: {header}")
            }
        }
    }
}

impl std::error::Error for PatchError {}

pub fn parse_unified_patch(input: &str) -> Result<DiffDocument, PatchError> {
    let mut parser = PatchParser::default();

    for raw_line in input.lines() {
        parser.push_line(raw_line.strip_suffix('\r').unwrap_or(raw_line))?;
    }

    let document = parser.finish();
    if document.files.is_empty() {
        return Err(PatchError::Empty);
    }
    Ok(document)
}

#[derive(Default)]
struct PatchParser {
    files: Vec<FileDiff>,
    current: Option<FileBuilder>,
}

impl PatchParser {
    fn push_line(&mut self, line: &str) -> Result<(), PatchError> {
        if line.starts_with("diff --git ") {
            self.finish_current_file();
            self.current = Some(FileBuilder::from_diff_git(
                usize_to_u32_saturating(self.files.len()),
                line,
            ));
            return Ok(());
        }

        let file = self
            .current
            .get_or_insert_with(|| FileBuilder::new(usize_to_u32_saturating(self.files.len())));

        if line.starts_with("@@ ") {
            file.start_hunk(line)?;
        } else if file.in_hunk() {
            file.push_hunk_line(line);
        } else {
            file.push_metadata_line(line);
        }

        Ok(())
    }

    fn finish_current_file(&mut self) {
        if let Some(file) = self.current.take() {
            self.files.push(file.finish());
        }
    }

    fn finish(mut self) -> DiffDocument {
        self.finish_current_file();
        DiffDocument { files: self.files }
    }
}

struct FileBuilder {
    file: FileDiff,
    old_text: String,
    new_text: String,
    old_line_count: u32,
    new_line_count: u32,
    hunk: Option<HunkBuilder>,
}

impl FileBuilder {
    fn new(file_id: u32) -> Self {
        Self {
            file: FileDiff {
                id: FileId(file_id),
                is_partial: true,
                ..FileDiff::default()
            },
            old_text: String::new(),
            new_text: String::new(),
            old_line_count: 0,
            new_line_count: 0,
            hunk: None,
        }
    }

    fn from_diff_git(file_id: u32, line: &str) -> Self {
        let mut builder = Self::new(file_id);
        let mut parts = line.split_whitespace().skip(2);
        builder.file.old_path = parts.next().and_then(strip_patch_path).map(str::to_owned);
        builder.file.new_path = parts.next().and_then(strip_patch_path).map(str::to_owned);
        builder
    }

    fn in_hunk(&self) -> bool {
        self.hunk.is_some()
    }

    fn push_metadata_line(&mut self, line: &str) {
        if let Some(mode) = line.strip_prefix("new file mode ") {
            self.file.status = FileStatus::Added;
            self.file.new_mode = Some(FileMode(mode.to_owned()));
        } else if let Some(mode) = line.strip_prefix("deleted file mode ") {
            self.file.status = FileStatus::Deleted;
            self.file.old_mode = Some(FileMode(mode.to_owned()));
        } else if let Some(mode) = line.strip_prefix("old mode ") {
            self.file.old_mode = Some(FileMode(mode.to_owned()));
            if self.file.status == FileStatus::Modified {
                self.file.status = FileStatus::ModeChanged;
            }
        } else if let Some(mode) = line.strip_prefix("new mode ") {
            self.file.new_mode = Some(FileMode(mode.to_owned()));
            if self.file.status == FileStatus::Modified {
                self.file.status = FileStatus::ModeChanged;
            }
        } else if let Some(path) = line.strip_prefix("rename from ") {
            self.file.old_path = Some(path.to_owned());
            self.file.status = FileStatus::Renamed;
        } else if let Some(path) = line.strip_prefix("rename to ") {
            self.file.new_path = Some(path.to_owned());
            self.file.status = FileStatus::Renamed;
        } else if let Some(rest) = line.strip_prefix("index ") {
            self.parse_index_line(rest);
        } else if let Some(path) = line.strip_prefix("--- ") {
            if let Some(path) = strip_patch_path(path) {
                self.file.old_path = Some(path.to_owned());
            }
        } else if let Some(path) = line.strip_prefix("+++ ") {
            if let Some(path) = strip_patch_path(path) {
                self.file.new_path = Some(path.to_owned());
            }
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            self.file.status = FileStatus::Binary;
            self.file.is_binary = true;
        }
    }

    fn parse_index_line(&mut self, rest: &str) {
        let Some((oids, mode)) = rest.split_once(' ') else {
            self.parse_index_oids(rest);
            return;
        };
        self.parse_index_oids(oids);
        self.file.old_mode = Some(FileMode(mode.to_owned()));
        self.file.new_mode = Some(FileMode(mode.to_owned()));
    }

    fn parse_index_oids(&mut self, oids: &str) {
        let Some((old_oid, new_oid)) = oids.split_once("..") else {
            return;
        };
        self.file.old_oid = Some(ObjectId(old_oid.to_owned()));
        self.file.new_oid = Some(ObjectId(new_oid.to_owned()));
    }

    fn start_hunk(&mut self, line: &str) -> Result<(), PatchError> {
        self.finish_hunk();
        let (old_start, old_count, new_start, new_count) = parse_hunk_header(line)?;
        // Header counts are untrusted input; the reservation is only a
        // warm-up, so cap it to keep a hostile count from forcing a huge
        // allocation. Real content still grows the buffers as it is pushed.
        const MAX_HUNK_RESERVE_BYTES: usize = 1 << 20;
        self.old_text.reserve(
            u32_to_usize_saturating(old_count)
                .saturating_mul(32)
                .min(MAX_HUNK_RESERVE_BYTES),
        );
        self.new_text.reserve(
            u32_to_usize_saturating(new_count)
                .saturating_mul(32)
                .min(MAX_HUNK_RESERVE_BYTES),
        );
        self.file.hunks.reserve(1);
        self.file.blocks.reserve(3);
        self.hunk = Some(HunkBuilder::new(
            HunkId(usize_to_u32_saturating(self.file.hunks.len())),
            line.to_owned(),
            old_start,
            old_count,
            new_start,
            new_count,
            usize_to_u32_saturating(self.file.blocks.len()),
            self.old_line_count,
            self.new_line_count,
        ));
        Ok(())
    }

    fn push_hunk_line(&mut self, line: &str) {
        let Some(hunk) = self.hunk.as_mut() else {
            return;
        };

        // Any `\`-prefixed hunk line is a "no newline at end of file" marker;
        // the message text is localized by diff/git, so only the prefix is
        // structural.
        if line.starts_with('\\') {
            match hunk.last_side {
                Some(DiffSide::Old) => {
                    if trim_trailing_newline(&mut self.old_text) {
                        hunk.mark_old_no_newline();
                    }
                }
                Some(DiffSide::New) => {
                    if trim_trailing_newline(&mut self.new_text) {
                        hunk.mark_new_no_newline();
                    }
                }
                None => {}
            }
            return;
        }

        let (kind, content) = match line.as_bytes().first().copied() {
            Some(b' ') => (PatchLineKind::Context, &line[1..]),
            Some(b'-') => (PatchLineKind::Old, &line[1..]),
            Some(b'+') => (PatchLineKind::New, &line[1..]),
            _ => (PatchLineKind::Context, line),
        };

        match kind {
            PatchLineKind::Context => hunk.push_context(
                content,
                &mut self.old_text,
                &mut self.new_text,
                &mut self.old_line_count,
                &mut self.new_line_count,
            ),
            PatchLineKind::Old => {
                hunk.push_old(content, &mut self.old_text, &mut self.old_line_count)
            }
            PatchLineKind::New => {
                hunk.push_new(content, &mut self.new_text, &mut self.new_line_count)
            }
        }
    }

    fn finish_hunk(&mut self) {
        let Some(mut builder) = self.hunk.take() else {
            return;
        };
        builder.finish_block();
        let blocks = builder.blocks;
        let mut hunk = Hunk::new(
            builder.id,
            builder.old_start,
            builder.old_count,
            builder.new_start,
            builder.new_count,
            BlockRange::default(),
        );
        hunk.header = builder.header;
        self.file.add_hunk(hunk, blocks);
    }

    fn finish(mut self) -> FileDiff {
        self.finish_hunk();
        if self.file.status == FileStatus::Renamed && !self.file.hunks.is_empty() {
            self.file.status = FileStatus::RenamedModified;
        }
        self.file.old_text = (self.old_line_count > 0).then(|| TextStore::from_text(self.old_text));
        self.file.new_text = (self.new_line_count > 0).then(|| TextStore::from_text(self.new_text));
        for block in &self.file.blocks {
            if block.kind == BlockKind::Change {
                self.file.additions = self.file.additions.saturating_add(block.new.len);
                self.file.deletions = self.file.deletions.saturating_add(block.old.len);
            }
        }
        self.file
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchLineKind {
    Context,
    Old,
    New,
}

struct HunkBuilder {
    id: HunkId,
    header: String,
    old_start: u32,
    old_count: u32,
    new_start: u32,
    new_count: u32,
    old_cursor: u32,
    new_cursor: u32,
    old_store_cursor: u32,
    new_store_cursor: u32,
    next_block_id: u32,
    current: Option<BlockBuilder>,
    blocks: Vec<Block>,
    last_side: Option<DiffSide>,
}

impl HunkBuilder {
    fn new(
        id: HunkId,
        header: String,
        old_start: u32,
        old_count: u32,
        new_start: u32,
        new_count: u32,
        next_block_id: u32,
        old_store_cursor: u32,
        new_store_cursor: u32,
    ) -> Self {
        Self {
            id,
            header,
            old_start,
            old_count,
            new_start,
            new_count,
            old_cursor: old_start,
            new_cursor: new_start,
            old_store_cursor,
            new_store_cursor,
            next_block_id,
            current: None,
            blocks: Vec::with_capacity(3),
            last_side: None,
        }
    }

    fn push_context(
        &mut self,
        content: &str,
        old_text: &mut String,
        new_text: &mut String,
        old_line_count: &mut u32,
        new_line_count: &mut u32,
    ) {
        self.ensure_block(BlockKind::Context);
        push_text_line(old_text, content);
        push_text_line(new_text, content);
        *old_line_count = old_line_count.saturating_add(1);
        *new_line_count = new_line_count.saturating_add(1);
        if let Some(block) = self.current.as_mut() {
            block.old.len = block.old.len.saturating_add(1);
            block.new.len = block.new.len.saturating_add(1);
        }
        self.old_cursor = self.old_cursor.saturating_add(1);
        self.new_cursor = self.new_cursor.saturating_add(1);
        self.old_store_cursor = self.old_store_cursor.saturating_add(1);
        self.new_store_cursor = self.new_store_cursor.saturating_add(1);
        self.last_side = Some(DiffSide::New);
    }

    fn push_old(&mut self, content: &str, old_text: &mut String, old_line_count: &mut u32) {
        self.ensure_block(BlockKind::Change);
        push_text_line(old_text, content);
        *old_line_count = old_line_count.saturating_add(1);
        if let Some(block) = self.current.as_mut() {
            block.old.len = block.old.len.saturating_add(1);
        }
        self.old_cursor = self.old_cursor.saturating_add(1);
        self.old_store_cursor = self.old_store_cursor.saturating_add(1);
        self.last_side = Some(DiffSide::Old);
    }

    fn push_new(&mut self, content: &str, new_text: &mut String, new_line_count: &mut u32) {
        self.ensure_block(BlockKind::Change);
        push_text_line(new_text, content);
        *new_line_count = new_line_count.saturating_add(1);
        if let Some(block) = self.current.as_mut() {
            block.new.len = block.new.len.saturating_add(1);
        }
        self.new_cursor = self.new_cursor.saturating_add(1);
        self.new_store_cursor = self.new_store_cursor.saturating_add(1);
        self.last_side = Some(DiffSide::New);
    }

    fn ensure_block(&mut self, kind: BlockKind) {
        if self
            .current
            .as_ref()
            .is_some_and(|block| block.kind == kind)
        {
            return;
        }
        self.finish_block();
        self.current = Some(BlockBuilder {
            id: BlockId(self.next_block_id),
            kind,
            old_line_start: self.old_cursor,
            new_line_start: self.new_cursor,
            old: SourceRange::new(self.old_store_cursor, 0),
            new: SourceRange::new(self.new_store_cursor, 0),
            old_no_newline_at_end: false,
            new_no_newline_at_end: false,
        });
        self.next_block_id = self.next_block_id.saturating_add(1);
    }

    fn mark_old_no_newline(&mut self) {
        if let Some(block) = self.current.as_mut() {
            block.old_no_newline_at_end = true;
        }
    }

    fn mark_new_no_newline(&mut self) {
        if let Some(block) = self.current.as_mut() {
            block.new_no_newline_at_end = true;
        }
    }

    fn finish_block(&mut self) {
        let Some(block) = self.current.take() else {
            return;
        };
        self.blocks.push(block.finish());
    }
}

struct BlockBuilder {
    id: BlockId,
    kind: BlockKind,
    old_line_start: u32,
    new_line_start: u32,
    old: SourceRange,
    new: SourceRange,
    old_no_newline_at_end: bool,
    new_no_newline_at_end: bool,
}

impl BlockBuilder {
    fn finish(self) -> Block {
        let mut block = match self.kind {
            BlockKind::Context => Block::context(self.id, self.old, self.new),
            BlockKind::Change => Block::change(self.id, self.old, self.new),
        }
        .with_source_lines(self.old_line_start, self.new_line_start);
        block.old_no_newline_at_end = self.old_no_newline_at_end;
        block.new_no_newline_at_end = self.new_no_newline_at_end;
        block
    }
}

fn parse_hunk_header(line: &str) -> Result<(u32, u32, u32, u32), PatchError> {
    let mut parts = line.split_whitespace();
    if parts.next() != Some("@@") {
        return Err(PatchError::InvalidHunkHeader(line.to_owned()));
    }
    let Some(old_range) = parts.next() else {
        return Err(PatchError::InvalidHunkHeader(line.to_owned()));
    };
    let Some(new_range) = parts.next() else {
        return Err(PatchError::InvalidHunkHeader(line.to_owned()));
    };
    let Some("@@") = parts.next() else {
        return Err(PatchError::InvalidHunkHeader(line.to_owned()));
    };
    let (old_start, old_count) = parse_signed_range(old_range, '-')
        .ok_or_else(|| PatchError::InvalidHunkHeader(line.to_owned()))?;
    let (new_start, new_count) = parse_signed_range(new_range, '+')
        .ok_or_else(|| PatchError::InvalidHunkHeader(line.to_owned()))?;
    Ok((old_start, old_count, new_start, new_count))
}

fn parse_signed_range(range: &str, sign: char) -> Option<(u32, u32)> {
    let range = range.strip_prefix(sign)?;
    let (start, count) = range.split_once(',').unwrap_or((range, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

fn strip_patch_path(path: &str) -> Option<&str> {
    let path = path.trim();
    if path == "/dev/null" {
        return None;
    }
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .or(Some(path))
}

fn push_text_line(text: &mut String, content: &str) {
    // Malformed input can append lines after a no-newline marker already
    // trimmed the trailing separator; restore it so stored line counts stay
    // in sync with the block ranges counted by the hunk builder.
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(content);
    text.push('\n');
}

/// Drops the trailing separator so the final line is stored without a
/// newline. Leaves the text untouched and returns false when the final line
/// is empty: popping its separator would erase the line entirely and desync
/// stored line counts from the hunk's block ranges.
fn trim_trailing_newline(text: &mut String) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[bytes.len() - 1] == b'\n' && bytes[bytes.len() - 2] != b'\n' {
        text.pop();
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::parse_unified_patch;
    use crate::model::FileStatus;
    use crate::projection::{ExpansionState, ProjectionOptions, ProjectionRowKind, project_file};

    #[test]
    fn parses_git_patch_metadata_and_hunk_rows() {
        let patch = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,3 +10,3 @@
 context
-old
+new
 tail
";
        let document = parse_unified_patch(patch).unwrap();
        let file = &document.files[0];

        assert_eq!(file.path(), "src/lib.rs");
        assert_eq!(file.status, FileStatus::Modified);
        assert_eq!(file.old_oid.as_ref().unwrap().0, "1111111");
        assert_eq!(file.new_oid.as_ref().unwrap().0, "2222222");

        let mut rows = Vec::new();
        project_file(
            file,
            ProjectionOptions::default(),
            &ExpansionState::default(),
            |row| rows.push(row),
        );

        assert_eq!(rows[1].kind, ProjectionRowKind::Context);
        assert_eq!(rows[1].old_line, Some(10));
        assert_eq!(rows[2].kind, ProjectionRowKind::Removed);
        assert_eq!(rows[2].old_line, Some(11));
        assert_eq!(rows[3].kind, ProjectionRowKind::Added);
        assert_eq!(rows[3].new_line, Some(11));
    }

    #[test]
    fn parses_no_newline_marker() {
        let patch = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
";
        let document = parse_unified_patch(patch).unwrap();
        let file = &document.files[0];
        let block = &file.blocks[0];

        assert!(block.old_no_newline_at_end);
        assert!(block.new_no_newline_at_end);
        assert!(file.old_text.as_ref().unwrap().no_newline_at_eof());
        assert!(file.new_text.as_ref().unwrap().no_newline_at_eof());
    }

    #[test]
    fn parses_localized_no_newline_marker() {
        let patch = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old
\\ Pas de fin de ligne a la fin du fichier
+new
\\ Kein Zeilenumbruch am Dateiende
";
        let document = parse_unified_patch(patch).unwrap();
        let file = &document.files[0];
        let block = &file.blocks[0];

        assert!(block.old_no_newline_at_end);
        assert!(block.new_no_newline_at_end);
        assert!(file.old_text.as_ref().unwrap().no_newline_at_eof());
        assert!(file.new_text.as_ref().unwrap().no_newline_at_eof());
    }

    // Regression for a fuzz-found inconsistency: a malformed `\` line after a
    // no-newline marker was pushed as a context line into a store whose
    // trailing separator had been trimmed, so block ranges pointed one line
    // past the stored text.
    #[test]
    fn block_ranges_stay_within_text_stores_for_malformed_marker() {
        let patch = "\
diff --git a/a.txt b/a.txt
index 3333333..4444444 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
 first
-old end
\\ No newline at end of file
+new end
\\ N[ newline at end of file
";
        let document = parse_unified_patch(patch).unwrap();
        let file = &document.files[0];
        for block in &file.blocks {
            if block.old.len > 0 {
                let text = file.old_text.as_ref().unwrap();
                assert!(block.old.end() <= text.line_count());
            }
            if block.new.len > 0 {
                let text = file.new_text.as_ref().unwrap();
                assert!(block.new.end() <= text.line_count());
            }
        }
    }

    // Regression for a fuzz-found inconsistency: a no-newline marker after an
    // empty line trimmed the separator and erased the line from the text
    // store, leaving block ranges one line past the stored text.
    #[test]
    fn no_newline_marker_after_empty_line_keeps_counts_in_sync() {
        let patch = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
 first

\\ No newline at end of file
";
        let document = parse_unified_patch(patch).unwrap();
        let file = &document.files[0];
        for block in &file.blocks {
            if block.old.len > 0 {
                let text = file.old_text.as_ref().unwrap();
                assert!(block.old.end() <= text.line_count());
            }
            if block.new.len > 0 {
                let text = file.new_text.as_ref().unwrap();
                assert!(block.new.end() <= text.line_count());
            }
        }
    }
}
