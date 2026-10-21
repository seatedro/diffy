pub mod files;
pub mod hover;
pub mod interaction;
pub mod outline;
pub mod pipeline;
pub mod symbols;

use diffy_diff::Side;
use diffy_ui::SourcePoint;

use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Controller, Intent, Reduction};
use crate::contracts::viewport::files::{FileAt, head_revision, revision_label};
use crate::contracts::viewport::interaction::InstallItem;
use crate::contracts::viewport::pipeline::PipelineAction;
use crate::contracts::viewport::symbols::{
    DefinitionTarget, ResolveRequest, SourcePosition, SymbolsAction, SymbolsEffect, SymbolsEvent,
};
use crate::contracts::viewport::{ViewportAction, ViewportEvent};
use crate::core::vcs::model::RevisionId;
use files::Reveal;
use symbols::Place;

pub use files::FilesController;
pub use interaction::InteractionController;
pub use pipeline::PipelineController;
use pipeline::PipelineInstall;
pub use symbols::SymbolsController;

#[derive(Debug, Default)]
pub struct ViewportController {
    pub pipeline: PipelineController,
    pub interaction: InteractionController,
    pub files: FilesController,
    pub symbols: SymbolsController,
    pub hover: hover::HoverController,
    document: Option<FileAt>,
}

impl ViewportController {
    fn prepare(&mut self) {
        if let Some(diff) = self.interaction.diff() {
            self.pipeline.set_preparation(diff.preparation());
        }
    }

    fn install(&mut self, mut reduction: Reduction, cx: &SharedContext) -> Reduction {
        let batch = self.pipeline.take_batch();
        if !batch.is_empty() {
            let items = batch.into_iter().map(|install| match install {
                PipelineInstall::View(install) => InstallItem::View(install),
                PipelineInstall::Slot(update) => InstallItem::Slot(update),
            });
            reduction.merge(self.interaction.install_batch(items, cx));
        }
        reduction
    }

    fn request_sources(&mut self, mut reduction: Reduction, cx: &SharedContext) -> Reduction {
        let requests = self.interaction.take_source_requests();
        if requests.is_empty() {
            return reduction;
        }
        self.prepare();
        for (file, purpose) in requests {
            reduction.merge(
                self.pipeline
                    .reduce_action(PipelineAction::RequestSources { file, purpose }, cx),
            );
        }
        self.install(reduction, cx)
    }
}

impl ViewportController {
    fn document(&self, cx: &SharedContext) -> Option<FileAt> {
        if let Some((file, _, _)) = self.files.active_place() {
            return Some(file);
        }
        let manifest = cx.manifest.as_deref()?;
        let file = manifest.file(cx.selected_file.as_ref()?.key)?;
        let (_, head) = head_revision(cx)?;
        Some(FileAt {
            path: file.paths.new_path()?.into_owned(),
            label: revision_label(&head.id),
            revision: head,
        })
    }

    fn publish_document(&mut self, mut reduction: Reduction, cx: &SharedContext) -> Reduction {
        let document = self.document(cx);
        if document != self.document {
            self.document = document.clone();
            reduction.intents.push(Intent::PublishDocument(document));
        }
        reduction
    }

    fn place(&self) -> Option<Place> {
        if let Some((file, scroll, caret)) = self.files.active_place() {
            return Some(Place::File {
                file,
                scroll,
                caret,
            });
        }
        self.interaction.scroll_offset().map(|scroll| Place::Diff {
            scroll,
            caret: self.interaction.caret(),
        })
    }

    fn follow_definition(&mut self, mut reduction: Reduction, cx: &SharedContext) -> Reduction {
        let position = match self.files.take_definition() {
            Some((file, point, text)) => Some(SourcePosition {
                path: file.path,
                revision: file.revision,
                line: point.line,
                byte: point.byte,
                text,
            }),
            None => self
                .interaction
                .take_definition()
                .and_then(|(point, text)| diff_position(point, text, cx)),
        };
        if let Some(position) = position {
            reduction.merge(self.symbols.resolve(position, cx));
        }
        reduction
    }

    fn jump(&mut self, target: DefinitionTarget, cx: &SharedContext) -> Reduction {
        let from = self.place();
        let reduction = self.show_target(target, cx);
        if let Some(from) = from {
            self.symbols.record(from);
        }
        reduction
    }

    fn show_target(&mut self, target: DefinitionTarget, cx: &SharedContext) -> Reduction {
        if self.files.active().is_none()
            && let Some(point) = diff_point(&target, cx)
            && let Some(reduction) = self.interaction.reveal_definition(point, target.len)
        {
            return reduction;
        }
        let file = FileAt {
            label: revision_label(&target.revision.id),
            path: target.path,
            revision: target.revision,
        };
        let reveal = Reveal::Source {
            line: target.line,
            column: target.column,
            len: target.len,
        };
        self.files.open_at(file, reveal, cx)
    }

    fn restore(&mut self, place: Place, cx: &SharedContext) -> Reduction {
        match place {
            Place::Diff { scroll, caret } => {
                let mut reduction = self.files.show_diff();
                let revealed = caret.and_then(|point| self.interaction.reveal_definition(point, 0));
                reduction
                    .merge(revealed.unwrap_or_else(|| self.interaction.restore_scroll(scroll)));
                reduction.changed = true;
                reduction
            }
            Place::File {
                file,
                scroll,
                caret,
            } => {
                let reveal = match caret {
                    Some(point) => Reveal::Source {
                        line: point.line,
                        column: point.byte,
                        len: 0,
                    },
                    None => Reveal::Scroll(scroll),
                };
                self.files.open_at(file, reveal, cx)
            }
        }
    }

    pub fn hover(
        &mut self,
        pointer: Option<(f32, f32)>,
        primary: bool,
        cx: &SharedContext,
    ) -> Reduction {
        self.hover.pointer = pointer;
        self.hover.primary = primary;
        let hit = pointer.and_then(|(x, y)| match self.files.active() {
            Some(_) => self.files.hover_at(x, y).map(|(file, hit)| {
                let position = SourcePosition {
                    path: file.path,
                    revision: file.revision,
                    line: hit.point.line,
                    byte: hit.point.byte,
                    text: hit.line.clone(),
                };
                (position, hit)
            }),
            None => {
                let hit = self.interaction.hover_at(x, y)?;
                Some((diff_position(hit.point, hit.line.clone(), cx)?, hit))
            }
        });
        let link = hit
            .as_ref()
            .filter(|_| primary)
            .map(|(_, hit)| (hit.point, hit.len));
        let mut changed = if self.files.active().is_some() {
            self.files.set_link(link) | self.interaction.set_link(None)
        } else {
            self.files.set_link(None) | self.interaction.set_link(link)
        };
        let target = hit.map(|(position, hit)| (hover_key(&position), position, hit));
        let anchors: Vec<_> = target
            .iter()
            .map(|(key, _, hit)| (*key, hit.rect))
            .collect();
        let now = self.hover.now_ms();
        changed |= self.hover.card.pointer_moved(pointer, &anchors, now);
        let mut reduction = Reduction {
            changed,
            ..Reduction::default()
        };
        let shown = self.hover.card.shown();
        if let Some((key, position, _)) = target
            && self.hover.key != Some(key)
            && shown.is_none_or(|shown| shown == key)
            && let Some((repo_path, head)) = head_revision(cx)
        {
            self.hover.key = Some(key);
            self.hover.generation += 1;
            reduction = reduction.effect(SymbolsEffect::Describe(Box::new(ResolveRequest {
                generation: self.hover.generation,
                repo_path,
                position,
                head,
                index: self.symbols.index().cloned(),
            })));
        }
        reduction
    }

    pub fn link_active(&self) -> bool {
        match self.files.active() {
            Some(_) => self.files.has_link(),
            None => self.interaction.has_link(),
        }
    }

    pub fn dismiss_hover(&mut self) -> bool {
        let link = self.files.set_link(None) | self.interaction.set_link(None);
        self.hover.dismiss() | link
    }

    fn symbols_action(&mut self, action: SymbolsAction, cx: &SharedContext) -> Reduction {
        match action {
            SymbolsAction::GoToDefinition => {
                let reduction = if self.files.active().is_some() {
                    self.files.request_definition()
                } else {
                    self.interaction.request_definition()
                };
                self.follow_definition(reduction, cx)
            }
            SymbolsAction::Jump(target) => self.jump(target, cx),
            SymbolsAction::Back => {
                let here = self.place();
                match self.symbols.back(here) {
                    Some(place) => self.restore(place, cx),
                    None => Reduction::default(),
                }
            }
            SymbolsAction::Forward => {
                let here = self.place();
                match self.symbols.forward(here) {
                    Some(place) => self.restore(place, cx),
                    None => Reduction::default(),
                }
            }
        }
    }
}

fn hover_key(position: &SourcePosition) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (
        &position.path,
        &position.revision.id,
        position.line,
        position.byte,
    )
        .hash(&mut hasher);
    hasher.finish()
}

fn diff_position(point: SourcePoint, text: String, cx: &SharedContext) -> Option<SourcePosition> {
    let manifest = cx.manifest.as_deref()?;
    let file = manifest.file(point.file)?;
    let (_, head) = head_revision(cx)?;
    let (path, revision) = match point.side {
        Side::New => (file.paths.new_path()?, head),
        Side::Old => (
            file.paths.old_path()?,
            RevisionId {
                backend: head.backend,
                id: manifest.resolved_left.clone(),
            },
        ),
    };
    Some(SourcePosition {
        path: path.into_owned(),
        revision,
        line: point.line,
        byte: point.byte,
        text,
    })
}

fn diff_point(target: &DefinitionTarget, cx: &SharedContext) -> Option<SourcePoint> {
    let manifest = cx.manifest.as_deref()?;
    let (_, head) = head_revision(cx)?;
    let side = if target.revision == head {
        Side::New
    } else if target.revision.id == manifest.resolved_left {
        Side::Old
    } else {
        return None;
    };
    let file = manifest.files.iter().find(|file| {
        let path = match side {
            Side::New => file.paths.new_path(),
            Side::Old => file.paths.old_path(),
        };
        path.as_deref() == Some(target.path.as_str())
    })?;
    Some(SourcePoint {
        file: file.key,
        side,
        line: target.line,
        byte: target.column,
    })
}

impl ViewportController {
    fn route_action(&mut self, action: ViewportAction, cx: &SharedContext) -> Reduction {
        match action {
            ViewportAction::Pipeline(action) => {
                self.prepare();
                let reduction = self.pipeline.reduce_action(action, cx);
                self.install(reduction, cx)
            }
            ViewportAction::Interaction(action) => {
                if self.files.active().is_some()
                    && let Some(reduction) = self.files.redirect(&action)
                {
                    return reduction;
                }
                let reduction = self.interaction.reduce_action(action, cx);
                let reduction = self.follow_definition(reduction, cx);
                self.request_sources(reduction, cx)
            }
            ViewportAction::Files(action) => {
                let reduction = self.files.reduce_action(action, cx);
                self.follow_definition(reduction, cx)
            }
            ViewportAction::Symbols(action) => self.symbols_action(action, cx),
        }
    }

    fn route_event(&mut self, event: ViewportEvent, cx: &SharedContext) -> Reduction {
        match event {
            ViewportEvent::Pipeline(event) => {
                self.prepare();
                let reduction = self.pipeline.reduce_event(event, cx);
                self.install(reduction, cx)
            }
            ViewportEvent::Interaction(event) => self.interaction.reduce_event(event, cx),
            ViewportEvent::Files(event) => self.files.reduce_event(event),
            ViewportEvent::Symbols(SymbolsEvent::Described {
                generation,
                description,
            }) => Reduction {
                changed: self.hover.described(generation, description),
                ..Reduction::default()
            },
            ViewportEvent::Symbols(event) => {
                let reduction = self.symbols.reduce_event(event);
                match self.symbols.take_jump() {
                    Some(target) => {
                        let mut reduction = reduction;
                        reduction.merge(self.jump(target, cx));
                        reduction
                    }
                    None => reduction,
                }
            }
        }
    }

    fn route_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        self.prepare();
        let reduction = self.pipeline.apply_intent(intent, cx);
        let mut reduction = self.install(reduction, cx);
        reduction.merge(self.interaction.apply_intent(intent, cx));
        reduction.merge(self.files.apply_intent(intent));
        reduction.merge(self.symbols.apply_intent(intent, cx));
        reduction
    }
}

impl Controller for ViewportController {
    type Action = ViewportAction;
    type Event = ViewportEvent;

    fn reduce_action(&mut self, action: ViewportAction, cx: &SharedContext) -> Reduction {
        let reduction = self.route_action(action, cx);
        self.publish_document(reduction, cx)
    }

    fn reduce_event(&mut self, event: ViewportEvent, cx: &SharedContext) -> Reduction {
        let reduction = self.route_event(event, cx);
        self.publish_document(reduction, cx)
    }

    fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        if matches!(intent, Intent::PublishDocument(_)) {
            return Reduction::default();
        }
        let reduction = self.route_intent(intent, cx);
        self.publish_document(reduction, cx)
    }
}
