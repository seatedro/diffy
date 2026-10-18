//! Unified diff text: parsing (git's format and plain `diff -u`), writing
//! in git's format, and applying a parsed file diff to its old text.
//!
//! The parser reads hunk bodies by the header's line counts, as `git apply`
//! does, so a removed line that reads `--- a/x` stays a removed line.

use std::fmt;
use std::sync::Arc;

use crate::model::{BlockKind, DiffDocument, FileMeta, FileStatus, Side, lines_before};
use crate::text::TextStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// No file diff in the input.
    Empty,
    /// One-based input line of a malformed `@@` header.
    InvalidHunkHeader { line: usize },
    /// The input ended inside a hunk body.
    TruncatedHunk { line: usize },
    /// A line inside a hunk body that is not ` `, `-`, `+`, or `\`, or that
    /// overruns the header's counts.
    MalformedHunkLine { line: usize },
    /// A hunk that starts before the previous one ends.
    HunkOutOfOrder { line: usize },
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("patch contains no file diffs"),
            Self::InvalidHunkHeader { line } => write!(f, "line {line}: invalid hunk header"),
            Self::TruncatedHunk { line } => write!(f, "line {line}: patch ends inside a hunk"),
            Self::MalformedHunkLine { line } => write!(f, "line {line}: malformed hunk line"),
            Self::HunkOutOfOrder { line } => write!(f, "line {line}: hunk out of order"),
        }
    }
}

impl std::error::Error for PatchError {}

/// Parses a unified diff of any number of files.
pub fn parse_unified(input: &str) -> Result<DiffDocument, PatchError> {
    let mut lines = input
        .split_inclusive('\n')
        .map(|l| l.strip_suffix('\n').unwrap_or(l))
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .peekable();
    let mut doc = DiffDocument::default();
    let mut file: Option<FileBuilder> = None;
    // Inside a `GIT binary patch` payload, which runs to the next file.
    let mut skipping_binary = false;
    while let Some((number, line)) = lines.next() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            finish(&mut doc, file.take());
            skipping_binary = false;
            let mut builder = FileBuilder::default();
            if let Some((old, new)) = git_header_paths(trim_cr(rest)) {
                builder.meta.old_path = Some(old.into());
                builder.meta.new_path = Some(new.into());
            }
            builder.git = true;
            file = Some(builder);
            continue;
        }
        if skipping_binary {
            continue;
        }
        if line.starts_with("--- ")
            && lines
                .peek()
                .is_some_and(|(_, next)| next.starts_with("+++ "))
        {
            let (_, plus) = lines.next().unwrap_or_default();
            // A plain `diff -u` file starts here; a git one already did.
            if file
                .as_ref()
                .is_none_or(|f| !f.git || f.hunks > 0 || f.saw_paths)
            {
                finish(&mut doc, file.take());
                file = Some(FileBuilder::default());
            }
            let builder = file.as_mut().expect("file just ensured");
            builder.saw_paths = true;
            builder.set_paths(&line[4..], &plus[4..]);
            continue;
        }
        if line.starts_with("@@ ") {
            let builder = file.get_or_insert_with(FileBuilder::default);
            builder.read_hunk(number, line, &mut lines)?;
            continue;
        }
        let Some(builder) = file.as_mut() else {
            continue;
        };
        if builder.hunks > 0 {
            // Between hunks only `@@` and the next file matter.
            continue;
        }
        if line.starts_with("GIT binary patch") {
            builder.meta.binary = true;
            skipping_binary = true;
        } else {
            builder.metadata(trim_cr(line));
        }
    }
    finish(&mut doc, file);
    if doc.file_count() == 0 {
        return Err(PatchError::Empty);
    }
    debug_assert_eq!(doc.verify_integrity(), Ok(()));
    Ok(doc)
}

fn trim_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn finish(doc: &mut DiffDocument, file: Option<FileBuilder>) {
    if let Some(file) = file {
        file.finish(doc);
    }
}

/// A parsed hunk waiting for its file to finish.
struct PendingHunk {
    header: (u32, u32, u32, u32),
    section: Arc<str>,
    stores: (u32, u32),
    blocks: Vec<(BlockKind, u32, u32)>,
}

#[derive(Default)]
struct FileBuilder {
    meta: FileMeta,
    git: bool,
    saw_paths: bool,
    hunks: u32,
    pending: Vec<PendingHunk>,
    old_text: String,
    new_text: String,
    old_lines: u32,
    new_lines: u32,
    /// A no-newline marker closed this side; no line may follow.
    old_closed: bool,
    new_closed: bool,
    /// Lines before the end of the last hunk, per side.
    old_end: u32,
    new_end: u32,
}

impl FileBuilder {
    fn set_paths(&mut self, minus: &str, plus: &str) {
        let old = patch_path(minus);
        let new = patch_path(plus);
        if old.is_none() {
            self.meta.status = FileStatus::Added;
        }
        if new.is_none() {
            self.meta.status = FileStatus::Deleted;
        }
        self.meta.old_path = old.map(Arc::from);
        self.meta.new_path = new.map(Arc::from);
    }

    fn metadata(&mut self, line: &str) {
        let m = &mut self.meta;
        if let Some(mode) = line.strip_prefix("new file mode ") {
            m.status = FileStatus::Added;
            m.new_mode = Some(mode.into());
            m.old_path = None;
        } else if let Some(mode) = line.strip_prefix("deleted file mode ") {
            m.status = FileStatus::Deleted;
            m.old_mode = Some(mode.into());
            m.new_path = None;
        } else if let Some(mode) = line.strip_prefix("old mode ") {
            m.old_mode = Some(mode.into());
        } else if let Some(mode) = line.strip_prefix("new mode ") {
            m.new_mode = Some(mode.into());
        } else if let Some(path) = line.strip_prefix("rename from ") {
            m.status = FileStatus::Renamed;
            m.old_path = Some(unquote(path).into());
        } else if let Some(path) = line.strip_prefix("rename to ") {
            m.status = FileStatus::Renamed;
            m.new_path = Some(unquote(path).into());
        } else if let Some(path) = line.strip_prefix("copy from ") {
            m.status = FileStatus::Copied;
            m.old_path = Some(unquote(path).into());
        } else if let Some(path) = line.strip_prefix("copy to ") {
            m.status = FileStatus::Copied;
            m.new_path = Some(unquote(path).into());
        } else if line.starts_with("Binary files ") && line.ends_with(" differ") {
            m.binary = true;
        }
    }

    /// Reads one hunk: the header at `number`, then exactly the lines its
    /// counts promise, plus no-newline markers.
    fn read_hunk<'a>(
        &mut self,
        number: usize,
        header: &str,
        lines: &mut std::iter::Peekable<impl Iterator<Item = (usize, &'a str)>>,
    ) -> Result<(), PatchError> {
        let (old_start, old_len, new_start, new_len, section) = parse_hunk_header(trim_cr(header))
            .ok_or(PatchError::InvalidHunkHeader { line: number })?;
        let (old_before, new_before) = (
            lines_before(old_start, old_len),
            lines_before(new_start, new_len),
        );
        if old_before < self.old_end
            || new_before < self.new_end
            || self.old_closed
            || self.new_closed
        {
            return Err(PatchError::HunkOutOfOrder { line: number });
        }
        let stores = (self.old_lines, self.new_lines);
        let (mut old_left, mut new_left) = (old_len, new_len);
        let mut blocks: Vec<(BlockKind, u32, u32)> = Vec::new();
        // Which sides the last line went to, for a following marker.
        let mut last: (bool, bool) = (false, false);
        let mut at = number;
        while old_left > 0 || new_left > 0 || lines.peek().is_some_and(|(_, l)| l.starts_with('\\'))
        {
            let Some((n, line)) = lines.next() else {
                return Err(PatchError::TruncatedHunk { line: at + 1 });
            };
            at = n;
            let malformed = PatchError::MalformedHunkLine { line: n };
            // Editors strip the space of empty context lines.
            let (tag, content) = match line.as_bytes().first() {
                None => (b' ', ""),
                Some(&tag) => (tag, &line[1..]),
            };
            let (old, new) = match tag {
                b'\\' => {
                    self.close(last).ok_or(malformed)?;
                    last = (false, false);
                    continue;
                }
                b' ' => (true, true),
                b'-' => (true, false),
                b'+' => (false, true),
                _ => return Err(malformed),
            };
            if (old && (old_left == 0 || self.old_closed))
                || (new && (new_left == 0 || self.new_closed))
            {
                return Err(malformed);
            }
            let kind = if old && new {
                BlockKind::Context
            } else {
                BlockKind::Change
            };
            if blocks.last().is_none_or(|b| b.0 != kind) {
                blocks.push((kind, 0, 0));
            }
            let block = blocks.last_mut().expect("block just ensured");
            if old {
                push_line(&mut self.old_text, content);
                self.old_lines += 1;
                old_left -= 1;
                block.1 += 1;
            }
            if new {
                push_line(&mut self.new_text, content);
                self.new_lines += 1;
                new_left -= 1;
                block.2 += 1;
            }
            last = (old, new);
        }
        self.old_end = old_before + old_len;
        self.new_end = new_before + new_len;
        self.hunks += 1;
        self.pending.push(PendingHunk {
            header: (old_start, old_len, new_start, new_len),
            section: section.into(),
            stores,
            blocks,
        });
        Ok(())
    }

    /// Applies a no-newline marker to the sides the last line went to.
    /// `None` when there is no such line or it is empty: an empty last line
    /// without a newline cannot exist, and dropping its separator would
    /// drop the line from the store.
    fn close(&mut self, (old, new): (bool, bool)) -> Option<()> {
        if !old && !new {
            return None;
        }
        for (side, text, closed) in [
            (old, &mut self.old_text, &mut self.old_closed),
            (new, &mut self.new_text, &mut self.new_closed),
        ] {
            if side {
                if text.ends_with("\n\n") || text == "\n" {
                    return None;
                }
                text.pop();
                *closed = true;
            }
        }
        Some(())
    }

    fn finish(self, doc: &mut DiffDocument) {
        let mut meta = self.meta;
        if meta.status == FileStatus::Modified
            && meta.old_path.is_some()
            && meta.new_path.is_some()
            && meta.old_path != meta.new_path
        {
            meta.status = FileStatus::Renamed;
        }
        doc.begin_file(meta, true);
        for hunk in &self.pending {
            doc.push_hunk(hunk.header, hunk.section.clone(), hunk.stores, &hunk.blocks);
        }
        doc.finish_file(TextStore::new(self.old_text), TextStore::new(self.new_text));
    }
}

fn push_line(text: &mut String, content: &str) {
    text.push_str(content);
    text.push('\n');
}

/// `@@ -a[,b] +c[,d] @@ section`.
fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32, &str)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, section) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let range = |r: &str| -> Option<(u32, u32)> {
        let (start, len) = r.split_once(',').unwrap_or((r, "1"));
        Some((start.parse().ok()?, len.parse().ok()?))
    };
    let ((os, ol), (ns, nl)) = (range(old)?, range(new)?);
    // A non-empty side starts at line 1 or later.
    if (ol > 0 && os == 0) || (nl > 0 && ns == 0) {
        return None;
    }
    Some((os, ol, ns, nl, section.strip_prefix(' ').unwrap_or(section)))
}

/// A `---`/`+++` path: `None` for `/dev/null`, else without the `a/` or
/// `b/` prefix and a trailing tab and timestamp.
fn patch_path(raw: &str) -> Option<String> {
    let raw = trim_cr(raw);
    let raw = raw.split('\t').next().unwrap_or(raw);
    let path = unquote(raw);
    if path == "/dev/null" {
        return None;
    }
    Some(strip_prefix(&path).to_owned())
}

fn strip_prefix(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

/// Undoes git's C-style quoting of paths with unusual bytes.
fn unquote(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_owned();
    };
    let mut bytes = Vec::with_capacity(inner.len());
    let mut it = inner.bytes().peekable();
    while let Some(b) = it.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        match it.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(d @ b'0'..=b'7') => {
                let mut v = u32::from(d - b'0');
                for _ in 0..2 {
                    match it.peek() {
                        Some(&o @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(o - b'0');
                            it.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(v as u8);
            }
            Some(other) => bytes.push(other),
            None => {}
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Old and new path of `a/x b/y`. Paths may hold spaces, so a header whose
/// halves name the same file is split in the middle first.
fn git_header_paths(rest: &str) -> Option<(String, String)> {
    if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find("\" ")? + 1;
        let (old, new) = (&rest[..=end], rest[end + 1..].trim_start());
        return Some((
            strip_prefix(&unquote(old)).to_owned(),
            strip_prefix(&unquote(new)).to_owned(),
        ));
    }
    let mid = rest.len() / 2;
    if rest.len() % 2 == 1 && rest.as_bytes().get(mid) == Some(&b' ') {
        let (old, new) = (strip_prefix(&rest[..mid]), strip_prefix(&rest[mid + 1..]));
        if old == new {
            return Some((old.to_owned(), new.to_owned()));
        }
    }
    let split = rest.find(" b/")?;
    Some((
        strip_prefix(&rest[..split]).to_owned(),
        rest[split + 3..].to_owned(),
    ))
}

// ---- Writing --------------------------------------------------------------

/// The document as a git-style unified diff, which [`parse_unified`]
/// reads back.
pub fn write_unified(doc: &DiffDocument) -> String {
    let mut out = String::new();
    for file in 0..doc.file_count() {
        write_file(doc, file, &mut out);
    }
    out
}

fn write_file(doc: &DiffDocument, file: u32, out: &mut String) {
    let f = file as usize;
    let meta = &doc.files().meta[f];
    let old = meta.old_path.as_deref();
    let new = meta.new_path.as_deref();
    let (a, b) = (old.or(new).unwrap_or(""), new.or(old).unwrap_or(""));
    out.push_str(&format!("diff --git a/{a} b/{b}\n"));
    match meta.status {
        FileStatus::Added => {
            let mode = meta.new_mode.as_deref().unwrap_or("100644");
            out.push_str(&format!("new file mode {mode}\n"));
        }
        FileStatus::Deleted => {
            let mode = meta.old_mode.as_deref().unwrap_or("100644");
            out.push_str(&format!("deleted file mode {mode}\n"));
        }
        FileStatus::Renamed | FileStatus::Copied => {
            if let (Some(o), Some(n)) = (&meta.old_mode, &meta.new_mode)
                && o != n
            {
                out.push_str(&format!("old mode {o}\nnew mode {n}\n"));
            }
            let verb = if meta.status == FileStatus::Renamed {
                "rename"
            } else {
                "copy"
            };
            out.push_str(&format!("{verb} from {a}\n{verb} to {b}\n"));
        }
        FileStatus::Modified => {
            if let (Some(o), Some(n)) = (&meta.old_mode, &meta.new_mode)
                && o != n
            {
                out.push_str(&format!("old mode {o}\nnew mode {n}\n"));
            }
        }
    }
    let minus = old.map_or_else(|| "/dev/null".to_owned(), |p| format!("a/{p}"));
    let plus = new.map_or_else(|| "/dev/null".to_owned(), |p| format!("b/{p}"));
    if meta.binary {
        out.push_str(&format!("Binary files {minus} and {plus} differ\n"));
        return;
    }
    let range = doc.files().hunks[f].clone();
    if range.is_empty() {
        return;
    }
    out.push_str(&format!("--- {minus}\n+++ {plus}\n"));
    let (h, bl) = (doc.hunks(), doc.blocks());
    let (old_text, new_text) = (doc.text(file, Side::Old), doc.text(file, Side::New));
    let count = |start: u32, len: u32| {
        if len == 1 {
            start.to_string()
        } else {
            format!("{start},{len}")
        }
    };
    for hunk in range {
        let hi = hunk as usize;
        let section = &h.section[hi];
        out.push_str(&format!(
            "@@ -{} +{} @@{}{section}\n",
            count(h.old_start[hi], h.old_len[hi]),
            count(h.new_start[hi], h.new_len[hi]),
            if section.is_empty() { "" } else { " " },
        ));
        for block in h.blocks[hi].clone() {
            let bi = block as usize;
            let (os, ns) = (bl.old_store[bi], bl.new_store[bi]);
            let line = |out: &mut String, tag: char, store: &TextStore, i: u32, closes: bool| {
                out.push(tag);
                out.push_str(store.line(i).unwrap_or(""));
                out.push('\n');
                if closes {
                    out.push_str("\\ No newline at end of file\n");
                }
            };
            let last_old = |i: u32| i + 1 == old_text.line_count() && old_text.no_newline_at_eof();
            let last_new = |i: u32| i + 1 == new_text.line_count() && new_text.no_newline_at_eof();
            match bl.kind[bi] {
                BlockKind::Context => {
                    for k in 0..bl.old_len[bi] {
                        // Both sides end here without a newline, or the
                        // line would be a change.
                        line(out, ' ', old_text, os + k, last_old(os + k));
                    }
                }
                BlockKind::Change => {
                    for k in 0..bl.old_len[bi] {
                        line(out, '-', old_text, os + k, last_old(os + k));
                    }
                    for k in 0..bl.new_len[bi] {
                        line(out, '+', new_text, ns + k, last_new(ns + k));
                    }
                }
            }
        }
    }
}

// ---- Applying -------------------------------------------------------------

/// Why [`apply`] could not apply a file diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// An old-side line of the diff does not match the text at this
    /// one-based line.
    Mismatch { line: u32 },
    /// A hunk reaches past the end of the text.
    OutOfRange { line: u32 },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mismatch { line } => write!(f, "line {line} does not match the diff"),
            Self::OutOfRange { line } => write!(f, "hunk at line {line} is past the end"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// Applies `file`'s hunks to `old`, which must match their old side
/// exactly, and returns the new text.
pub fn apply(doc: &DiffDocument, file: u32, old: &str) -> Result<String, ApplyError> {
    let source = TextStore::new(old);
    let (old_store, new_store) = (doc.text(file, Side::Old), doc.text(file, Side::New));
    let (h, b) = (doc.hunks(), doc.blocks());
    let mut out = String::with_capacity(old.len());
    let mut at = 0u32;
    // Whether `out` ends with a line whose newline is still owed.
    let mut open = false;
    let push = |out: &mut String, line: &str, open: &mut bool| {
        if *open {
            out.push('\n');
        }
        out.push_str(line);
        *open = true;
    };
    let ranges = doc.files().hunks[file as usize].clone();
    let last_hunk = ranges.end.checked_sub(1);
    let mut ends_at_eof = false;
    for hunk in ranges {
        let hi = hunk as usize;
        let before = lines_before(h.old_start[hi], h.old_len[hi]);
        if before > source.line_count() || before < at {
            return Err(ApplyError::OutOfRange { line: before + 1 });
        }
        for i in at..before {
            push(&mut out, source.line(i).unwrap_or(""), &mut open);
        }
        at = before;
        for block in h.blocks[hi].clone() {
            let bi = block as usize;
            for k in 0..b.old_len[bi] {
                let expected = old_store.line(b.old_store[bi] + k);
                let actual = source.line(at);
                if actual.is_none() {
                    return Err(ApplyError::OutOfRange { line: at + 1 });
                }
                if expected != actual {
                    return Err(ApplyError::Mismatch { line: at + 1 });
                }
                if b.kind[bi] == BlockKind::Context {
                    push(&mut out, actual.unwrap_or(""), &mut open);
                }
                at += 1;
            }
            if b.kind[bi] == BlockKind::Change {
                for k in 0..b.new_len[bi] {
                    push(
                        &mut out,
                        new_store.line(b.new_store[bi] + k).unwrap_or(""),
                        &mut open,
                    );
                }
            }
        }
        if Some(hunk) == last_hunk {
            ends_at_eof = at == source.line_count();
        }
    }
    for i in at..source.line_count() {
        push(&mut out, source.line(i).unwrap_or(""), &mut open);
    }
    // The last hunk decides the final newline when it reaches the end;
    // otherwise the untouched tail keeps the old text's.
    let newline = if ends_at_eof {
        !new_store.no_newline_at_eof()
    } else {
        !source.no_newline_at_eof()
    };
    if open && newline {
        out.push('\n');
    }
    Ok(out)
}
