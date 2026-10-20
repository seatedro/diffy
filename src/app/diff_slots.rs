use std::rc::Rc;
use std::sync::Arc;

use diffy_ui::diff_view::decorator::{
    AnnotationContext, DiffDecorator, GutterContext, HeaderContext, HeaderSlot, LineContext,
    LineMark, SeparatorContext,
};
use quark::view;
use quark_ui::element::*;
use quark_ui::style::Styled;

use crate::app::tokens::Sp;

type SlotKey = (u64, u64, u64, *const (), *const (), *const (), u32);

#[derive(Default)]
pub struct DiffSlots {
    revision: u64,
    cached: Option<(SlotKey, Rc<AppDiffDecorator>)>,
    empty: Option<Rc<NoDecorations>>,
}

impl DiffSlots {
    pub fn decorator(
        &mut self,
        review: Rc<dyn DiffDecorator>,
        mutations: Rc<dyn DiffDecorator>,
        navigation: Rc<dyn DiffDecorator>,
        scale: f32,
    ) -> Rc<dyn DiffDecorator> {
        let key = (
            review.revision(),
            mutations.revision(),
            navigation.revision(),
            Rc::as_ptr(&review).cast::<()>(),
            Rc::as_ptr(&mutations).cast::<()>(),
            Rc::as_ptr(&navigation).cast::<()>(),
            scale.to_bits(),
        );
        if let Some((cached, decorator)) = &self.cached
            && *cached == key
        {
            return decorator.clone();
        }
        self.revision = self.revision.wrapping_add(1);
        let decorator = Rc::new(AppDiffDecorator {
            revision: self.revision,
            review,
            mutations,
            navigation,
            scale,
        });
        self.cached = Some((key, decorator.clone()));
        decorator
    }

    pub fn empty(&mut self) -> Rc<dyn DiffDecorator> {
        self.empty
            .get_or_insert_with(|| Rc::new(NoDecorations))
            .clone()
    }
}

pub struct NoDecorations;

impl DiffDecorator for NoDecorations {
    fn revision(&self) -> u64 {
        0
    }
}

pub struct AppDiffDecorator {
    revision: u64,
    review: Rc<dyn DiffDecorator>,
    mutations: Rc<dyn DiffDecorator>,
    navigation: Rc<dyn DiffDecorator>,
    scale: f32,
}

impl AppDiffDecorator {
    fn both(&self, first: Option<AnyElement>, second: Option<AnyElement>) -> Option<AnyElement> {
        match (first, second) {
            (Some(first), Some(second)) => {
                let scale = self.scale;
                Some(view! { scale,
                    <div class="flex-row items-center" gap={Sp::XS}>
                        {first}
                        {second}
                    </div>
                })
            }
            (first, second) => first.or(second),
        }
    }
}

impl DiffDecorator for AppDiffDecorator {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn header_slot(&self, slot: HeaderSlot, cx: &HeaderContext) -> Option<AnyElement> {
        self.both(
            self.review.header_slot(slot, cx),
            self.mutations.header_slot(slot, cx),
        )
    }

    fn header(&self, cx: &HeaderContext) -> Option<AnyElement> {
        self.mutations.header(cx).or_else(|| self.review.header(cx))
    }

    fn separator(&self, cx: &SeparatorContext) -> Option<AnyElement> {
        self.mutations
            .separator(cx)
            .or_else(|| self.review.separator(cx))
    }

    fn annotation(&self, cx: &AnnotationContext) -> Option<AnyElement> {
        self.review
            .annotation(cx)
            .or_else(|| self.mutations.annotation(cx))
    }

    fn gutter_utility(&self, cx: &GutterContext) -> Option<AnyElement> {
        self.both(
            self.mutations.gutter_utility(cx),
            self.review.gutter_utility(cx),
        )
    }

    fn line_mark(&self, cx: &LineContext) -> Option<LineMark> {
        self.mutations
            .line_mark(cx)
            .or_else(|| self.review.line_mark(cx))
    }

    fn file_state(&self, cx: &HeaderContext) -> Option<String> {
        self.mutations
            .file_state(cx)
            .or_else(|| self.review.file_state(cx))
    }

    fn hunk_scope(&self, cx: &SeparatorContext) -> Option<Arc<[Arc<str>]>> {
        self.mutations
            .hunk_scope(cx)
            .or_else(|| self.review.hunk_scope(cx))
    }

    fn banner(&self, width: f32) -> Option<(f32, AnyElement)> {
        self.review
            .banner(width)
            .or_else(|| self.mutations.banner(width))
            .or_else(|| self.navigation.banner(width))
    }

    fn hunk_actions(&self, cx: &SeparatorContext) -> Option<AnyElement> {
        self.both(
            self.mutations.hunk_actions(cx),
            self.review.hunk_actions(cx),
        )
    }
}
