use crate::inline::InlineSpan;
use crate::text::{TextStore, u32_to_usize_saturating, usize_to_u32_saturating};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct HunkId(pub u32);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffSide {
    Old,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileMode(pub String);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FileStatus {
    #[default]
    Modified,
    Added,
    Deleted,
    Renamed,
    RenamedModified,
    ModeChanged,
    Binary,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SourceRange {
    /// Zero-based line index in the side-specific text store.
    pub start: u32,
    pub len: u32,
}

impl SourceRange {
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    pub const fn end(self) -> u32 {
        self.start.saturating_add(self.len)
    }

    pub const fn contains_index(self, index: u32) -> bool {
        index >= self.start && index < self.end()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct BlockRange {
    pub start: u32,
    pub len: u32,
}

impl BlockRange {
    pub const fn end(self) -> u32 {
        self.start.saturating_add(self.len)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Context,
    Change,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    /// One-based source line number for the first old-side line in this block.
    pub old_line_start: u32,
    /// One-based source line number for the first new-side line in this block.
    pub new_line_start: u32,
    pub old: SourceRange,
    pub new: SourceRange,
    pub old_no_newline_at_end: bool,
    pub new_no_newline_at_end: bool,
    pub old_inline: Vec<InlineSpan>,
    pub new_inline: Vec<InlineSpan>,
}

impl Block {
    pub fn context(id: BlockId, old: SourceRange, new: SourceRange) -> Self {
        Self {
            id,
            kind: BlockKind::Context,
            old_line_start: old.start.saturating_add(1),
            new_line_start: new.start.saturating_add(1),
            old,
            new,
            old_no_newline_at_end: false,
            new_no_newline_at_end: false,
            old_inline: Vec::new(),
            new_inline: Vec::new(),
        }
    }

    pub fn change(id: BlockId, old: SourceRange, new: SourceRange) -> Self {
        Self {
            id,
            kind: BlockKind::Change,
            old_line_start: old.start.saturating_add(1),
            new_line_start: new.start.saturating_add(1),
            old,
            new,
            old_no_newline_at_end: false,
            new_no_newline_at_end: false,
            old_inline: Vec::new(),
            new_inline: Vec::new(),
        }
    }

    pub fn with_source_lines(mut self, old_line_start: u32, new_line_start: u32) -> Self {
        self.old_line_start = old_line_start;
        self.new_line_start = new_line_start;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub id: HunkId,
    /// One-based line number in the old file.
    pub old_start: u32,
    pub old_count: u32,
    /// One-based line number in the new file.
    pub new_start: u32,
    pub new_count: u32,
    pub header: String,
    pub blocks: BlockRange,
}

impl Hunk {
    pub fn new(
        id: HunkId,
        old_start: u32,
        old_count: u32,
        new_start: u32,
        new_count: u32,
        blocks: BlockRange,
    ) -> Self {
        Self {
            id,
            old_start,
            old_count,
            new_start,
            new_count,
            header: format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@"),
            blocks,
        }
    }

    pub const fn old_start_index(&self) -> u32 {
        self.old_start.saturating_sub(1)
    }

    pub const fn new_start_index(&self) -> u32 {
        self.new_start.saturating_sub(1)
    }

    pub const fn old_end_index(&self) -> u32 {
        self.old_start_index().saturating_add(self.old_count)
    }

    pub const fn new_end_index(&self) -> u32 {
        self.new_start_index().saturating_add(self.new_count)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub id: FileId,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_oid: Option<ObjectId>,
    pub new_oid: Option<ObjectId>,
    pub old_mode: Option<FileMode>,
    pub new_mode: Option<FileMode>,
    pub status: FileStatus,
    pub is_binary: bool,
    pub is_partial: bool,
    pub old_text: Option<TextStore>,
    pub new_text: Option<TextStore>,
    pub hunks: Vec<Hunk>,
    pub blocks: Vec<Block>,
}

impl FileDiff {
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }

    pub fn side_text(&self, side: DiffSide) -> Option<&TextStore> {
        match side {
            DiffSide::Old => self.old_text.as_ref(),
            DiffSide::New => self.new_text.as_ref(),
        }
    }

    pub fn block(&self, id: BlockId) -> Option<&Block> {
        self.blocks
            .get(u32_to_usize_saturating(id.0))
            .filter(|block| block.id == id)
    }

    pub fn hunk(&self, id: HunkId) -> Option<&Hunk> {
        self.hunks
            .get(u32_to_usize_saturating(id.0))
            .filter(|hunk| hunk.id == id)
    }

    pub fn hunk_blocks(&self, hunk: &Hunk) -> &[Block] {
        let start = u32_to_usize_saturating(hunk.blocks.start);
        let end = u32_to_usize_saturating(hunk.blocks.end());
        self.blocks.get(start..end).unwrap_or(&[])
    }

    pub fn add_hunk(&mut self, mut hunk: Hunk, blocks: impl IntoIterator<Item = Block>) {
        let start = usize_to_u32_saturating(self.blocks.len());
        self.blocks.extend(blocks);
        let end = usize_to_u32_saturating(self.blocks.len());
        hunk.blocks = BlockRange {
            start,
            len: end.saturating_sub(start),
        };
        self.hunks.push(hunk);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffDocument {
    pub files: Vec<FileDiff>,
}

#[cfg(test)]
mod tests {
    use super::{Block, BlockId, BlockRange, DiffSide, FileDiff, Hunk, HunkId, SourceRange};
    use crate::text::TextStore;

    #[test]
    fn file_stores_side_text_and_blocks_by_compact_ids() {
        let mut file = FileDiff {
            old_text: Some(TextStore::from_text("old\n")),
            new_text: Some(TextStore::from_text("new\n")),
            ..FileDiff::default()
        };
        file.add_hunk(
            Hunk::new(HunkId(0), 1, 1, 1, 1, BlockRange::default()),
            [Block::change(
                BlockId(0),
                SourceRange::new(0, 1),
                SourceRange::new(0, 1),
            )],
        );

        assert_eq!(file.side_text(DiffSide::Old).unwrap().line_count(), 1);
        assert_eq!(file.block(BlockId(0)).unwrap().old.start, 0);
        assert_eq!(file.hunk_blocks(&file.hunks[0]).len(), 1);
    }
}
