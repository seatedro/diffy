use std::collections::HashMap;
use std::sync::Arc;

use diffy_syntax::{HighlightKind, HighlightSpan, HighlightWorker, Highlighted, LanguageId};
use quark::selection::BlockKey;
use quark_ui::document::{CodeHighlighter, SyntaxTone, ToneSpan};

#[derive(Debug, Default)]
struct Slot {
    lang: String,
    source: Arc<str>,
    generation: u64,
    awaiting: bool,
    held: Option<(u64, u32)>,
    version: u64,
    spans: Vec<ToneSpan>,
}

pub struct ReviewCodeHighlighter {
    worker: HighlightWorker,
    slots: HashMap<BlockKey, Slot>,
    next_generation: u64,
    next_version: u64,
}

impl ReviewCodeHighlighter {
    pub fn new(worker: HighlightWorker) -> Self {
        Self {
            worker,
            slots: HashMap::new(),
            next_generation: 0,
            next_version: 0,
        }
    }

    fn apply(&mut self, result: Highlighted) -> Option<BlockKey> {
        let key = BlockKey(result.slot);
        let slot = self.slots.get_mut(&key)?;
        if result.generation != slot.generation {
            return None;
        }
        if !result.pending {
            slot.awaiting = false;
        }
        let stamp = (result.generation, result.revision);
        if slot.held.is_some_and(|held| held >= stamp) || result.part.is_some() {
            return None;
        }
        slot.held = Some(stamp);
        let spans = tone_spans(&result.spans);
        if spans.is_empty() && slot.version == 0 {
            return None;
        }
        self.next_version += 1;
        slot.version = self.next_version;
        slot.spans = spans;
        Some(key)
    }

    fn drain(
        &mut self,
        mut next: impl FnMut(&HighlightWorker) -> Option<Highlighted>,
    ) -> Vec<BlockKey> {
        let mut keys = Vec::new();
        while let Some(result) = next(&self.worker) {
            if let Some(key) = self.apply(result)
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        keys
    }

    fn awaiting(&self) -> bool {
        self.slots.values().any(|slot| slot.awaiting)
    }
}

impl CodeHighlighter for ReviewCodeHighlighter {
    fn version(&mut self, slot: BlockKey, lang: &str, code: &str) -> u64 {
        let entry = self.slots.entry(slot).or_default();
        if entry.generation != 0 && entry.lang == lang && *entry.source == *code {
            return entry.version;
        }
        let grew = entry.lang == lang && code.starts_with(&*entry.source);
        if !grew {
            entry.version = 0;
            entry.spans.clear();
        }
        entry.lang.clear();
        entry.lang.push_str(lang);
        entry.source = Arc::from(code);
        self.next_generation += 1;
        entry.generation = self.next_generation;
        entry.held = None;
        match LanguageId::from_fence(lang) {
            Some(language) => {
                entry.awaiting = true;
                self.worker
                    .request(slot.0, entry.generation, language, entry.source.clone());
            }
            None => {
                entry.awaiting = false;
                entry.version = 0;
                entry.spans.clear();
            }
        }
        entry.version
    }

    fn spans(&self, slot: BlockKey) -> &[ToneSpan] {
        self.slots.get(&slot).map_or(&[], |slot| &slot.spans)
    }

    fn poll(&mut self) -> Vec<BlockKey> {
        self.drain(|worker| worker.try_recv().ok().flatten())
    }

    fn finish_pending(&mut self) -> Vec<BlockKey> {
        let mut keys = self.poll();
        while self.awaiting() {
            let Ok(result) = self.worker.recv() else {
                for slot in self.slots.values_mut() {
                    slot.awaiting = false;
                }
                break;
            };
            let mut once = Some(result);
            for key in self.drain(|_| once.take()) {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        keys
    }

    fn forget(&mut self, slot: BlockKey) {
        self.slots.remove(&slot);
    }

    fn len(&self) -> usize {
        self.slots.len()
    }
}

pub fn syntax_tone(kind: HighlightKind) -> Option<SyntaxTone> {
    Some(match kind {
        HighlightKind::Keyword | HighlightKind::Preprocessor | HighlightKind::Tag => {
            SyntaxTone::Keyword
        }
        HighlightKind::String => SyntaxTone::String,
        HighlightKind::Comment => SyntaxTone::Comment,
        HighlightKind::Function => SyntaxTone::Function,
        HighlightKind::Type | HighlightKind::Builtin | HighlightKind::Namespace => SyntaxTone::Type,
        HighlightKind::Number | HighlightKind::Constant => SyntaxTone::Number,
        HighlightKind::Property | HighlightKind::Attribute | HighlightKind::Label => {
            SyntaxTone::Property
        }
        HighlightKind::Operator => SyntaxTone::Operator,
        HighlightKind::Normal | HighlightKind::Punctuation | HighlightKind::Variable => {
            return None;
        }
    })
}

fn tone_spans(spans: &[HighlightSpan]) -> Vec<ToneSpan> {
    spans
        .iter()
        .filter(|span| span.length > 0)
        .filter_map(|span| {
            Some(ToneSpan {
                range: span.offset..span.offset.saturating_add(span.length),
                tone: syntax_tone(span.kind)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlighter() -> ReviewCodeHighlighter {
        ReviewCodeHighlighter::new(HighlightWorker::default())
    }

    #[test]
    fn unchanged_source_keeps_its_version_and_request() {
        let mut h = highlighter();
        let key = BlockKey(1);
        h.version(key, "rust", "fn main() {}");
        let generation = h.slots[&key].generation;

        h.version(key, "rust", "fn main() {}");

        assert_eq!(h.slots[&key].generation, generation);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn plain_results_settle_without_a_new_version() {
        let mut h = highlighter();
        let key = BlockKey(2);
        assert_eq!(h.version(key, "rust", "let x = 1;"), 0);

        let keys = h.finish_pending();

        assert!(keys.is_empty());
        assert!(!h.awaiting());
        assert_eq!(h.version(key, "rust", "let x = 1;"), 0);
        assert!(h.spans(key).is_empty());
    }

    #[test]
    fn unknown_fences_stay_plain_without_requests() {
        let mut h = highlighter();
        let key = BlockKey(3);

        assert_eq!(h.version(key, "", "plain text"), 0);
        assert_eq!(h.version(key, "not a tag!", "plain text"), 0);

        assert!(!h.awaiting());
        assert!(h.finish_pending().is_empty());
    }

    #[test]
    fn stale_generations_and_older_revisions_are_dropped() {
        let mut h = highlighter();
        let key = BlockKey(4);
        h.version(key, "rust", "a");
        h.version(key, "rust", "b");
        let current = h.slots[&key].generation;
        let result = |generation, revision| Highlighted {
            slot: 4,
            generation,
            revision,
            source: Arc::from("b"),
            spans: vec![HighlightSpan {
                offset: 0,
                length: 1,
                kind: HighlightKind::Keyword,
            }],
            pending: false,
            unresolved: Vec::new(),
            part: None,
        };

        assert_eq!(h.apply(result(current - 1, 0)), None);
        assert_eq!(h.apply(result(current, 1)), Some(key));
        let version = h.version(key, "rust", "b");
        assert_ne!(version, 0);
        assert_eq!(h.apply(result(current, 0)), None);
        assert_eq!(
            h.spans(key),
            [ToneSpan {
                range: 0..1,
                tone: SyntaxTone::Keyword
            }]
        );
    }

    #[test]
    fn growing_source_keeps_the_held_highlight() {
        let mut h = highlighter();
        let key = BlockKey(5);
        h.version(key, "rust", "fn");
        let generation = h.slots[&key].generation;
        h.apply(Highlighted {
            slot: 5,
            generation,
            revision: 0,
            source: Arc::from("fn"),
            spans: vec![HighlightSpan {
                offset: 0,
                length: 2,
                kind: HighlightKind::Keyword,
            }],
            pending: false,
            unresolved: Vec::new(),
            part: None,
        });
        let held = h.version(key, "rust", "fn");

        assert_eq!(h.version(key, "rust", "fn main"), held);
        assert_eq!(h.version(key, "python", "fn main"), 0);
    }

    #[test]
    fn categories_map_to_markdown_tones() {
        assert_eq!(
            syntax_tone(HighlightKind::Keyword),
            Some(SyntaxTone::Keyword)
        );
        assert_eq!(
            syntax_tone(HighlightKind::Constant),
            Some(SyntaxTone::Number)
        );
        assert_eq!(syntax_tone(HighlightKind::Builtin), Some(SyntaxTone::Type));
        assert_eq!(
            syntax_tone(HighlightKind::Attribute),
            Some(SyntaxTone::Property)
        );
        assert_eq!(syntax_tone(HighlightKind::Punctuation), None);
        assert_eq!(syntax_tone(HighlightKind::Normal), None);
    }
}
