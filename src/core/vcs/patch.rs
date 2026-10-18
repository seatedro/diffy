use std::collections::HashSet;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use crate::core::vcs::mutation::{PatchSelection, Side, SideRange};
use crate::core::vcs::source::{ObjectId, SourceIdentity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    Malformed(&'static str),
    FileNotInPatch,
    Binary,
    NoLineChanges,
    NothingSelected,
    Stale,
    AmbiguousHunk,
    NewlineConflict,
    Unsupported(&'static str),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(what) => write!(f, "The repository diff could not be read: {what}."),
            Self::FileNotInPatch => f.write_str("This file has no changes in the repository diff."),
            Self::Binary => f.write_str("Binary files can only be changed as a whole file."),
            Self::NoLineChanges => f.write_str("This file has no line changes to apply."),
            Self::NothingSelected => f.write_str("Select changed lines first."),
            Self::Stale => f.write_str(
                "The selection no longer matches the file on disk. Refresh and try again.",
            ),
            Self::AmbiguousHunk => f.write_str(
                "This hunk does not match the repository diff. Select its lines instead.",
            ),
            Self::NewlineConflict => f.write_str(
                "The selection would change the final newline. Include the last line of the file.",
            ),
            Self::Unsupported(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for PatchError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactLine {
    pub kind: LineKind,
    pub bytes: Range<usize>,
    pub no_newline: Option<Range<usize>>,
    pub old: Option<u32>,
    pub new: Option<u32>,
}

impl ExactLine {
    fn on_side(&self, side: Side) -> bool {
        match self.kind {
            LineKind::Context => true,
            LineKind::Removed => side == Side::Old,
            LineKind::Added => side == Side::New,
        }
    }

    fn change_line(&self, side: Side) -> Option<u32> {
        match (self.kind, side) {
            (LineKind::Removed, Side::Old) => self.old,
            (LineKind::Added, Side::New) => self.new,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactHunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub section: Range<usize>,
    pub lines: Vec<ExactLine>,
}

impl ExactHunk {
    fn span(&self, side: Side) -> Range<u32> {
        let (start, len) = match side {
            Side::Old => (self.old_start, self.old_len),
            Side::New => (self.new_start, self.new_len),
        };
        let before = lines_before(start, len);
        before..before + len
    }

    pub fn additions(&self) -> u32 {
        self.lines
            .iter()
            .filter(|line| line.kind == LineKind::Added)
            .count() as u32
    }

    pub fn deletions(&self) -> u32 {
        self.lines
            .iter()
            .filter(|line| line.kind == LineKind::Removed)
            .count() as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderKind {
    DiffGit,
    OldPath,
    NewPath,
    NewFile,
    DeletedFile,
    Mode,
    Rename,
    Index,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HeaderLine {
    kind: HeaderKind,
    bytes: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactFile {
    bytes: Arc<[u8]>,
    header: Vec<HeaderLine>,
    old_path: Option<Vec<u8>>,
    new_path: Option<Vec<u8>>,
    pub new_file: bool,
    pub deleted_file: bool,
    pub renamed: bool,
    pub binary: bool,
    pub identity: SourceIdentity,
    pub hunks: Vec<ExactHunk>,
}

impl ExactFile {
    pub fn old_path(&self) -> Option<&[u8]> {
        self.old_path.as_deref()
    }

    pub fn new_path(&self) -> Option<&[u8]> {
        self.new_path.as_deref()
    }

    pub fn path(&self) -> Option<&[u8]> {
        self.new_path().or(self.old_path())
    }

    pub fn additions(&self) -> u32 {
        self.hunks.iter().map(ExactHunk::additions).sum()
    }

    pub fn deletions(&self) -> u32 {
        self.hunks.iter().map(ExactHunk::deletions).sum()
    }

    fn slice(&self, range: &Range<usize>) -> &[u8] {
        &self.bytes[range.clone()]
    }

    fn header_line(&self, kind: HeaderKind) -> Option<&[u8]> {
        self.header
            .iter()
            .find(|line| line.kind == kind)
            .map(|line| self.slice(&line.bytes))
    }
}

pub fn lines_before(start: u32, len: u32) -> u32 {
    start.saturating_sub(u32::from(len > 0))
}

fn split_lines(bytes: &[u8]) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| start + i + 1);
        lines.push(start..end);
        start = end;
    }
    lines
}

fn content(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n").unwrap_or(line)
}

pub fn parse_exact_patch(bytes: &Arc<[u8]>) -> Result<Vec<ExactFile>, PatchError> {
    let lines = split_lines(bytes);
    let mut files = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let line = content(&bytes[lines[at].clone()]);
        if line.starts_with(b"diff --git ") || line.starts_with(b"--- ") {
            let (file, next) = parse_file(bytes, &lines, at)?;
            files.push(file);
            at = next;
        } else {
            at += 1;
        }
    }
    Ok(files)
}

fn parse_file(
    bytes: &Arc<[u8]>,
    lines: &[Range<usize>],
    start: usize,
) -> Result<(ExactFile, usize), PatchError> {
    let mut file = ExactFile {
        bytes: bytes.clone(),
        header: Vec::new(),
        old_path: None,
        new_path: None,
        new_file: false,
        deleted_file: false,
        renamed: false,
        binary: false,
        identity: SourceIdentity::default(),
        hunks: Vec::new(),
    };
    let mut at = start;
    let mut seen_paths = false;
    while at < lines.len() {
        let range = lines[at].clone();
        let line = content(&bytes[range.clone()]);
        if line.starts_with(b"@@ ") {
            break;
        }
        if at > start && line.starts_with(b"diff --git ") {
            return Ok((file, at));
        }
        if seen_paths && !line.starts_with(b"+++ ") {
            break;
        }
        let kind = if line.starts_with(b"diff --git ") {
            HeaderKind::DiffGit
        } else if let Some(name) = line.strip_prefix(b"--- ") {
            file.old_path = decode_path(name);
            HeaderKind::OldPath
        } else if let Some(name) = line.strip_prefix(b"+++ ") {
            file.new_path = decode_path(name);
            seen_paths = true;
            HeaderKind::NewPath
        } else if line.starts_with(b"new file mode ") {
            file.new_file = true;
            HeaderKind::NewFile
        } else if line.starts_with(b"deleted file mode ") {
            file.deleted_file = true;
            HeaderKind::DeletedFile
        } else if line.starts_with(b"old mode ") || line.starts_with(b"new mode ") {
            HeaderKind::Mode
        } else if line.starts_with(b"rename from ")
            || line.starts_with(b"rename to ")
            || line.starts_with(b"copy from ")
            || line.starts_with(b"copy to ")
            || line.starts_with(b"similarity index ")
            || line.starts_with(b"dissimilarity index ")
        {
            if line.starts_with(b"rename") || line.starts_with(b"copy") {
                file.renamed = true;
            }
            HeaderKind::Rename
        } else if let Some(rest) = line.strip_prefix(b"index ") {
            file.identity = parse_index(rest);
            HeaderKind::Index
        } else if line.starts_with(b"Binary files ") || line.starts_with(b"GIT binary patch") {
            file.binary = true;
            HeaderKind::Other
        } else {
            HeaderKind::Other
        };
        file.header.push(HeaderLine { kind, bytes: range });
        at += 1;
        if file.binary {
            while at < lines.len() && !bytes[lines[at].clone()].starts_with(b"diff --git ") {
                at += 1;
            }
            return Ok((file, at));
        }
    }
    while at < lines.len() {
        let line = content(&bytes[lines[at].clone()]);
        if !line.starts_with(b"@@ ") {
            break;
        }
        let (hunk, next) = parse_hunk(bytes, lines, at)?;
        file.hunks.push(hunk);
        at = next;
    }
    Ok((file, at))
}

fn parse_hunk(
    bytes: &[u8],
    lines: &[Range<usize>],
    start: usize,
) -> Result<(ExactHunk, usize), PatchError> {
    let header = lines[start].clone();
    let line = content(&bytes[header.clone()]);
    let rest = line
        .strip_prefix(b"@@ -")
        .ok_or(PatchError::Malformed("bad hunk header"))?;
    let close = find(rest, b" @@").ok_or(PatchError::Malformed("bad hunk header"))?;
    let ranges = &rest[..close];
    let section_start = header.start + 4 + close + 3;
    let section = section_start..header.start + line.len();
    let (old, new) = split_once(ranges, b" +").ok_or(PatchError::Malformed("bad hunk header"))?;
    let (old_start, old_len) = parse_range(old)?;
    let (new_start, new_len) = parse_range(new)?;
    let mut hunk = ExactHunk {
        old_start,
        old_len,
        new_start,
        new_len,
        section,
        lines: Vec::new(),
    };
    let mut old_line = lines_before(old_start, old_len);
    let mut new_line = lines_before(new_start, new_len);
    let (mut old_left, mut new_left) = (old_len, new_len);
    let mut at = start + 1;
    while at < lines.len() {
        let range = lines[at].clone();
        let raw = &bytes[range.clone()];
        if raw.starts_with(b"\\") {
            let last = hunk
                .lines
                .last_mut()
                .ok_or(PatchError::Malformed("marker before any line"))?;
            last.no_newline = Some(range);
            at += 1;
            continue;
        }
        if old_left == 0 && new_left == 0 {
            break;
        }
        let kind = match raw.first() {
            Some(b' ') | Some(b'\n') => LineKind::Context,
            Some(b'-') => LineKind::Removed,
            Some(b'+') => LineKind::Added,
            _ => return Err(PatchError::Malformed("hunk shorter than its header")),
        };
        let (old, new) = match kind {
            LineKind::Context => {
                if old_left == 0 || new_left == 0 {
                    return Err(PatchError::Malformed("hunk longer than its header"));
                }
                old_left -= 1;
                new_left -= 1;
                old_line += 1;
                new_line += 1;
                (Some(old_line - 1), Some(new_line - 1))
            }
            LineKind::Removed => {
                if old_left == 0 {
                    return Err(PatchError::Malformed("hunk longer than its header"));
                }
                old_left -= 1;
                old_line += 1;
                (Some(old_line - 1), None)
            }
            LineKind::Added => {
                if new_left == 0 {
                    return Err(PatchError::Malformed("hunk longer than its header"));
                }
                new_left -= 1;
                new_line += 1;
                (None, Some(new_line - 1))
            }
        };
        hunk.lines.push(ExactLine {
            kind,
            bytes: range,
            no_newline: None,
            old,
            new,
        });
        at += 1;
    }
    if old_left != 0 || new_left != 0 {
        return Err(PatchError::Malformed("hunk shorter than its header"));
    }
    Ok((hunk, at))
}

fn parse_range(text: &[u8]) -> Result<(u32, u32), PatchError> {
    let parse = |digits: &[u8]| -> Result<u32, PatchError> {
        std::str::from_utf8(digits)
            .ok()
            .and_then(|digits| digits.parse().ok())
            .ok_or(PatchError::Malformed("bad hunk range"))
    };
    match split_once(text, b",") {
        Some((start, len)) => Ok((parse(start)?, parse(len)?)),
        None => Ok((parse(text)?, 1)),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn split_once<'a>(haystack: &'a [u8], needle: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let at = find(haystack, needle)?;
    Some((&haystack[..at], &haystack[at + needle.len()..]))
}

fn parse_index(rest: &[u8]) -> SourceIdentity {
    let objects = rest.split(|&b| b == b' ').next().unwrap_or_default();
    let Some((old, new)) = split_once(objects, b"..") else {
        return SourceIdentity::default();
    };
    let object = |id: &[u8]| ObjectId::parse(std::str::from_utf8(id).ok()?);
    SourceIdentity {
        old: object(old),
        new: object(new),
    }
}

fn decode_path(token: &[u8]) -> Option<Vec<u8>> {
    let token = token.strip_suffix(b"\r").unwrap_or(token);
    let token = token.strip_suffix(b"\t").unwrap_or(token);
    if token == b"/dev/null" {
        return None;
    }
    let decoded = if token.first() == Some(&b'"') {
        unquote(token)?
    } else {
        token.to_vec()
    };
    let stripped = decoded
        .strip_prefix(b"a/")
        .or_else(|| decoded.strip_prefix(b"b/"))
        .map(<[u8]>::to_vec);
    Some(stripped.unwrap_or(decoded))
}

fn unquote(token: &[u8]) -> Option<Vec<u8>> {
    let inner = token.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
    let mut out = Vec::with_capacity(inner.len());
    let mut at = 0;
    while at < inner.len() {
        let byte = inner[at];
        at += 1;
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let escaped = *inner.get(at)?;
        at += 1;
        match escaped {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(0x07),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'v' => out.push(0x0b),
            b'0'..=b'7' => {
                let digits = inner.get(at - 1..at + 2)?;
                let value = digits.iter().try_fold(0u16, |acc, &d| {
                    (b'0'..=b'7')
                        .contains(&d)
                        .then(|| acc * 8 + u16::from(d - b'0'))
                })?;
                out.push(u8::try_from(value).ok()?);
                at += 2;
            }
            other => out.push(other),
        }
    }
    Some(out)
}

pub fn patch_identity(bytes: &[u8]) -> SourceIdentity {
    split_lines(bytes)
        .into_iter()
        .map(|range| content(&bytes[range]))
        .take_while(|line| !line.starts_with(b"@@ "))
        .find_map(|line| line.strip_prefix(b"index ").map(parse_index))
        .unwrap_or_default()
}

pub fn select_file<'a>(files: &'a [ExactFile], path: &str) -> Result<&'a ExactFile, PatchError> {
    let path = path.as_bytes();
    files
        .iter()
        .find(|file| file.new_path() == Some(path))
        .or_else(|| files.iter().find(|file| file.path() == Some(path)))
        .or(match files {
            [only] => Some(only),
            _ => None,
        })
        .ok_or(PatchError::FileNotInPatch)
}

fn resolve(file: &ExactFile, selection: &PatchSelection) -> Result<Vec<Vec<bool>>, PatchError> {
    let mut selected: Vec<Vec<bool>> = file
        .hunks
        .iter()
        .map(|hunk| vec![false; hunk.lines.len()])
        .collect();
    match selection {
        PatchSelection::Lines(ranges) => {
            let wanted: HashSet<(Side, u32)> = ranges
                .iter()
                .flat_map(|range| range.lines.clone().map(move |line| (range.side, line)))
                .collect();
            for &(side, line) in &wanted {
                if !file
                    .hunks
                    .iter()
                    .any(|hunk| hunk.span(side).contains(&line))
                {
                    return Err(PatchError::Stale);
                }
            }
            for (hunk, flags) in file.hunks.iter().zip(&mut selected) {
                for (line, flag) in hunk.lines.iter().zip(flags.iter_mut()) {
                    *flag = [Side::Old, Side::New].into_iter().any(|side| {
                        line.change_line(side)
                            .is_some_and(|n| wanted.contains(&(side, n)))
                    });
                }
            }
        }
        PatchSelection::Hunks(ranges) => {
            for range in ranges {
                let matched = matching_hunks(file, range);
                if matched.is_empty() {
                    return Err(PatchError::Stale);
                }
                for index in matched {
                    let hunk = &file.hunks[index];
                    let covered = hunk
                        .lines
                        .iter()
                        .filter_map(|line| line.change_line(range.side))
                        .all(|n| range.lines.contains(&n));
                    if !covered {
                        return Err(PatchError::AmbiguousHunk);
                    }
                    for (line, flag) in hunk.lines.iter().zip(selected[index].iter_mut()) {
                        *flag = line.kind != LineKind::Context;
                    }
                }
            }
        }
        PatchSelection::HunksAt(ranges) => {
            for range in ranges {
                let mut found = false;
                for (hunk, flags) in file.hunks.iter().zip(&mut selected) {
                    let span = hunk.span(range.side);
                    if !range.lines.clone().any(|line| span.contains(&line)) {
                        continue;
                    }
                    found = true;
                    for (line, flag) in hunk.lines.iter().zip(flags.iter_mut()) {
                        *flag = line.kind != LineKind::Context;
                    }
                }
                if !found {
                    return Err(PatchError::Stale);
                }
            }
        }
    }
    if selected.iter().flatten().all(|flag| !flag) {
        return Err(PatchError::NothingSelected);
    }
    Ok(selected)
}

fn matching_hunks(file: &ExactFile, range: &SideRange) -> Vec<usize> {
    file.hunks
        .iter()
        .enumerate()
        .filter(|(_, hunk)| {
            let span = hunk.span(range.side);
            if range.lines.is_empty() || span.is_empty() {
                span.start == range.lines.start && span.end == range.lines.end
                    || (!span.is_empty() && span.contains(&range.lines.start))
                    || (!range.lines.is_empty() && range.lines.contains(&span.start))
            } else {
                span.start < range.lines.end && range.lines.start < span.end
            }
        })
        .map(|(index, _)| index)
        .collect()
}

pub fn build_selection_patch(
    file: &ExactFile,
    selection: &PatchSelection,
    reverse: bool,
) -> Result<Vec<u8>, PatchError> {
    if file.binary {
        return Err(PatchError::Binary);
    }
    if file.hunks.is_empty() {
        return Err(PatchError::NoLineChanges);
    }
    let selected = resolve(file, selection)?;
    let full = file.hunks.iter().zip(&selected).all(|(hunk, flags)| {
        hunk.lines
            .iter()
            .zip(flags)
            .all(|(line, &flag)| flag || line.kind == LineKind::Context)
    });
    let mut out = Vec::new();
    write_header(file, &mut out, full, reverse)?;
    let mut delta: i64 = 0;
    let mut emitted: Vec<(Side, bool)> = Vec::new();
    for (hunk, flags) in file.hunks.iter().zip(&selected) {
        if !flags.iter().any(|&flag| flag) {
            continue;
        }
        let mut body = Vec::new();
        let (mut old_len, mut new_len) = (0u32, 0u32);
        for (line, &flag) in hunk.lines.iter().zip(flags) {
            let keep_as = match (line.kind, flag, reverse) {
                (LineKind::Context, _, _) => Some(LineKind::Context),
                (kind, true, _) => Some(kind),
                (LineKind::Removed, false, false) | (LineKind::Added, false, true) => {
                    Some(LineKind::Context)
                }
                (LineKind::Added, false, false) | (LineKind::Removed, false, true) => None,
            };
            let Some(kind) = keep_as else {
                continue;
            };
            let raw = file.slice(&line.bytes);
            if kind == line.kind || raw == b"\n" {
                body.extend_from_slice(raw);
            } else {
                body.push(b' ');
                body.extend_from_slice(&raw[1..]);
            }
            if !raw.ends_with(b"\n") {
                body.push(b'\n');
            }
            let marker = line.no_newline.as_ref().map(|range| file.slice(range));
            if let Some(marker) = marker {
                body.extend_from_slice(marker);
                if !marker.ends_with(b"\n") {
                    body.push(b'\n');
                }
            }
            let shown = ExactLine {
                kind,
                bytes: 0..0,
                no_newline: None,
                old: None,
                new: None,
            };
            for side in [Side::Old, Side::New] {
                if shown.on_side(side) {
                    if emitted.contains(&(side, true)) {
                        return Err(PatchError::NewlineConflict);
                    }
                    emitted.push((side, marker.is_some()));
                }
            }
            match kind {
                LineKind::Context => {
                    old_len += 1;
                    new_len += 1;
                }
                LineKind::Removed => old_len += 1,
                LineKind::Added => new_len += 1,
            }
        }
        let (old_start, new_start) = if reverse {
            let new_before = i64::from(lines_before(hunk.new_start, hunk.new_len));
            let old_before = new_before - delta;
            (start_of(old_before, old_len)?, hunk.new_start)
        } else {
            let old_before = i64::from(lines_before(hunk.old_start, hunk.old_len));
            let new_before = old_before + delta;
            (hunk.old_start, start_of(new_before, new_len)?)
        };
        delta += i64::from(new_len) - i64::from(old_len);
        out.extend_from_slice(
            format!("@@ -{old_start},{old_len} +{new_start},{new_len} @@").as_bytes(),
        );
        out.extend_from_slice(file.slice(&hunk.section));
        out.push(b'\n');
        out.extend_from_slice(&body);
    }
    Ok(out)
}

fn start_of(before: i64, len: u32) -> Result<u32, PatchError> {
    let start = before + i64::from(len > 0);
    u32::try_from(start).map_err(|_| PatchError::Malformed("hunk before file start"))
}

fn write_header(
    file: &ExactFile,
    out: &mut Vec<u8>,
    full: bool,
    reverse: bool,
) -> Result<(), PatchError> {
    if file.renamed && !reverse {
        return Err(PatchError::Unsupported(
            "Renamed files can only be staged as a whole file.",
        ));
    }
    if file.deleted_file && reverse && !full {
        return Err(PatchError::Unsupported(
            "Restore the whole deleted file instead of selected lines.",
        ));
    }
    let as_modification =
        file.renamed || (!full && ((file.new_file && reverse) || (file.deleted_file && !reverse)));
    let new_token = file
        .header_line(HeaderKind::NewPath)
        .and_then(|line| line.strip_prefix(b"+++ "))
        .map(content);
    let old_token = file
        .header_line(HeaderKind::OldPath)
        .and_then(|line| line.strip_prefix(b"--- "))
        .map(content);
    let (Some(old_token), Some(new_token)) = (old_token, new_token) else {
        return Err(PatchError::Malformed("missing file names"));
    };
    if as_modification {
        let base = if file.deleted_file {
            swap_prefix(old_token, b'b')
        } else {
            new_token.to_vec()
        };
        let a = swap_prefix(&base, b'a');
        let b = swap_prefix(&base, b'b');
        out.extend_from_slice(b"diff --git ");
        out.extend_from_slice(&strip_tab(&a));
        out.push(b' ');
        out.extend_from_slice(&strip_tab(&b));
        out.push(b'\n');
        out.extend_from_slice(b"--- ");
        out.extend_from_slice(&a);
        out.push(b'\n');
        out.extend_from_slice(b"+++ ");
        out.extend_from_slice(&b);
        out.push(b'\n');
        return Ok(());
    }
    for line in &file.header {
        match line.kind {
            HeaderKind::Index | HeaderKind::Mode | HeaderKind::Rename => continue,
            _ => {
                let raw = file.slice(&line.bytes);
                out.extend_from_slice(raw);
                if !raw.ends_with(b"\n") {
                    out.push(b'\n');
                }
            }
        }
    }
    Ok(())
}

fn strip_tab(token: &[u8]) -> Vec<u8> {
    token.strip_suffix(b"\t").unwrap_or(token).to_vec()
}

fn swap_prefix(token: &[u8], prefix: u8) -> Vec<u8> {
    let mut out = token.to_vec();
    let at = usize::from(out.first() == Some(&b'"'));
    if out.get(at + 1) == Some(&b'/') && matches!(out.get(at), Some(b'a') | Some(b'b')) {
        out[at] = prefix;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &[u8]) -> ExactFile {
        let bytes: Arc<[u8]> = Arc::from(text);
        parse_exact_patch(&bytes).unwrap().remove(0)
    }

    fn lines(side: Side, range: Range<u32>) -> PatchSelection {
        PatchSelection::Lines(vec![SideRange { side, lines: range }])
    }

    const MODIFIED: &[u8] = b"diff --git a/f b/f\nindex 1111111..2222222 100644\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@ fn x\n a\n-b\n+B\n c\n";

    #[test]
    fn parses_identity_and_source_lines() {
        let file = parse(MODIFIED);
        assert_eq!(
            file.identity.old.as_ref().map(ObjectId::as_str),
            Some("1111111")
        );
        assert_eq!(
            file.identity.new.as_ref().map(ObjectId::as_str),
            Some("2222222")
        );
        assert_eq!(file.path(), Some(&b"f"[..]));
        let hunk = &file.hunks[0];
        assert_eq!(hunk.lines[1].old, Some(1));
        assert_eq!(hunk.lines[2].new, Some(1));
        assert_eq!((file.additions(), file.deletions()), (1, 1));
    }

    #[test]
    fn forward_line_selection_keeps_unselected_removals_as_context() {
        let file = parse(MODIFIED);
        let patch = build_selection_patch(&file, &lines(Side::New, 1..2), false).unwrap();
        assert_eq!(
            patch,
            b"diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3 +1,4 @@ fn x\n a\n b\n+B\n c\n"
        );
    }

    #[test]
    fn reverse_line_selection_keeps_unselected_additions_as_context() {
        let file = parse(MODIFIED);
        let patch = build_selection_patch(&file, &lines(Side::Old, 1..2), true).unwrap();
        assert_eq!(
            patch,
            b"diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,4 +1,3 @@ fn x\n a\n-b\n B\n c\n"
        );
    }

    #[test]
    fn crlf_and_invalid_utf8_lines_are_copied_exactly() {
        let file = parse(b"--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n x\r\n-\xff\r\n+\xfe\r\n");
        let patch = build_selection_patch(&file, &lines(Side::New, 1..2), false).unwrap();
        assert_eq!(
            patch,
            b"--- a/f\n+++ b/f\n@@ -1,2 +1,3 @@\n x\r\n \xff\r\n+\xfe\r\n"
        );
    }

    #[test]
    fn newline_marker_conflicts_are_rejected() {
        let file =
            parse(b"--- a/f\n+++ b/f\n@@ -1 +1,2 @@\n-x\n\\ No newline at end of file\n+x\n+y\n");
        assert_eq!(
            build_selection_patch(&file, &lines(Side::New, 1..2), false),
            Err(PatchError::NewlineConflict)
        );
        let whole = build_selection_patch(&file, &lines(Side::New, 0..2), false);
        assert!(whole.is_err());
        let all = PatchSelection::Lines(vec![
            SideRange {
                side: Side::Old,
                lines: 0..1,
            },
            SideRange {
                side: Side::New,
                lines: 0..2,
            },
        ]);
        let patch = build_selection_patch(&file, &all, false).unwrap();
        assert_eq!(
            patch,
            b"--- a/f\n+++ b/f\n@@ -1,1 +1,2 @@\n-x\n\\ No newline at end of file\n+x\n+y\n"
        );
    }

    #[test]
    fn later_hunks_shift_their_target_start() {
        let file = parse(
            b"--- a/f\n+++ b/f\n@@ -1,2 +1,3 @@\n a\n+n\n b\n@@ -10,2 +11,2 @@\n j\n-k\n+K\n",
        );
        let both = PatchSelection::Lines(vec![
            SideRange {
                side: Side::Old,
                lines: 10..11,
            },
            SideRange {
                side: Side::New,
                lines: 11..12,
            },
        ]);
        let patch = build_selection_patch(&file, &both, false).unwrap();
        assert_eq!(patch, b"--- a/f\n+++ b/f\n@@ -10,2 +10,2 @@\n j\n-k\n+K\n");
    }

    #[test]
    fn hunk_targets_resolve_by_source_coverage() {
        let file = parse(MODIFIED);
        let hunk = PatchSelection::Hunks(vec![SideRange {
            side: Side::New,
            lines: 0..3,
        }]);
        let patch = build_selection_patch(&file, &hunk, false).unwrap();
        assert!(patch.ends_with(b" a\n-b\n+B\n c\n"));
        let partial = PatchSelection::Hunks(vec![SideRange {
            side: Side::New,
            lines: 0..1,
        }]);
        assert_eq!(
            build_selection_patch(&file, &partial, false),
            Err(PatchError::AmbiguousHunk)
        );
        let elsewhere = PatchSelection::Hunks(vec![SideRange {
            side: Side::New,
            lines: 20..22,
        }]);
        assert_eq!(
            build_selection_patch(&file, &elsewhere, false),
            Err(PatchError::Stale)
        );
        let narrow = PatchSelection::Hunks(vec![SideRange {
            side: Side::Old,
            lines: 2..3,
        }]);
        assert_eq!(
            build_selection_patch(&file, &narrow, false),
            Err(PatchError::AmbiguousHunk)
        );
    }

    #[test]
    fn selections_outside_the_diff_are_stale() {
        let file = parse(MODIFIED);
        assert_eq!(
            build_selection_patch(&file, &lines(Side::New, 40..41), false),
            Err(PatchError::Stale)
        );
        assert_eq!(
            build_selection_patch(&file, &lines(Side::New, 0..1), false),
            Err(PatchError::NothingSelected)
        );
    }

    #[test]
    fn partial_new_file_reverse_becomes_a_modification() {
        let file = parse(b"diff --git a/n b/n\nnew file mode 100644\nindex 0000000..3333333\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert!(file.new_file);
        assert_eq!(file.identity.old, None);
        let patch = build_selection_patch(&file, &lines(Side::New, 1..2), true).unwrap();
        assert_eq!(
            patch,
            b"diff --git a/n b/n\n--- a/n\n+++ b/n\n@@ -1,1 +1,2 @@\n a\n+b\n"
        );
        let forward = build_selection_patch(&file, &lines(Side::New, 0..1), false).unwrap();
        assert_eq!(
            forward,
            b"diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,1 @@\n+a\n"
        );
    }

    #[test]
    fn mode_and_rename_headers_are_explicit() {
        let mode = parse(b"diff --git a/f b/f\nold mode 100644\nnew mode 100755\nindex 1111111..2222222\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n");
        let patch = build_selection_patch(&mode, &lines(Side::New, 0..1), false).unwrap();
        assert_eq!(
            patch,
            b"diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,1 +1,2 @@\n a\n+b\n"
        );
        let rename = parse(b"diff --git a/old b/\"new \\303\\251\"\nsimilarity index 80%\nrename from old\nrename to \"new \\303\\251\"\n--- a/old\n+++ \"b/new \\303\\251\"\n@@ -1 +1 @@\n-a\n+b\n");
        assert_eq!(rename.new_path(), Some("new é".as_bytes()));
        assert!(build_selection_patch(&rename, &lines(Side::New, 0..1), false).is_err());
        let patch = build_selection_patch(&rename, &lines(Side::New, 0..1), true).unwrap();
        assert_eq!(
            patch,
            b"diff --git \"a/new \\303\\251\" \"b/new \\303\\251\"\n--- \"a/new \\303\\251\"\n+++ \"b/new \\303\\251\"\n@@ -0,0 +1,1 @@\n+b\n"
        );
    }

    #[test]
    fn binary_files_are_rejected() {
        let file = parse(
            b"diff --git a/b b/b\nindex 1111111..2222222 100644\nBinary files a/b and b/b differ\n",
        );
        assert!(file.binary);
        assert_eq!(
            build_selection_patch(&file, &lines(Side::New, 0..1), false),
            Err(PatchError::Binary)
        );
    }
}
