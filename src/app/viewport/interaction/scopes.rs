use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{BlockKind, FileDiffSnapshot, Side, SourceCoverage};
use diffy_syntax::{LanguageId, ScopeTable, ScopeWorker};
use diffy_ui::diff_view::decorator::FileSource;

use crate::contracts::shared::FileKey;

pub type ScopeLabel = Arc<[Arc<str>]>;

struct FileScopes {
    revision: u64,
    keys: [u64; 2],
    anchors: Vec<(Side, u32)>,
    tables: [Option<Arc<ScopeTable>>; 2],
    labels: Vec<Option<ScopeLabel>>,
}

impl FileScopes {
    fn relabel(&mut self) {
        self.labels = self
            .anchors
            .iter()
            .map(|&(side, line)| {
                let table = self.tables[side as usize].as_ref()?;
                let scopes: Vec<Arc<str>> = table.at(line).into_iter().map(Arc::from).collect();
                (!scopes.is_empty()).then(|| scopes.into())
            })
            .collect();
    }
}

#[derive(Default)]
pub struct Scopes {
    worker: Option<ScopeWorker>,
    files: HashMap<FileKey, FileScopes>,
    owners: HashMap<u64, FileKey>,
    generation: u64,
}

pub type SharedScopes = Rc<RefCell<Scopes>>;

fn key(file: FileKey, revision: u64, side: Side) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    (file.0, revision, side as u8).hash(&mut hasher);
    hasher.finish()
}

impl Scopes {
    pub fn set_worker(&mut self, worker: ScopeWorker) {
        self.worker = Some(worker);
    }

    pub fn has_worker(&self) -> bool {
        self.worker.is_some()
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.owners.clear();
    }

    pub fn request(&mut self, snapshot: &FileDiffSnapshot) {
        let Some(worker) = &self.worker else {
            return;
        };
        let doc = &snapshot.diff;
        let revision = snapshot.revision.0;
        if self
            .files
            .get(&snapshot.id)
            .is_some_and(|f| f.revision == revision)
            || snapshot.coverage() != SourceCoverage::Full
        {
            return;
        }
        let Some(language) = LanguageId::from_path(doc.path(0)) else {
            return;
        };
        let (h, b) = (doc.hunks(), doc.blocks());
        let anchors: Vec<(Side, u32)> = doc.files().hunks[0]
            .clone()
            .map(|hunk| {
                let first = h.blocks[hunk as usize]
                    .clone()
                    .map(|block| block as usize)
                    .find(|&bi| b.kind[bi] == BlockKind::Change);
                match first {
                    Some(bi) if b.new_len[bi] > 0 => (Side::New, b.new_line[bi].saturating_sub(1)),
                    Some(bi) => (Side::Old, b.old_line[bi].saturating_sub(1)),
                    None => (Side::New, h.new_start[hunk as usize].saturating_sub(1)),
                }
            })
            .collect();
        if let Some(old) = self.files.remove(&snapshot.id) {
            for key in old.keys {
                self.owners.remove(&key);
            }
        }
        let keys = [Side::Old, Side::New].map(|side| key(snapshot.id, revision, side));
        for side in [Side::Old, Side::New] {
            if !anchors.iter().any(|&(s, _)| s == side) {
                continue;
            }
            self.generation += 1;
            let k = keys[side as usize];
            self.owners.insert(k, snapshot.id);
            worker.request(
                k,
                self.generation,
                language.clone(),
                doc.text(0, side).shared().clone(),
            );
        }
        self.files.insert(
            snapshot.id,
            FileScopes {
                revision,
                keys,
                labels: vec![None; anchors.len()],
                anchors,
                tables: [None, None],
            },
        );
    }

    pub fn poll(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let mut changed = false;
        while let Some(result) = worker.try_recv() {
            let Some(file) = self
                .owners
                .get(&result.key)
                .and_then(|key| self.files.get_mut(key))
            else {
                continue;
            };
            let Some(side) = file.keys.iter().position(|k| *k == result.key) else {
                continue;
            };
            file.tables[side] = Some(result.table);
            file.relabel();
            changed = true;
        }
        changed
    }

    pub fn label(&self, source: FileSource, hunk: u32) -> Option<ScopeLabel> {
        let file = self.files.get(&source.id)?;
        if file.revision != source.revision.0 {
            return None;
        }
        file.labels.get(hunk as usize)?.clone()
    }
}
