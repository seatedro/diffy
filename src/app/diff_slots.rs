use diffy_ui::diff_view::decorator::{
    AnnotationContext, DiffDecorator, GutterContext, HeaderContext, HeaderSlot, SeparatorContext,
};
use quark::view;
use quark_ui::element::*;
use quark_ui::style::Styled;

use crate::app::tokens::Sp;

#[derive(Debug, Default)]
pub struct DiffSlots {
    revision: u64,
    sources: Option<(u64, u64)>,
}

impl DiffSlots {
    pub fn decorator<'a>(
        &mut self,
        review: &'a dyn DiffDecorator,
        mutations: &'a dyn DiffDecorator,
        scale: f32,
    ) -> AppDiffDecorator<'a> {
        let sources = Some((review.revision(), mutations.revision()));
        if self.sources != sources {
            self.sources = sources;
            self.revision = self.revision.wrapping_add(1);
        }
        AppDiffDecorator {
            revision: self.revision,
            review,
            mutations,
            scale,
        }
    }
}

pub struct AppDiffDecorator<'a> {
    revision: u64,
    review: &'a dyn DiffDecorator,
    mutations: &'a dyn DiffDecorator,
    scale: f32,
}

impl AppDiffDecorator<'_> {
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

impl DiffDecorator for AppDiffDecorator<'_> {
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
}
