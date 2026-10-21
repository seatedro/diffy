use std::rc::Rc;

use diffy_diff::Side;
use diffy_syntax::{GrammarStore, HighlightWorker};
use quark::selection::{Selection, SelectionPoint};
use quark_components::CollectionEnv;
use quark_text::{LayoutCache, TextSystem};
use quark_ui::Action;
use quark_ui::element::{AnyElement, ScrollHandle};
use quark_ui::theme::Theme;

#[cfg(doc)]
use super::DiffViewState;
use super::annotations::{DiffAnchor, DiffAnnotation};
use super::decorator::{DiffDecorator, DiffSources, DiffViewOptions};
use super::hover::HoverHit;
use super::navigation::{DiffTarget, FileId, RevealAlign, SourcePoint};
use super::prepared::{AnnotationId, DiffPreviewLimit, ViewFrame};
use super::presentation::{DiffAppearance, DiffPresentation};
use super::search::{FindOptions, SearchDirection, SearchSummary};
use super::selection::{CopyContent, LineHit};
use super::session::{DiffSessionViewState, forward};
use super::{DiffEvent, DiffOutcome, DiffStyle};

impl DiffSessionViewState {
    forward! {
        fn style() -> DiffStyle;
        fn presentation() -> DiffPresentation;
        fn scroll_offset() -> f32;
        fn content_height() -> f32;
        fn selection() -> Option<Selection>;
        fn frame() -> Option<&Rc<ViewFrame>>;
        fn horizontal_scroll(side: Side) -> &ScrollHandle;
        fn wants_frame() -> bool;
        fn selected_text() -> String;
        fn copy(content: CopyContent) -> Option<String>;
        fn search_summary() -> SearchSummary;
        fn focused_target() -> Option<DiffTarget>;
        fn point_at(x: f32, y: f32) -> Option<SelectionPoint>;
        fn is_file_collapsed(file: FileId) -> bool;
        fn syntax_pending() -> bool;
        fn focused_hunk() -> Option<(FileId, u32)>;
        fn top_file() -> Option<FileId>;
        fn selected_lines() -> Option<DiffAnchor>;
        fn line_at(x: f32, y: f32) -> Option<LineHit>;
        fn caret() -> Option<(SourcePoint, String)>;
        fn hover_at(x: f32, y: f32) -> Option<HoverHit>;
        fn has_link() -> bool;
    }

    forward! { mut
        fn set_style(style: DiffStyle) -> ();
        fn set_presentation(presentation: DiffPresentation) -> ();
        fn set_appearance(appearance: DiffAppearance) -> ();
        fn set_preview_limit(limit: Option<DiffPreviewLimit>) -> ();
        fn set_viewport(width: f32, height: f32) -> ();
        fn set_selection(selection: Option<Selection>, side: Side) -> ();
        fn select_all() -> ();
        fn focus_change(forward: bool) -> Option<LineHit>;
        fn reveal_target(target: DiffTarget, align: RevealAlign) -> bool;
        fn set_find_query(query: &str, options: FindOptions) -> ();
        fn next_match(direction: SearchDirection) -> Option<SourcePoint>;
        fn set_annotations(annotations: Vec<DiffAnnotation>) -> ();
        fn set_annotation_height(id: AnnotationId, revision: u64, height: f32) -> bool;
        fn handle(event: DiffEvent) -> DiffOutcome;
        fn set_file_collapsed(file: FileId, collapsed: bool) -> bool;
        fn enable_syntax(store: GrammarStore) -> ();
        fn enable_syntax_shared(worker: &HighlightWorker, store: GrammarStore) -> ();
        fn finish_syntax() -> bool;
        fn refresh_syntax() -> ();
        fn select_source(point: SourcePoint, len: u32) -> bool;
        fn set_link(link: Option<(SourcePoint, u32)>) -> bool;
    }

    /// See [`DiffViewState::set_syntax_wake`].
    pub fn set_syntax_wake(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.view.set_syntax_wake(wake);
    }

    /// Each annotation with whether it is outdated, in the order given.
    pub fn annotations(&self) -> impl Iterator<Item = (&DiffAnnotation, bool)> {
        self.view.annotations()
    }

    /// See [`DiffViewState::prepare`].
    pub fn prepare(
        &mut self,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        now_ms: u64,
    ) {
        self.view.prepare(text, layouts, scale, now_ms);
    }

    pub fn sources(&self) -> DiffSources {
        self.view.sources()
    }

    pub fn view_with_options(
        &mut self,
        theme: &Theme,
        env: CollectionEnv,
        on_event: fn(DiffEvent) -> Action,
        options: DiffViewOptions,
    ) -> AnyElement {
        super::view::diff_view_custom(&mut self.view, theme, env, on_event, options)
    }

    /// An anchor on `lines` of `file`'s current revision, when they exist.
    pub fn anchor_for(
        &self,
        file: FileId,
        side: Side,
        lines: std::ops::Range<u32>,
    ) -> Option<DiffAnchor> {
        self.view.anchor_for(file, side, lines)
    }
}

/// The session diff at its viewport size, as of the last
/// [`DiffSessionViewState::prepare`]; see [`super::diff_view`].
pub fn diff_session_view(
    state: &mut DiffSessionViewState,
    theme: &Theme,
    env: CollectionEnv,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    super::diff_view(&mut state.view, theme, env, on_event)
}

/// [`diff_session_view`] with the app's `decorator`; see
/// [`super::diff_view_with`].
pub fn diff_session_view_with(
    state: &mut DiffSessionViewState,
    theme: &Theme,
    env: CollectionEnv,
    on_event: fn(DiffEvent) -> Action,
    decorator: Option<Rc<dyn DiffDecorator>>,
) -> AnyElement {
    super::diff_view_with(&mut state.view, theme, env, on_event, decorator)
}
