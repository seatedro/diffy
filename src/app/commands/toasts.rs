use crate::contracts::commands::{ToastKind, ToastUpdate};
use crate::contracts::shared::JobId;
use crate::contracts::shell::Action;

const INFO_LIFETIME_MS: u64 = 4_000;
const ERROR_LIFETIME_MS: u64 = 8_000;
const FINISHED_PROGRESS_LIFETIME_MS: u64 = 1_500;
const MAX_TOASTS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct ToastItem {
    pub id: u64,
    pub job: Option<JobId>,
    pub kind: ToastKind,
    pub message: String,
    pub description: Option<String>,
    pub progress: Option<(u64, u64)>,
    pub shown_at_ms: Option<u64>,
    pub undo: Option<Action>,
    pub expired: Option<Action>,
}

impl ToastItem {
    pub fn lifetime_ms(&self) -> Option<u64> {
        if self.undo.is_some() {
            return None;
        }
        match self.progress {
            Some((done, total)) if total > 0 && done >= total => {
                Some(FINISHED_PROGRESS_LIFETIME_MS)
            }
            Some(_) => None,
            None => Some(match self.kind {
                ToastKind::Error => ERROR_LIFETIME_MS,
                ToastKind::Info | ToastKind::Success => INFO_LIFETIME_MS,
            }),
        }
    }

    pub fn fraction(&self) -> Option<f32> {
        self.progress.map(|(done, total)| {
            if total == 0 {
                0.0
            } else {
                (done as f32 / total as f32).clamp(0.0, 1.0)
            }
        })
    }
}

#[derive(Debug, Default)]
pub struct ToastHost {
    items: Vec<ToastItem>,
    next_id: u64,
}

impl ToastHost {
    pub fn items(&self) -> &[ToastItem] {
        &self.items
    }

    pub fn apply(&mut self, update: ToastUpdate) {
        match update {
            ToastUpdate::Show {
                id,
                kind,
                message,
                description,
            } => {
                if let Some(item) =
                    id.and_then(|job| self.items.iter_mut().find(|t| t.job == Some(job)))
                {
                    item.kind = kind;
                    item.message = message;
                    item.description = description;
                    item.progress = None;
                    item.shown_at_ms = None;
                    return;
                }
                // The same message again restarts the one showing rather
                // than stacking a copy per repeated key press.
                if id.is_none()
                    && let Some(item) = self.items.iter_mut().find(|t| {
                        t.job.is_none()
                            && t.undo.is_none()
                            && t.kind == kind
                            && t.message == message
                            && t.description == description
                    })
                {
                    item.shown_at_ms = None;
                    return;
                }
                self.push(ToastItem {
                    id: 0,
                    job: id,
                    kind,
                    message,
                    description,
                    progress: None,
                    shown_at_ms: None,
                    undo: None,
                    expired: None,
                });
            }
            ToastUpdate::Progress {
                id,
                message,
                completed,
                total,
            } => {
                if let Some(item) = self.items.iter_mut().find(|t| t.job == Some(id)) {
                    let finished_now =
                        total > 0 && completed >= total && item.progress.is_none_or(|(d, t)| d < t);
                    item.message = message;
                    item.progress = Some((completed, total));
                    if finished_now {
                        item.shown_at_ms = None;
                    }
                    return;
                }
                self.push(ToastItem {
                    id: 0,
                    job: Some(id),
                    kind: ToastKind::Info,
                    message,
                    description: None,
                    progress: Some((completed, total)),
                    shown_at_ms: None,
                    undo: None,
                    expired: None,
                });
            }
            ToastUpdate::Dismiss(job) => self.items.retain(|t| t.job != Some(job)),
            ToastUpdate::Undo {
                id,
                kind,
                message,
                undo,
                expired,
                ..
            } => {
                self.items.retain(|t| t.job != Some(id));
                self.push(ToastItem {
                    id: 0,
                    job: Some(id),
                    kind,
                    message,
                    description: None,
                    progress: None,
                    shown_at_ms: None,
                    undo: Some(*undo),
                    expired: expired.map(|a| *a),
                });
            }
        }
    }

    pub fn take_by_job(&mut self, job: JobId) -> Option<ToastItem> {
        let index = self.items.iter().position(|t| t.job == Some(job))?;
        Some(self.items.remove(index))
    }

    pub fn take(&mut self, id: u64) -> Option<ToastItem> {
        let index = self.items.iter().position(|t| t.id == id)?;
        Some(self.items.remove(index))
    }

    fn push(&mut self, mut item: ToastItem) {
        self.next_id += 1;
        item.id = self.next_id;
        self.items.push(item);
        if self.items.len() > MAX_TOASTS {
            let excess = self.items.len() - MAX_TOASTS;
            self.items.drain(..excess);
        }
    }

    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.id != id);
        self.items.len() != before
    }

    pub fn tick(&mut self, now_ms: u64) -> Option<u64> {
        for item in &mut self.items {
            if item.shown_at_ms.is_none() {
                item.shown_at_ms = Some(now_ms);
            }
        }
        self.items
            .retain(|t| match (t.lifetime_ms(), t.shown_at_ms) {
                (Some(life), Some(shown)) => now_ms < shown.saturating_add(life),
                _ => true,
            });
        self.items
            .iter()
            .filter_map(|t| {
                Some(
                    t.shown_at_ms?
                        .saturating_add(t.lifetime_ms()?)
                        .saturating_sub(now_ms),
                )
            })
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_updates_in_place_then_expires_after_completion() {
        let mut host = ToastHost::default();
        let job = JobId(7);
        host.apply(ToastUpdate::Progress {
            id: job,
            message: "Fetching".to_owned(),
            completed: 1,
            total: 4,
        });
        assert_eq!(host.tick(0), None);
        host.apply(ToastUpdate::Progress {
            id: job,
            message: "Fetching".to_owned(),
            completed: 4,
            total: 4,
        });
        assert_eq!(host.items().len(), 1);
        assert_eq!(host.items()[0].fraction(), Some(1.0));
        assert_eq!(host.tick(100), Some(FINISHED_PROGRESS_LIFETIME_MS));
        assert_eq!(host.tick(100 + FINISHED_PROGRESS_LIFETIME_MS), None);
        assert!(host.items().is_empty());
    }

    #[test]
    fn errors_outlive_info_and_dismiss_by_id() {
        let mut host = ToastHost::default();
        host.apply(ToastUpdate::info("Saved"));
        host.apply(ToastUpdate::error("Failed"));
        host.tick(0);
        host.tick(INFO_LIFETIME_MS);
        assert_eq!(host.items().len(), 1);
        assert_eq!(host.items()[0].kind, ToastKind::Error);
        let id = host.items()[0].id;
        assert!(host.dismiss(id));
        assert!(!host.dismiss(id));
    }
}
