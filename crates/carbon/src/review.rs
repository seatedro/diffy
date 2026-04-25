use crate::model::{DiffSide, FileId, ObjectId};
use crate::projection::ProjectionRow;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AnnotationId(pub u64);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LineRange {
    /// One-based source line number.
    pub start: u32,
    pub len: u32,
}

impl LineRange {
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    pub const fn end(self) -> u32 {
        self.start.saturating_add(self.len)
    }

    pub const fn contains(self, line: u32) -> bool {
        line >= self.start && line < self.end()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ByteRange {
    pub start: u32,
    pub len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Anchor {
    pub file_id: FileId,
    pub side: Option<DiffSide>,
    pub line_range: LineRange,
    pub byte_range: Option<ByteRange>,
    pub old_oid: Option<ObjectId>,
    pub new_oid: Option<ObjectId>,
}

impl Anchor {
    pub fn file(file_id: FileId) -> Self {
        Self {
            file_id,
            side: None,
            line_range: LineRange::default(),
            byte_range: None,
            old_oid: None,
            new_oid: None,
        }
    }

    pub fn touches_row(&self, row: &ProjectionRow) -> bool {
        if self.file_id != row.file_id {
            return false;
        }
        if self.side.is_none() && self.line_range.len == 0 {
            return true;
        }
        match self.side {
            Some(DiffSide::Old) => row
                .old_line
                .is_some_and(|line| self.line_range.contains(line)),
            Some(DiffSide::New) => row
                .new_line
                .is_some_and(|line| self.line_range.contains(line)),
            None => {
                row.old_line
                    .is_some_and(|line| self.line_range.contains(line))
                    || row
                        .new_line
                        .is_some_and(|line| self.line_range.contains(line))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SuggestedChange {
    pub replacement: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictResolution {
    Current,
    Incoming,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AnnotationKind {
    Comment,
    Diagnostic {
        severity: DiagnosticSeverity,
        code: Option<String>,
    },
    SuggestedChange(SuggestedChange),
    ConflictResolution(ConflictResolution),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Annotation {
    pub id: AnnotationId,
    pub anchor: Anchor,
    pub kind: AnnotationKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnnotationSet {
    annotations: Vec<Annotation>,
}

impl AnnotationSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, annotation: Annotation) {
        self.annotations.push(annotation);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Annotation> {
        self.annotations.iter()
    }

    pub fn for_row<'a>(&'a self, row: &'a ProjectionRow) -> impl Iterator<Item = &'a Annotation> {
        self.annotations
            .iter()
            .filter(move |annotation| annotation.anchor.touches_row(row))
    }
}

#[cfg(test)]
mod tests {
    use super::{Anchor, Annotation, AnnotationId, AnnotationKind, AnnotationSet, LineRange};
    use crate::model::{DiffSide, FileId};
    use crate::projection::{ProjectionRow, ProjectionRowKind};

    #[test]
    fn annotations_attach_to_side_specific_projection_rows() {
        let row = ProjectionRow {
            file_id: FileId(1),
            kind: ProjectionRowKind::Added,
            new_line: Some(42),
            ..ProjectionRow::default()
        };
        let mut set = AnnotationSet::new();
        set.push(Annotation {
            id: AnnotationId(7),
            anchor: Anchor {
                file_id: FileId(1),
                side: Some(DiffSide::New),
                line_range: LineRange::new(42, 1),
                byte_range: None,
                old_oid: None,
                new_oid: None,
            },
            kind: AnnotationKind::Comment,
            message: "looks good".to_owned(),
        });

        assert_eq!(set.for_row(&row).count(), 1);
    }
}
