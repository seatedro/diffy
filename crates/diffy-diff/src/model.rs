//! The diff document: files, hunks, and blocks as flat column tables.
//!
//! A file owns a contiguous range of hunks and a hunk a contiguous range of
//! blocks. A block is a run of context lines (the same on both sides) or a
//! change (old lines replaced by new lines, either side possibly empty).
//! Block lines live in the file's two [`TextStore`]s: the whole file for a
//! diff computed from two texts, or only the lines the patch shows for a
//! parsed patch (`partial`), so store indices and file line numbers are
//! kept separately.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use crate::text::TextStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FileStatus {
    #[default]
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
}

impl FileStatus {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Modified => "modified",
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Context,
    Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    Old,
    New,
}

/// Header facts about a file, as a patch states them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileMeta {
    pub old_path: Option<Arc<str>>,
    pub new_path: Option<Arc<str>>,
    pub status: FileStatus,
    pub binary: bool,
    pub old_mode: Option<Arc<str>>,
    pub new_mode: Option<Arc<str>>,
}

/// File columns. Read through [`DiffDocument::files`].
#[derive(Debug, Clone, Default)]
pub struct FileTable {
    pub meta: Vec<FileMeta>,
    /// The stores hold only the lines the patch shows.
    pub partial: Vec<bool>,
    pub old_text: Vec<TextStore>,
    pub new_text: Vec<TextStore>,
    pub additions: Vec<u32>,
    pub deletions: Vec<u32>,
    pub hunks: Vec<Range<u32>>,
}

/// Hunk columns. Starts and lengths are the header's: a start is the
/// one-based first line, or the line before the hunk when it is empty.
#[derive(Debug, Clone, Default)]
pub struct HunkTable {
    pub file: Vec<u32>,
    pub old_start: Vec<u32>,
    pub old_len: Vec<u32>,
    pub new_start: Vec<u32>,
    pub new_len: Vec<u32>,
    /// Text after the closing `@@`, often the enclosing function.
    pub section: Vec<Arc<str>>,
    pub blocks: Vec<Range<u32>>,
}

/// Block columns.
#[derive(Debug, Clone, Default)]
pub struct BlockTable {
    pub kind: Vec<BlockKind>,
    /// First line's index in the file's old and new stores.
    pub old_store: Vec<u32>,
    pub new_store: Vec<u32>,
    pub old_len: Vec<u32>,
    pub new_len: Vec<u32>,
    /// First line's one-based line number in the old and new file.
    pub old_line: Vec<u32>,
    pub new_line: Vec<u32>,
}

/// A whole diff: any number of files.
#[derive(Debug, Clone, Default)]
pub struct DiffDocument {
    files: FileTable,
    hunks: HunkTable,
    blocks: BlockTable,
}

/// A file's row of the file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSummary {
    pub index: u32,
    /// The new path, or the old one for a deleted file.
    pub path: Arc<str>,
    /// The old path when it differs (renames and copies).
    pub old_path: Option<Arc<str>>,
    pub status: FileStatus,
    pub binary: bool,
    pub additions: u32,
    pub deletions: u32,
}

/// What a file's diff changes besides its lines, for metadata rows: a
/// file whose only change is its mode, name, or binary content must not
/// read as an empty `+0 -0` diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    pub status: FileStatus,
    pub binary: bool,
    /// Old and new mode, when both are stated and differ.
    pub mode_change: Option<(Arc<str>, Arc<str>)>,
    /// Whether any hunk changes or shows text.
    pub has_hunks: bool,
    /// Whether each side's last line lacks a newline, as the whole source
    /// or the patch's marker states it.
    pub old_missing_newline: bool,
    pub new_missing_newline: bool,
}

/// Lines before a hunk side: its start, less one when it is not empty.
pub(crate) fn lines_before(start: u32, len: u32) -> u32 {
    start.saturating_sub(u32::from(len > 0))
}

impl DiffDocument {
    pub fn files(&self) -> &FileTable {
        &self.files
    }

    pub fn hunks(&self) -> &HunkTable {
        &self.hunks
    }

    pub fn blocks(&self) -> &BlockTable {
        &self.blocks
    }

    pub fn file_count(&self) -> u32 {
        self.files.meta.len() as u32
    }

    pub fn hunk_count(&self) -> u32 {
        self.hunks.file.len() as u32
    }

    pub fn text(&self, file: u32, side: Side) -> &TextStore {
        match side {
            Side::Old => &self.files.old_text[file as usize],
            Side::New => &self.files.new_text[file as usize],
        }
    }

    /// The path shown for `file`: the new path, else the old.
    pub fn path(&self, file: u32) -> &str {
        let meta = &self.files.meta[file as usize];
        meta.new_path
            .as_deref()
            .or(meta.old_path.as_deref())
            .unwrap_or("")
    }

    /// The file list with line stats.
    pub fn summaries(&self) -> impl ExactSizeIterator<Item = FileSummary> + '_ {
        (0..self.file_count()).map(|i| {
            let f = i as usize;
            let meta = &self.files.meta[f];
            let path: Arc<str> = meta
                .new_path
                .clone()
                .or_else(|| meta.old_path.clone())
                .unwrap_or_else(|| Arc::from(""));
            FileSummary {
                index: i,
                old_path: meta.old_path.clone().filter(|old| *old != path),
                path,
                status: meta.status,
                binary: meta.binary,
                additions: self.files.additions[f],
                deletions: self.files.deletions[f],
            }
        })
    }

    /// Status, binary, mode, and end-of-file facts of `file`.
    pub fn facts(&self, file: u32) -> FileFacts {
        let f = file as usize;
        let meta = &self.files.meta[f];
        let mode_change = match (&meta.old_mode, &meta.new_mode) {
            (Some(old), Some(new)) if old != new => Some((old.clone(), new.clone())),
            _ => None,
        };
        FileFacts {
            status: meta.status,
            binary: meta.binary,
            mode_change,
            has_hunks: !self.files.hunks[f].is_empty(),
            old_missing_newline: self.files.old_text[f].no_newline_at_eof(),
            new_missing_newline: self.files.new_text[f].no_newline_at_eof(),
        }
    }

    /// Added and deleted lines over every file.
    pub fn totals(&self) -> (u64, u64) {
        let add = self.files.additions.iter().map(|&n| u64::from(n)).sum();
        let del = self.files.deletions.iter().map(|&n| u64::from(n)).sum();
        (add, del)
    }

    /// Lines the gap above hunk `hunk` holds (between it and the previous
    /// hunk of its file, or the file start), on the new side. Zero for
    /// partial files, whose stores lack the lines.
    pub fn gap_above(&self, hunk: u32) -> u32 {
        let h = hunk as usize;
        let file = self.hunks.file[h];
        if self.files.partial[file as usize] {
            return 0;
        }
        let start = lines_before(self.hunks.new_start[h], self.hunks.new_len[h]);
        let prev_end = (self.files.hunks[file as usize].start < hunk).then(|| {
            let p = h - 1;
            lines_before(self.hunks.new_start[p], self.hunks.new_len[p]) + self.hunks.new_len[p]
        });
        start - prev_end.unwrap_or(0)
    }

    /// Lines after the last hunk of `file` to its end, on the new side.
    pub fn gap_after(&self, file: u32) -> u32 {
        let f = file as usize;
        let range = &self.files.hunks[f];
        if self.files.partial[f] {
            return 0;
        }
        // A file with full text but no hunks is unchanged: all of it is one
        // gap, so expanding it shows the whole file.
        if range.is_empty() {
            return self.files.new_text[f].line_count();
        }
        let h = range.end as usize - 1;
        let end =
            lines_before(self.hunks.new_start[h], self.hunks.new_len[h]) + self.hunks.new_len[h];
        self.files.new_text[f].line_count() - end
    }

    // ---- Building ------------------------------------------------------

    /// Starts a file; add its hunks with [`Self::push_hunk`] and close it
    /// with [`Self::finish_file`].
    pub(crate) fn begin_file(&mut self, meta: FileMeta, partial: bool) -> u32 {
        let hunks = self.hunk_count();
        let f = &mut self.files;
        f.meta.push(meta);
        f.partial.push(partial);
        f.old_text.push(TextStore::default());
        f.new_text.push(TextStore::default());
        f.additions.push(0);
        f.deletions.push(0);
        f.hunks.push(hunks..hunks);
        f.meta.len() as u32 - 1
    }

    /// Adds a hunk to the open file. `blocks` are `(kind, old_len,
    /// new_len)`; their lines follow `old_store`/`new_store` in the stores.
    pub(crate) fn push_hunk(
        &mut self,
        header: (u32, u32, u32, u32),
        section: Arc<str>,
        stores: (u32, u32),
        blocks: &[(BlockKind, u32, u32)],
    ) {
        let (old_start, old_len, new_start, new_len) = header;
        let file = self.file_count() - 1;
        let first = self.blocks.kind.len() as u32;
        let (mut old_store, mut new_store) = stores;
        let mut old_line = lines_before(old_start, old_len) + 1;
        let mut new_line = lines_before(new_start, new_len) + 1;
        let (mut adds, mut dels) = (0, 0);
        for &(kind, o, n) in blocks {
            let b = &mut self.blocks;
            b.kind.push(kind);
            b.old_store.push(old_store);
            b.new_store.push(new_store);
            b.old_len.push(o);
            b.new_len.push(n);
            b.old_line.push(old_line);
            b.new_line.push(new_line);
            old_store += o;
            new_store += n;
            old_line += o;
            new_line += n;
            if kind == BlockKind::Change {
                adds += n;
                dels += o;
            }
        }
        let h = &mut self.hunks;
        h.file.push(file);
        h.old_start.push(old_start);
        h.old_len.push(old_len);
        h.new_start.push(new_start);
        h.new_len.push(new_len);
        h.section.push(section);
        h.blocks.push(first..self.blocks.kind.len() as u32);
        let f = file as usize;
        self.files.hunks[f].end = self.hunk_count();
        self.files.additions[f] += adds;
        self.files.deletions[f] += dels;
    }

    pub(crate) fn finish_file(&mut self, old: TextStore, new: TextStore) {
        let f = self.file_count() as usize - 1;
        self.files.old_text[f] = old;
        self.files.new_text[f] = new;
        debug_assert_eq!(self.verify_file(f as u32), Ok(()));
    }

    /// Appends file `file` of `src`. With `full` stores the copy holds the
    /// whole texts, so its blocks index them by line number; the caller
    /// has checked that the texts match the hunks.
    pub(crate) fn push_file_from(
        &mut self,
        src: &DiffDocument,
        file: u32,
        full: Option<(TextStore, TextStore)>,
    ) {
        let f = file as usize;
        let (h, b) = (&src.hunks, &src.blocks);
        self.begin_file(
            src.files.meta[f].clone(),
            full.is_none() && src.files.partial[f],
        );
        let mut blocks = Vec::new();
        for hunk in src.files.hunks[f].clone() {
            let hi = hunk as usize;
            let range = h.blocks[hi].clone();
            blocks.clear();
            blocks.extend(range.clone().map(|bi| {
                let bi = bi as usize;
                (b.kind[bi], b.old_len[bi], b.new_len[bi])
            }));
            let stores = match (&full, range.is_empty()) {
                (Some(_), _) | (None, true) => (
                    lines_before(h.old_start[hi], h.old_len[hi]),
                    lines_before(h.new_start[hi], h.new_len[hi]),
                ),
                (None, false) => (
                    b.old_store[range.start as usize],
                    b.new_store[range.start as usize],
                ),
            };
            let header = (
                h.old_start[hi],
                h.old_len[hi],
                h.new_start[hi],
                h.new_len[hi],
            );
            self.push_hunk(header, h.section[hi].clone(), stores, &blocks);
        }
        let (old, new) =
            full.unwrap_or_else(|| (src.files.old_text[f].clone(), src.files.new_text[f].clone()));
        self.finish_file(old, new);
    }

    /// A one-file document holding file `file`, sharing its texts.
    pub fn extract_file(&self, file: u32) -> DiffDocument {
        let mut doc = DiffDocument::default();
        doc.push_file_from(self, file, None);
        doc
    }

    /// Appends every file of `other`.
    pub fn append(&mut self, other: DiffDocument) {
        let hunk_base = self.hunk_count();
        let block_base = self.blocks.kind.len() as u32;
        let file_base = self.file_count();
        let shift = |r: &Range<u32>, by: u32| r.start + by..r.end + by;
        let DiffDocument {
            files,
            hunks,
            blocks,
        } = other;
        self.files.meta.extend(files.meta);
        self.files.partial.extend(files.partial);
        self.files.old_text.extend(files.old_text);
        self.files.new_text.extend(files.new_text);
        self.files.additions.extend(files.additions);
        self.files.deletions.extend(files.deletions);
        self.files
            .hunks
            .extend(files.hunks.iter().map(|r| shift(r, hunk_base)));
        self.hunks
            .file
            .extend(hunks.file.iter().map(|f| f + file_base));
        self.hunks.old_start.extend(hunks.old_start);
        self.hunks.old_len.extend(hunks.old_len);
        self.hunks.new_start.extend(hunks.new_start);
        self.hunks.new_len.extend(hunks.new_len);
        self.hunks.section.extend(hunks.section);
        self.hunks
            .blocks
            .extend(hunks.blocks.iter().map(|r| shift(r, block_base)));
        let b = &mut self.blocks;
        b.kind.extend(blocks.kind);
        b.old_store.extend(blocks.old_store);
        b.new_store.extend(blocks.new_store);
        b.old_len.extend(blocks.old_len);
        b.new_len.extend(blocks.new_len);
        b.old_line.extend(blocks.old_line);
        b.new_line.extend(blocks.new_line);
        debug_assert_eq!(self.verify_integrity(), Ok(()));
    }

    // ---- Integrity -----------------------------------------------------

    /// Checks every table. O(files + hunks + blocks).
    pub fn verify_integrity(&self) -> Result<(), IntegrityError> {
        let f = &self.files;
        let n = f.meta.len();
        let file_columns = [
            f.partial.len(),
            f.old_text.len(),
            f.new_text.len(),
            f.additions.len(),
            f.deletions.len(),
            f.hunks.len(),
        ];
        let h = &self.hunks;
        let hunk_columns = [
            h.old_start.len(),
            h.old_len.len(),
            h.new_start.len(),
            h.new_len.len(),
            h.section.len(),
            h.blocks.len(),
        ];
        let b = &self.blocks;
        let block_columns = [
            b.old_store.len(),
            b.new_store.len(),
            b.old_len.len(),
            b.new_len.len(),
            b.old_line.len(),
            b.new_line.len(),
        ];
        if file_columns.iter().any(|&len| len != n)
            || hunk_columns.iter().any(|&len| len != h.file.len())
            || block_columns.iter().any(|&len| len != b.kind.len())
        {
            return Err(IntegrityError::ColumnLength);
        }
        let mut next_hunk = 0;
        for file in 0..n as u32 {
            if f.hunks[file as usize].start != next_hunk {
                return Err(IntegrityError::HunkRange { file });
            }
            next_hunk = f.hunks[file as usize].end;
            self.verify_file(file)?;
        }
        if next_hunk != self.hunk_count() {
            return Err(IntegrityError::HunkRange { file: n as u32 });
        }
        let mut next_block = 0;
        for (hunk, blocks) in h.blocks[..self.hunk_count() as usize].iter().enumerate() {
            if blocks.start != next_block {
                return Err(IntegrityError::BlockRange { hunk: hunk as u32 });
            }
            next_block = blocks.end;
        }
        if next_block as usize != b.kind.len() {
            return Err(IntegrityError::BlockRange {
                hunk: self.hunk_count(),
            });
        }
        Ok(())
    }

    /// Checks one file's hunks and blocks against its stores and stats.
    fn verify_file(&self, file: u32) -> Result<(), IntegrityError> {
        let (f, h, b) = (&self.files, &self.hunks, &self.blocks);
        let fi = file as usize;
        let err = |hunk: usize, what: &'static str| IntegrityError::Hunk {
            file,
            hunk: hunk as u32,
            what,
        };
        let partial = f.partial[fi];
        let (old_lines, new_lines) = (f.old_text[fi].line_count(), f.new_text[fi].line_count());
        let (mut old_end, mut new_end) = (0, 0);
        let (mut old_store, mut new_store) = (0, 0);
        let (mut adds, mut dels) = (0, 0);
        for hunk in f.hunks[fi].clone() {
            let hi = hunk as usize;
            if h.file[hi] != file {
                return Err(err(hi, "owner"));
            }
            let old_before = lines_before(h.old_start[hi], h.old_len[hi]);
            let new_before = lines_before(h.new_start[hi], h.new_len[hi]);
            if old_before < old_end || new_before < new_end {
                return Err(err(hi, "order"));
            }
            let (mut old_line, mut new_line) = (old_before + 1, new_before + 1);
            let mut last_kind = None;
            for block in h.blocks[hi].clone() {
                let bi = block as usize;
                let (o, n) = (b.old_len[bi], b.new_len[bi]);
                if last_kind == Some(b.kind[bi])
                    || (o == 0 && n == 0)
                    || (b.kind[bi] == BlockKind::Context && o != n)
                {
                    return Err(err(hi, "block shape"));
                }
                last_kind = Some(b.kind[bi]);
                if b.old_line[bi] != old_line || b.new_line[bi] != new_line {
                    return Err(err(hi, "line numbers"));
                }
                let stores_follow = if partial {
                    b.old_store[bi] == old_store && b.new_store[bi] == new_store
                } else {
                    b.old_store[bi] == old_line - 1 && b.new_store[bi] == new_line - 1
                };
                if !stores_follow {
                    return Err(err(hi, "store index"));
                }
                if b.old_store[bi] + o > old_lines || b.new_store[bi] + n > new_lines {
                    return Err(err(hi, "store bounds"));
                }
                old_line += o;
                new_line += n;
                old_store = b.old_store[bi] + o;
                new_store = b.new_store[bi] + n;
                if b.kind[bi] == BlockKind::Change {
                    adds += n;
                    dels += o;
                }
            }
            if old_line - 1 - old_before != h.old_len[hi]
                || new_line - 1 - new_before != h.new_len[hi]
            {
                return Err(err(hi, "length"));
            }
            old_end = old_before + h.old_len[hi];
            new_end = new_before + h.new_len[hi];
        }
        if partial && (old_store != old_lines || new_store != new_lines) {
            return Err(IntegrityError::Stores { file });
        }
        if adds != f.additions[fi] || dels != f.deletions[fi] {
            return Err(IntegrityError::Stats { file });
        }
        Ok(())
    }
}

/// A broken [`DiffDocument`] invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityError {
    ColumnLength,
    /// The file's hunks do not follow the previous file's.
    HunkRange {
        file: u32,
    },
    /// The hunk's blocks do not follow the previous hunk's.
    BlockRange {
        hunk: u32,
    },
    Hunk {
        file: u32,
        hunk: u32,
        what: &'static str,
    },
    /// A partial file's stores hold lines no block uses.
    Stores {
        file: u32,
    },
    Stats {
        file: u32,
    },
}

impl fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "diff integrity: {self:?}")
    }
}

impl std::error::Error for IntegrityError {}
