//! Versioned files of a diff that changes while it is shown: stable file
//! identities, monotonic revisions, and the rules that keep a late update
//! from undoing a newer one.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::model::{DiffDocument, IntegrityError};
use crate::remap::SourceRemap;
use crate::source::{FileSources, HydrationError, SourceCoverage};

/// A file's identity, assigned by the app and independent of its path and
/// arrival order. A rename keeps it; a file deleted and created again gets
/// a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u64);

/// A content revision. Revisions of one file, and of the session's order,
/// only increase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Revision(pub u64);

/// One revision of one file's diff.
#[derive(Debug, Clone)]
pub struct FileDiffSnapshot {
    pub id: FileId,
    pub revision: Revision,
    /// A valid one-file document; text stores are shared, not copied.
    pub diff: Arc<DiffDocument>,
}

impl FileDiffSnapshot {
    /// Checks that `diff` holds exactly one file and is intact.
    pub fn new(
        id: FileId,
        revision: Revision,
        diff: Arc<DiffDocument>,
    ) -> Result<Self, UpdateError> {
        if diff.file_count() != 1 {
            return Err(UpdateError::NotOneFile {
                file: id,
                files: diff.file_count(),
            });
        }
        diff.verify_integrity()
            .map_err(|error| UpdateError::Integrity { file: id, error })?;
        Ok(Self { id, revision, diff })
    }

    pub fn coverage(&self) -> SourceCoverage {
        self.diff.coverage(0)
    }
}

/// Why sources are wanted, so the app can prioritize loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourcePurpose {
    ExpandContext,
    CompleteSyntax,
}

/// A request for the exact sources of one file revision. The app reads
/// them from that revision's old and new commits, not from whatever file
/// has the same path now, and answers with [`DiffUpdate::Hydrate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceRequest {
    pub file: FileId,
    pub revision: Revision,
    pub purpose: SourcePurpose,
}

/// A change to a [`DiffSession`].
#[derive(Debug, Clone)]
pub enum DiffUpdate {
    /// Adds a file at the end of the order, or replaces an existing file
    /// with a newer revision. `remap` maps the previous revision's source
    /// lines to this one's, for keeping selection and scroll anchors.
    Upsert {
        file: FileDiffSnapshot,
        remap: Option<SourceRemap>,
    },
    /// Removes a file for good: later upserts of the id are refused.
    Remove { file: FileId, revision: Revision },
    /// Reorders files. Listed files come first in the given order; live
    /// files it omits keep their relative order after them; removed ids
    /// are skipped.
    Order {
        revision: Revision,
        files: Arc<[FileId]>,
    },
    /// Supplies the whole sources of a patch-only file revision. Keeps the
    /// revision: the diff is the same, only its coverage grows.
    Hydrate {
        file: FileId,
        revision: Revision,
        sources: FileSources,
    },
}

/// What an applied [`DiffUpdate`] changed. Indices are positions in
/// [`DiffSession::order`] after the update.
#[derive(Debug, Clone)]
pub enum UpdateOutcome {
    Inserted {
        index: u32,
    },
    Replaced {
        index: u32,
        previous: Revision,
        remap: Option<SourceRemap>,
    },
    /// `index` is where the file was, or `None` when it was never seen.
    Removed {
        index: Option<u32>,
    },
    Reordered,
    Hydrated {
        index: u32,
    },
}

/// Why a [`DiffUpdate`] was refused. A refused update leaves the session
/// as it was; [`UpdateError::Stale`] and [`UpdateError::Removed`] are the
/// normal fate of updates that arrive late and can be dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// The revision is not newer than the one the session holds.
    Stale {
        file: Option<FileId>,
        current: Revision,
        received: Revision,
    },
    /// The file was removed at `revision`; its id cannot come back.
    Removed {
        file: FileId,
        revision: Revision,
    },
    UnknownFile {
        file: FileId,
    },
    DuplicateInOrder {
        file: FileId,
    },
    NotOneFile {
        file: FileId,
        files: u32,
    },
    Integrity {
        file: FileId,
        error: IntegrityError,
    },
    /// The remap is not from the held revision to the new one, or its
    /// line counts do not fit the snapshots.
    RemapMismatch {
        file: FileId,
    },
    Hydration {
        file: FileId,
        error: HydrationError,
    },
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "diff update: {self:?}")
    }
}

impl std::error::Error for UpdateError {}

#[derive(Debug, Clone)]
enum Record {
    Live(FileDiffSnapshot),
    /// Removed at this revision.
    Tombstone(Revision),
}

/// The files of a changing diff, in display order. Lookups by id are
/// O(1); an update touches one file's record, plus O(files) of order
/// bookkeeping when files are inserted, removed, or reordered.
#[derive(Debug, Clone, Default)]
pub struct DiffSession {
    records: HashMap<FileId, Record>,
    order: Vec<FileId>,
    /// Position of each live file in `order`.
    index: HashMap<FileId, u32>,
    order_revision: Revision,
}

impl DiffSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Live files in display order.
    pub fn order(&self) -> &[FileId] {
        &self.order
    }

    pub fn order_revision(&self) -> Revision {
        self.order_revision
    }

    pub fn len(&self) -> u32 {
        self.order.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn file(&self, id: FileId) -> Option<&FileDiffSnapshot> {
        match self.records.get(&id)? {
            Record::Live(snapshot) => Some(snapshot),
            Record::Tombstone(_) => None,
        }
    }

    pub fn index_of(&self, id: FileId) -> Option<u32> {
        self.index.get(&id).copied()
    }

    /// The file at `index` of the order.
    pub fn at(&self, index: u32) -> Option<&FileDiffSnapshot> {
        self.file(*self.order.get(index as usize)?)
    }

    /// Whether `id` was removed.
    pub fn is_removed(&self, id: FileId) -> bool {
        matches!(self.records.get(&id), Some(Record::Tombstone(_)))
    }

    /// The request that would complete file `id`'s sources, or `None` when
    /// it is unknown, binary, or already whole.
    pub fn source_request(&self, id: FileId, purpose: SourcePurpose) -> Option<SourceRequest> {
        let snapshot = self.file(id)?;
        let binary = snapshot.diff.files().meta[0].binary;
        (snapshot.coverage() == SourceCoverage::PatchOnly && !binary).then_some(SourceRequest {
            file: id,
            revision: snapshot.revision,
            purpose,
        })
    }

    pub fn apply(&mut self, update: DiffUpdate) -> Result<UpdateOutcome, UpdateError> {
        let outcome = match update {
            DiffUpdate::Upsert { file, remap } => self.upsert(file, remap),
            DiffUpdate::Remove { file, revision } => self.remove(file, revision),
            DiffUpdate::Order { revision, files } => self.reorder(revision, &files),
            DiffUpdate::Hydrate {
                file,
                revision,
                sources,
            } => self.hydrate(file, revision, sources),
        };
        debug_assert_eq!(self.verify_integrity(), Ok(()));
        outcome
    }

    fn upsert(
        &mut self,
        file: FileDiffSnapshot,
        remap: Option<SourceRemap>,
    ) -> Result<UpdateOutcome, UpdateError> {
        let id = file.id;
        match self.records.get(&id) {
            None => {
                let index = self.order.len() as u32;
                self.order.push(id);
                self.index.insert(id, index);
                self.records.insert(id, Record::Live(file));
                Ok(UpdateOutcome::Inserted { index })
            }
            Some(&Record::Tombstone(revision)) => Err(UpdateError::Removed { file: id, revision }),
            Some(Record::Live(current)) => {
                if file.revision <= current.revision {
                    return Err(UpdateError::Stale {
                        file: Some(id),
                        current: current.revision,
                        received: file.revision,
                    });
                }
                if let Some(remap) = &remap
                    && !remap.fits(current, &file)
                {
                    return Err(UpdateError::RemapMismatch { file: id });
                }
                let previous = current.revision;
                self.records.insert(id, Record::Live(file));
                Ok(UpdateOutcome::Replaced {
                    index: self.index[&id],
                    previous,
                    remap,
                })
            }
        }
    }

    fn remove(&mut self, id: FileId, revision: Revision) -> Result<UpdateOutcome, UpdateError> {
        let current = match self.records.get(&id) {
            Some(Record::Live(snapshot)) => Some(snapshot.revision),
            Some(&Record::Tombstone(revision)) => {
                return Err(UpdateError::Removed { file: id, revision });
            }
            None => None,
        };
        if let Some(current) = current
            && revision <= current
        {
            return Err(UpdateError::Stale {
                file: Some(id),
                current,
                received: revision,
            });
        }
        self.records.insert(id, Record::Tombstone(revision));
        let index = self.index.remove(&id);
        if let Some(at) = index {
            self.order.remove(at as usize);
            self.reindex(at as usize);
        }
        Ok(UpdateOutcome::Removed { index })
    }

    fn reorder(
        &mut self,
        revision: Revision,
        files: &[FileId],
    ) -> Result<UpdateOutcome, UpdateError> {
        if revision <= self.order_revision {
            return Err(UpdateError::Stale {
                file: None,
                current: self.order_revision,
                received: revision,
            });
        }
        let mut listed = HashMap::with_capacity(files.len());
        for &id in files {
            match self.records.get(&id) {
                None => return Err(UpdateError::UnknownFile { file: id }),
                Some(Record::Tombstone(_)) => continue,
                Some(Record::Live(_)) => {}
            }
            if listed.insert(id, ()).is_some() {
                return Err(UpdateError::DuplicateInOrder { file: id });
            }
        }
        let mut order: Vec<FileId> = files
            .iter()
            .copied()
            .filter(|id| listed.contains_key(id))
            .collect();
        order.extend(self.order.iter().filter(|id| !listed.contains_key(id)));
        self.order = order;
        self.order_revision = revision;
        self.reindex(0);
        Ok(UpdateOutcome::Reordered)
    }

    fn hydrate(
        &mut self,
        id: FileId,
        revision: Revision,
        sources: FileSources,
    ) -> Result<UpdateOutcome, UpdateError> {
        let current = match self.records.get(&id) {
            Some(Record::Live(snapshot)) => snapshot,
            Some(&Record::Tombstone(revision)) => {
                return Err(UpdateError::Removed { file: id, revision });
            }
            None => return Err(UpdateError::UnknownFile { file: id }),
        };
        if revision != current.revision {
            return Err(UpdateError::Stale {
                file: Some(id),
                current: current.revision,
                received: revision,
            });
        }
        let diff = current
            .diff
            .hydrate_file(0, sources)
            .map_err(|error| UpdateError::Hydration { file: id, error })?;
        let snapshot = FileDiffSnapshot {
            id,
            revision,
            diff: Arc::new(diff),
        };
        self.records.insert(id, Record::Live(snapshot));
        Ok(UpdateOutcome::Hydrated {
            index: self.index[&id],
        })
    }

    fn reindex(&mut self, from: usize) {
        for (i, id) in self.order.iter().enumerate().skip(from) {
            self.index.insert(*id, i as u32);
        }
    }

    /// Checks that the order lists every live file once, the index agrees
    /// with it, and every snapshot holds one file under its own id.
    /// O(files).
    pub fn verify_integrity(&self) -> Result<(), SessionIntegrityError> {
        let live = self
            .records
            .values()
            .filter(|r| matches!(r, Record::Live(_)))
            .count();
        if live != self.order.len() || self.index.len() != self.order.len() {
            return Err(SessionIntegrityError::OrderCount);
        }
        for (i, id) in self.order.iter().enumerate() {
            let Some(Record::Live(snapshot)) = self.records.get(id) else {
                return Err(SessionIntegrityError::NotLive(*id));
            };
            if self.index.get(id) != Some(&(i as u32)) {
                return Err(SessionIntegrityError::Index(*id));
            }
            if snapshot.id != *id || snapshot.diff.file_count() != 1 {
                return Err(SessionIntegrityError::Snapshot(*id));
            }
        }
        Ok(())
    }
}

/// A broken [`DiffSession`] invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIntegrityError {
    /// The order and the live records disagree in number.
    OrderCount,
    NotLive(FileId),
    Index(FileId),
    Snapshot(FileId),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;

    use super::{
        DiffSession, DiffUpdate, FileDiffSnapshot, FileId, Revision, SourcePurpose, UpdateError,
        UpdateOutcome,
    };
    use crate::remap::SourceRemap;
    use crate::source::{FileSources, SourceCoverage};
    use crate::{Side, TextStore, diff_texts, parse_unified, write_unified};

    fn snapshot(id: u64, revision: u64, new: &str) -> FileDiffSnapshot {
        let doc = diff_texts(Some("f"), Some("f"), Some("base\n"), Some(new), 3);
        FileDiffSnapshot::new(FileId(id), Revision(revision), Arc::new(doc)).unwrap()
    }

    fn upsert(id: u64, revision: u64) -> DiffUpdate {
        DiffUpdate::Upsert {
            file: snapshot(id, revision, &format!("f{id} r{revision}\n")),
            remap: None,
        }
    }

    fn order(revision: u64, ids: &[u64]) -> DiffUpdate {
        DiffUpdate::Order {
            revision: Revision(revision),
            files: ids.iter().map(|&id| FileId(id)).collect(),
        }
    }

    /// Live files as `id@revision`, in order.
    fn dump(session: &DiffSession) -> String {
        let files: Vec<String> = session
            .order()
            .iter()
            .map(|&id| format!("{}@{}", id.0, session.file(id).unwrap().revision.0))
            .collect();
        files.join(" ")
    }

    fn outcome(result: Result<UpdateOutcome, UpdateError>) -> String {
        match result {
            Ok(UpdateOutcome::Inserted { index }) => format!("inserted {index}"),
            Ok(UpdateOutcome::Replaced {
                index, previous, ..
            }) => {
                format!("replaced {index} from {}", previous.0)
            }
            Ok(UpdateOutcome::Removed { index }) => format!("removed {index:?}"),
            Ok(UpdateOutcome::Reordered) => "reordered".into(),
            Ok(UpdateOutcome::Hydrated { index }) => format!("hydrated {index}"),
            Err(UpdateError::Stale { .. }) => "stale".into(),
            Err(UpdateError::Removed { .. }) => "gone".into(),
            Err(UpdateError::UnknownFile { file }) => format!("unknown {}", file.0),
            Err(UpdateError::DuplicateInOrder { file }) => format!("duplicate {}", file.0),
            Err(error) => format!("{error:?}"),
        }
    }

    #[test]
    fn late_updates_cannot_undo_newer_ones() {
        let remove = |id, revision| DiffUpdate::Remove {
            file: FileId(id),
            revision: Revision(revision),
        };
        let steps = [
            (upsert(1, 1), "inserted 0", "1@1"),
            (upsert(2, 1), "inserted 1", "1@1 2@1"),
            (upsert(1, 1), "stale", "1@1 2@1"),
            (upsert(1, 3), "replaced 0 from 1", "1@3 2@1"),
            (upsert(1, 2), "stale", "1@3 2@1"),
            (remove(2, 2), "removed Some(1)", "1@3"),
            (upsert(2, 5), "gone", "1@3"),
            (order(1, &[2, 1]), "reordered", "1@3"),
            (order(1, &[1]), "stale", "1@3"),
            (upsert(3, 1), "inserted 1", "1@3 3@1"),
            (order(2, &[3, 1, 3]), "duplicate 3", "1@3 3@1"),
            (order(2, &[9]), "unknown 9", "1@3 3@1"),
            (order(2, &[3]), "reordered", "3@1 1@3"),
            // A removal that overtakes the file's first upsert still wins.
            (remove(4, 1), "removed None", "3@1 1@3"),
            (upsert(4, 1), "gone", "3@1 1@3"),
        ];
        let mut session = DiffSession::new();
        for (i, (update, expected, files)) in steps.into_iter().enumerate() {
            assert_eq!(outcome(session.apply(update)), expected, "step {i}");
            assert_eq!(dump(&session), files, "step {i}");
        }
    }

    /// Per file: its last revision and whether a removal follows it.
    type Files = Vec<(u64, bool)>;
    /// `(file, revision, is_removal)`.
    type Updates = Vec<(u64, u64, bool)>;

    /// Every revision of files 0..3, each optionally removed after its
    /// last, delivered in any order.
    fn deliveries() -> impl Strategy<Value = (Files, Updates)> {
        prop::collection::vec((1u64..4, any::<bool>()), 3).prop_flat_map(|files| {
            let updates: Vec<(u64, u64, bool)> = files
                .iter()
                .enumerate()
                .flat_map(|(id, &(last, removed))| {
                    let id = id as u64;
                    (1..=last)
                        .map(move |r| (id, r, false))
                        .chain(removed.then_some((id, last + 1, true)))
                })
                .collect();
            (Just(files), Just(updates).prop_shuffle())
        })
    }

    proptest! {
        #[test]
        fn any_delivery_order_converges_to_the_newest_revisions((files, updates) in deliveries()) {
            let mut session = DiffSession::new();
            for (id, revision, remove) in updates {
                let update = if remove {
                    DiffUpdate::Remove { file: FileId(id), revision: Revision(revision) }
                } else {
                    upsert(id, revision)
                };
                let _ = session.apply(update);
            }
            for (id, &(last, removed)) in files.iter().enumerate() {
                let file = session.file(FileId(id as u64));
                let shown = file.map(|f| f.diff.text(0, Side::New).as_str().to_owned());
                let expected = (!removed).then(|| format!("f{id} r{last}\n"));
                prop_assert_eq!(shown, expected);
            }
        }
    }

    #[test]
    fn hydrating_completes_a_revision_without_replacing_it() {
        let full = diff_texts(Some("f"), Some("f"), Some("a\nb\n"), Some("a\nc\n"), 0);
        let patch = parse_unified(&write_unified(&full)).unwrap();
        let first = FileDiffSnapshot::new(FileId(7), Revision(1), Arc::new(patch)).unwrap();
        let mut session = DiffSession::new();
        session
            .apply(DiffUpdate::Upsert {
                file: first,
                remap: None,
            })
            .unwrap();
        let request = session.source_request(FileId(7), SourcePurpose::ExpandContext);
        assert_eq!(request.map(|r| r.revision), Some(Revision(1)));
        let hydrate = |revision| DiffUpdate::Hydrate {
            file: FileId(7),
            revision: Revision(revision),
            sources: FileSources {
                old: Some(TextStore::new("a\nb\n")),
                new: Some(TextStore::new("a\nc\n")),
            },
        };
        assert_eq!(outcome(session.apply(hydrate(0))), "stale");
        assert_eq!(outcome(session.apply(hydrate(1))), "hydrated 0");
        let held = session.file(FileId(7)).unwrap();
        assert_eq!(
            (held.revision, held.coverage()),
            (Revision(1), SourceCoverage::Full)
        );
        assert_eq!(
            session.source_request(FileId(7), SourcePurpose::ExpandContext),
            None
        );
    }

    #[test]
    fn a_remap_must_lead_from_the_held_revision_to_the_new_one() {
        let mut session = DiffSession::new();
        session.apply(upsert(1, 2)).unwrap();
        let (r1, r3, r4) = (
            snapshot(1, 1, "x\n"),
            snapshot(1, 3, "y\n"),
            snapshot(1, 4, "z\n"),
        );
        let wrong = SourceRemap::between(&r1, &r3).unwrap();
        let update = DiffUpdate::Upsert {
            file: r3.clone(),
            remap: Some(wrong),
        };
        assert_eq!(
            session.apply(update).err(),
            Some(UpdateError::RemapMismatch { file: FileId(1) })
        );
        session
            .apply(DiffUpdate::Upsert {
                file: r3.clone(),
                remap: None,
            })
            .unwrap();
        let right = SourceRemap::between(&r3, &r4).unwrap();
        let update = DiffUpdate::Upsert {
            file: r4,
            remap: Some(right),
        };
        assert_eq!(outcome(session.apply(update)), "replaced 0 from 3");
    }
}
