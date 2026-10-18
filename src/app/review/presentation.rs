use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use diffy_ui::diff_view::AnnotationId;
use diffy_ui::diff_view::decorator::{AnnotationContext, DiffDecorator, HeaderContext, HeaderSlot};
use quark::view;
use quark_app::ViewContext;
use quark_components::{
    AvatarImage, Button, ButtonSize, ButtonStyle, PopoverSide, anchored, avatar, popover_panel,
};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::document::{
    DocumentEvent, DocumentStyle, MarkdownDocument, MarkdownEntry, RowChrome, RowDecorator,
    TextMeasurer,
};
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome, text_editor_element};
use quark_ui::theme::Theme;
use quark_ui::virtual_list::RowKey;

use super::code_highlighter::ReviewCodeHighlighter;
use super::composer::{ComposerEditor, ComposerTarget};
use super::state::ReviewController;
use crate::app::text_targets::REVIEW_COMPOSER;
use crate::app::tokens::{Rad, Sp, TextStyle};
use crate::contracts::review::{CardKey, ComposerFormat, ReviewAction};
use crate::contracts::shared::{FileKey, SharedContext, SourceTarget};
use crate::core::review::{
    ReviewDraft, ReviewDraftKind, ReviewDraftState, ReviewResolution, ReviewSession, ReviewSide,
    ReviewThread,
};
use crate::core::syntax::grammar::Grammars;

const MEASURE_HEIGHT: f32 = 100_000.0;
const COMPOSER_MIN_LINES: f32 = 3.0;
const COMPOSER_MAX_LINES: f32 = 12.0;

type Wake = Arc<dyn Fn() + Send + Sync>;

struct CardDoc {
    doc: MarkdownDocument,
    rows: Vec<CommentRow>,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommentRow {
    pub label: String,
    pub markdown: String,
    pub face: bool,
}

type Faces = Rc<RefCell<HashMap<String, AvatarImage>>>;

#[derive(Default)]
struct HostInner {
    docs: HashMap<CardKey, CardDoc>,
    composer: Option<ComposerEditor>,
    preview: Option<CardDoc>,
    theme: Option<Theme>,
    widths: HashMap<CardKey, f32>,
    grammars: Option<Arc<Grammars>>,
    wake: Option<Wake>,
    faces: Faces,
    viewport: (f32, f32),
    default_width: f32,
}

#[derive(Clone, Default)]
pub struct CardHost(Rc<RefCell<HostInner>>);

struct CommentChrome {
    faces: Faces,
}

impl RowDecorator for CommentChrome {
    fn header(&self, chrome: &RowChrome, _width: f32, theme: &Theme) -> Option<AnyElement> {
        let label = chrome.label.as_deref()?.to_owned();
        let login = label
            .trim_start_matches('@')
            .split_whitespace()
            .next()
            .unwrap_or("?")
            .to_owned();
        let scale = theme.metrics.ui_scale();
        let size = TextStyle::BodySmall.font_size(theme);
        let face = self
            .faces
            .try_borrow()
            .ok()
            .and_then(|faces| faces.get(&login).cloned());
        let test_id = format!("review.avatar.{login}");
        Some(view! { scale,
            <div class="flex-row items-center" gap={Sp::XS}>
                <div test_id={test_id}>{avatar(login).size(size * 1.4 / scale).image(face)}</div>
                <text size={size} class="font-medium truncate" color={theme.colors.text_strong}>
                    {label}
                </text>
            </div>
        })
    }
}

fn document_font(theme: &Theme) -> f32 {
    TextStyle::BodySmall.font_size(theme)
}

fn new_doc(inner: &mut HostInner, theme_font: f32, rows: &[CommentRow]) -> CardDoc {
    let mut doc = MarkdownDocument::new(DocumentStyle {
        pad_x: 0.0,
        ..DocumentStyle::for_font_size(theme_font)
    });
    doc.set_decorator(CommentChrome {
        faces: Rc::clone(&inner.faces),
    });
    let grammars: &Grammars = match inner.grammars.as_deref() {
        Some(grammars) => grammars,
        None => Grammars::shared(),
    };
    let worker = match inner.wake.clone() {
        Some(wake) => grammars.worker_waking(move || wake()),
        None => grammars.worker(),
    };
    doc.set_code_highlighter(ReviewCodeHighlighter::new(worker));
    let header = (theme_font * 1.8).round();
    let entries = rows.iter().enumerate().map(|(index, row)| MarkdownEntry {
        row: RowKey(index as u64),
        chrome: RowChrome {
            header_height: if row.label.is_empty() { 0.0 } else { header },
            label: (!row.label.is_empty()).then(|| Arc::from(row.label.as_str())),
            kind: u32::from(row.face),
            ..RowChrome::default()
        },
        markdown: row.markdown.clone(),
    });
    if let Err(error) = doc.extend(entries) {
        tracing::warn!("review card rows rejected: {error:?}");
    }
    CardDoc {
        doc,
        rows: rows.to_vec(),
        width: 0.0,
        height: 0.0,
    }
}

impl CardHost {
    fn with<R>(&self, f: impl FnOnce(&mut HostInner) -> R) -> Option<R> {
        self.0.try_borrow_mut().ok().map(|mut inner| f(&mut inner))
    }

    pub fn set_wake(&self, wake: Wake) {
        self.with(|inner| inner.wake = Some(wake));
    }

    pub fn set_face(&self, login: String, face: AvatarImage) {
        self.with(|inner| {
            if let Ok(mut faces) = inner.faces.try_borrow_mut() {
                faces.insert(login, face);
            }
        });
    }

    pub fn remove_face(&self, login: &str) {
        self.with(|inner| {
            if let Ok(mut faces) = inner.faces.try_borrow_mut() {
                faces.remove(login);
            }
        });
    }

    fn faces(&self) -> Faces {
        self.with(|inner| Rc::clone(&inner.faces))
            .unwrap_or_default()
    }

    pub fn set_grammars(&self, grammars: Arc<Grammars>) {
        self.with(|inner| inner.grammars = Some(grammars));
    }

    pub fn grammars_changed(&self) {
        self.with(|inner| {
            inner.docs.clear();
            inner.preview = None;
        });
    }

    pub fn reset_composer(&self, body: &str) {
        self.with(|inner| {
            inner.composer = Some(ComposerEditor::with_text(body));
            inner.preview = None;
        });
    }

    pub fn clear_composer(&self) {
        self.with(|inner| {
            inner.composer = None;
            inner.preview = None;
        });
    }

    pub fn composer_text(&self) -> String {
        self.with(|inner| {
            inner
                .composer
                .as_ref()
                .map(|c| c.editor.text().to_owned())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    }

    pub fn format_composer(&self, format: ComposerFormat) -> TextEditOutcome {
        self.with(|inner| inner.composer.as_mut().map(|c| c.format(format)))
            .flatten()
            .unwrap_or_default()
    }

    pub fn edit_composer(&self, command: TextEditCommand, now_ms: u64) -> TextEditOutcome {
        self.with(|inner| {
            inner
                .composer
                .as_mut()
                .map(|c| c.editor.apply_at(command, now_ms))
        })
        .flatten()
        .unwrap_or_default()
    }

    pub fn composer_preedit(&self, text: String, cursor: Option<(usize, usize)>) {
        self.with(|inner| {
            if let Some(composer) = inner.composer.as_mut() {
                composer.editor.set_preedit(text, cursor);
            }
        });
    }

    pub fn scroll_composer(&self, lines: i32) {
        self.with(|inner| {
            if let Some(composer) = inner.composer.as_mut() {
                let step = composer.editor.scroll_line_height_px();
                composer.editor.scroll(lines as f32 * step);
            }
        });
    }

    pub fn handle_document(&self, card: &CardKey, event: DocumentEvent) -> bool {
        self.with(|inner| {
            let doc = if *card == CardKey::NewComment {
                inner.preview.as_mut()
            } else {
                inner.docs.get_mut(card)
            };
            doc.map(|doc| doc.doc.handle(event)).is_some()
        })
        .unwrap_or(false)
    }

    pub fn selected_text(&self, card: &CardKey) -> Option<String> {
        self.with(|inner| inner.docs.get(card).map(|doc| doc.doc.selected_text()))
            .flatten()
            .filter(|text| !text.is_empty())
    }

    #[cfg(test)]
    pub fn select_all_for_test(&self, card: &CardKey) {
        self.with(|inner| {
            if let Some(doc) = inner.docs.get_mut(card) {
                doc.doc.document_mut().select_all();
            }
        });
    }

    fn sync(&self, cards: &[(CardKey, Vec<CommentRow>)], preview: Option<&str>) {
        let font = self
            .with(|inner| inner.theme.as_ref().map(document_font))
            .flatten()
            .unwrap_or(13.0);
        self.with(|inner| {
            inner
                .docs
                .retain(|key, _| cards.iter().any(|(card, _)| card == key));
            for (key, rows) in cards {
                if inner.docs.get(key).is_some_and(|doc| doc.rows == *rows) {
                    continue;
                }
                let doc = new_doc(inner, font, rows);
                inner.docs.insert(key.clone(), doc);
            }
            match preview {
                Some(markdown) => {
                    let row = [CommentRow {
                        face: false,
                        label: String::new(),
                        markdown: if markdown.trim().is_empty() {
                            "Nothing to preview".to_owned()
                        } else {
                            markdown.to_owned()
                        },
                    }];
                    if inner.preview.as_ref().is_none_or(|doc| doc.rows != row) {
                        inner.preview = Some(new_doc(inner, font, &row));
                    }
                }
                None => inner.preview = None,
            }
        });
    }

    pub fn prepare(&self, cx: &mut ViewContext, default_width: f32) -> bool {
        let theme = cx.theme.clone();
        let scale = cx.frame.scale_factor();
        let now_ms = cx.frame.elapsed().as_millis() as u64;
        let focused = cx.is_focused(REVIEW_COMPOSER);
        let viewport = cx.frame.size();
        let mut measuring = false;
        let changed = self
            .with(|inner| {
                inner.viewport = viewport;
                inner.default_width = default_width;
                let mut changed = inner.theme.as_ref() != Some(&theme);
                if changed {
                    inner.theme = Some(theme.clone());
                    for doc in inner.docs.values_mut().chain(inner.preview.as_mut()) {
                        doc.width = 0.0;
                    }
                }
                let ui = theme.metrics.ui_scale();
                let pad = (Sp::SM * ui).round();
                let font = document_font(&theme);
                let text = cx.frame.text();
                let widths = &inner.widths;
                let composer_key = CardKey::NewComment;
                for (key, card) in inner
                    .docs
                    .iter_mut()
                    .chain(inner.preview.as_mut().map(|doc| (&composer_key, doc)))
                {
                    let width = widths.get(key).copied().unwrap_or(default_width);
                    let inner_width = (width - pad * 2.0).max(1.0);
                    changed |= card.doc.poll_highlights();
                    let mut measurer =
                        TextMeasurer::new(&mut text.system, &mut text.layouts, font, scale);
                    card.doc.prepare(
                        inner_width,
                        MEASURE_HEIGHT,
                        now_ms,
                        theme.reduced_motion,
                        &mut measurer,
                    );
                    let height = card.doc.document().list().rows().total_extent() as f32;
                    let mut measurer =
                        TextMeasurer::new(&mut text.system, &mut text.layouts, font, scale);
                    card.doc.prepare(
                        inner_width,
                        height,
                        now_ms,
                        theme.reduced_motion,
                        &mut measurer,
                    );
                    measuring |= card.doc.is_measuring();
                    if (card.height - height).abs() > 0.5 || (card.width - inner_width).abs() > 0.5
                    {
                        card.height = height;
                        card.width = inner_width;
                        changed = true;
                    }
                }
                if let Some(composer) = inner.composer.as_mut() {
                    let width = widths.get(&composer_key).copied().unwrap_or(default_width);
                    composer.editor.set_clock(now_ms);
                    composer.editor.set_font_size(font);
                    composer.editor.set_scale_factor(scale);
                    let line = (font * 1.5).round();
                    let height = composer.editor.flush_fit(
                        &mut text.system,
                        (width - pad * 4.0).max(1.0),
                        line * COMPOSER_MIN_LINES,
                        line * COMPOSER_MAX_LINES,
                    );
                    if (composer.height - height).abs() > 0.5 || composer.focused != focused {
                        composer.height = height;
                        composer.focused = focused;
                        changed = true;
                    }
                }
                changed
            })
            .unwrap_or(false);
        if measuring {
            cx.frame.request_frame();
        }
        changed
    }

    fn record_width(&self, card: &CardKey, width: f32, composing: bool) {
        self.with(|inner| {
            inner.widths.insert(card.clone(), width);
            if composing {
                inner.widths.insert(CardKey::NewComment, width);
            }
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardTone {
    Open,
    Resolved,
    Pending,
    Submitting,
    Failed,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComposerView {
    pub card: CardKey,
    pub submit_label: &'static str,
    pub test_id: String,
    pub preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CardModel {
    pub key: CardKey,
    pub id: AnnotationId,
    pub target: Option<SourceTarget>,
    pub path: String,
    pub title: String,
    pub status: String,
    pub tone: CardTone,
    pub expanded: bool,
    pub summary: String,
    pub can_reply: bool,
    pub resolution: Option<(bool, &'static str)>,
    pub edit_node: Option<String>,
    pub delete_node: Option<String>,
    pub failure: Option<String>,
    pub retry_draft: Option<crate::core::review::ReviewDraftId>,
    pub file_level: Option<FileKey>,
    pub test_id: String,
    pub accessible: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReviewAnnotation {
    pub id: AnnotationId,
    pub card: CardKey,
    pub target: SourceTarget,
    pub revision: u64,
}

pub struct ReviewPresentation {
    revision: u64,
    cards: Rc<[CardModel]>,
    annotations: Rc<[ReviewAnnotation]>,
    files: Rc<[(String, FileKey, Vec<CardKey>)]>,
    open_file: Option<String>,
    composer: Option<ComposerView>,
    host: CardHost,
}

impl std::fmt::Debug for ReviewPresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewPresentation")
            .field("revision", &self.revision)
            .field("cards", &self.cards.len())
            .finish_non_exhaustive()
    }
}

pub fn annotation_id(key: &CardKey) -> AnnotationId {
    let mut hasher = DefaultHasher::new();
    "review".hash(&mut hasher);
    key.hash(&mut hasher);
    AnnotationId(hasher.finish())
}

pub fn card_test_id(key: &CardKey) -> String {
    match key {
        CardKey::Thread(id) => format!("review.thread.{}", id.0),
        CardKey::Draft(id) => format!("review.draft.{}", id.0),
        CardKey::NewComment => "review.new".to_owned(),
    }
}

fn content_revision(
    model: &CardModel,
    rows: &[CommentRow],
    composer: Option<&ComposerView>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    model.hash(&mut hasher);
    rows.hash(&mut hasher);
    composer.filter(|c| c.card == model.key).hash(&mut hasher);
    hasher.finish()
}

fn locate(
    anchor: Option<&crate::core::review::ReviewAnchor>,
    cx: &SharedContext,
) -> Option<SourceTarget> {
    let anchor = anchor?;
    let manifest = cx.manifest.as_ref()?;
    manifest.files.iter().find_map(|file| {
        anchor
            .matches_paths(&file.paths)
            .then(|| anchor.to_source_target(file.key, file.revision, &file.paths))
            .flatten()
    })
}

fn file_level(
    anchor: Option<&crate::core::review::ReviewAnchor>,
    cx: &SharedContext,
) -> Option<FileKey> {
    let anchor = anchor.filter(|anchor| anchor.active_line_range().is_none())?;
    cx.manifest
        .as_ref()?
        .files
        .iter()
        .find(|file| anchor.matches_paths(&file.paths))
        .map(|file| file.key)
}

pub fn thread_line_label(anchor: Option<&crate::core::review::ReviewAnchor>) -> String {
    let Some(anchor) = anchor else {
        return "Comment on file".to_owned();
    };
    let Some(line) = anchor.active_line_range().map(|range| range.github_line()) else {
        return "Comment on file".to_owned();
    };
    let side = match anchor.side {
        Some(ReviewSide::New) => "R",
        Some(ReviewSide::Old) => "L",
        None => "",
    };
    format!("Comment on line {side}{line}")
}

pub fn thread_status_parts(thread: &ReviewThread) -> (&'static str, String) {
    let word = match thread.status.resolution {
        ReviewResolution::Resolved => "Resolved",
        ReviewResolution::Unresolved => "Unresolved",
        ReviewResolution::Unknown => "Open",
    };
    let replies = thread.comments.len().saturating_sub(1);
    let mut meta = match replies {
        0 => String::new(),
        1 => "1 reply".to_owned(),
        n => format!("{n} replies"),
    };
    if thread.status.outdated {
        if !meta.is_empty() {
            meta.push_str(" · ");
        }
        meta.push_str("outdated");
    }
    (word, meta)
}

fn resolution_action(thread: &ReviewThread) -> Option<(bool, &'static str)> {
    match thread.status.resolution {
        ReviewResolution::Resolved if thread.permissions.can_unresolve => {
            Some((false, "Reopen thread"))
        }
        ReviewResolution::Unresolved | ReviewResolution::Unknown
            if thread.permissions.can_resolve =>
        {
            Some((true, "Resolve thread"))
        }
        _ => None,
    }
}

fn first_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("```"))
        .unwrap_or("(empty comment)")
        .to_owned()
}

fn comment_rows(thread: &ReviewThread, faces: &HashMap<String, AvatarImage>) -> Vec<CommentRow> {
    thread
        .comments
        .iter()
        .enumerate()
        .map(|(index, comment)| {
            let author = comment
                .author_login
                .as_deref()
                .filter(|login| !login.is_empty())
                .unwrap_or("unknown");
            let time = relative_time_label(comment.created_at.as_deref());
            let label = match (index == 0, time) {
                (true, Some(t)) => format!("@{author} · {t}"),
                (false, Some(t)) => format!("@{author} replied · {t}"),
                (true, None) => format!("@{author}"),
                (false, None) => format!("@{author} replied"),
            };
            let markdown = if comment.body.trim().is_empty() {
                "(empty comment)".to_owned()
            } else {
                comment.body.clone()
            };
            CommentRow {
                face: faces.contains_key(author),
                label,
                markdown,
            }
        })
        .collect()
}

fn thread_model(
    thread: &ReviewThread,
    session: &ReviewSession,
    expanded: bool,
    cx: &SharedContext,
) -> CardModel {
    let key = CardKey::Thread(thread.id.clone());
    let line_label = thread_line_label(thread.anchor.as_ref());
    let title = if thread.status.outdated {
        format!("{line_label} · outdated")
    } else {
        line_label.clone()
    };
    let (word, meta) = thread_status_parts(thread);
    let author = thread
        .comments
        .first()
        .and_then(|c| c.author_login.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let snippet = thread
        .comments
        .first()
        .map(|c| first_line(&c.body))
        .unwrap_or_else(|| "(no comments)".to_owned());
    let failed = session
        .drafts
        .values()
        .find_map(|draft| match (&draft.kind, &draft.state) {
            (ReviewDraftKind::Reply { thread_id }, ReviewDraftState::Failed(message))
                if *thread_id == thread.id =>
            {
                Some((draft.id, message.clone()))
            }
            _ => None,
        });
    let first = thread.comments.first();
    let status = if meta.is_empty() {
        word.to_owned()
    } else {
        format!("{word}, {meta}")
    };
    CardModel {
        id: annotation_id(&key),
        target: locate(thread.anchor.as_ref(), cx),
        path: thread.path().unwrap_or_default().to_owned(),
        accessible: format!(
            "Review thread, {line_label}, {status}, {}",
            if expanded { "expanded" } else { "collapsed" }
        ),
        title,
        status: word.to_owned(),
        tone: if thread.status.resolution == ReviewResolution::Resolved {
            CardTone::Resolved
        } else {
            CardTone::Open
        },
        expanded,
        summary: format!("@{author}: {snippet}"),
        can_reply: thread.permissions.can_reply,
        resolution: resolution_action(thread),
        edit_node: first
            .filter(|c| c.viewer_can_update)
            .and_then(|c| c.backend_node_id.clone()),
        delete_node: first
            .filter(|c| c.viewer_can_delete)
            .and_then(|c| c.backend_node_id.clone()),
        failure: failed.as_ref().map(|(_, message)| message.clone()),
        retry_draft: failed.map(|(id, _)| id),
        file_level: file_level(thread.anchor.as_ref(), cx),
        test_id: card_test_id(&key),
        key,
    }
}

fn draft_model(draft: &ReviewDraft, cx: &SharedContext) -> (CardModel, Vec<CommentRow>) {
    let key = CardKey::Draft(draft.id);
    let (status, tone, failure) = match &draft.state {
        ReviewDraftState::Pending | ReviewDraftState::Submitted { .. } => {
            ("Pending", CardTone::Pending, None)
        }
        ReviewDraftState::Submitting => ("Submitting", CardTone::Submitting, None),
        ReviewDraftState::Failed(message) => ("Failed", CardTone::Failed, Some(message.clone())),
    };
    let title = match draft.kind {
        ReviewDraftKind::General => "Review summary".to_owned(),
        _ => thread_line_label(draft.anchor.as_ref()),
    };
    let model = CardModel {
        id: annotation_id(&key),
        target: locate(draft.anchor.as_ref(), cx),
        path: draft
            .anchor
            .as_ref()
            .map(|a| a.path.clone())
            .unwrap_or_default(),
        accessible: format!("Pending review comment, {title}, {status}"),
        title,
        status: status.to_owned(),
        tone,
        expanded: true,
        summary: first_line(&draft.body),
        can_reply: false,
        resolution: None,
        edit_node: None,
        delete_node: None,
        failure,
        retry_draft: None,
        file_level: file_level(draft.anchor.as_ref(), cx),
        test_id: card_test_id(&key),
        key,
    };
    let rows = vec![CommentRow {
        face: false,
        label: String::new(),
        markdown: if draft.body.trim().is_empty() {
            "(empty comment)".to_owned()
        } else {
            draft.body.clone()
        },
    }];
    (model, rows)
}

impl ReviewPresentation {
    pub fn empty(host: CardHost) -> Self {
        Self {
            revision: 0,
            cards: Rc::from([]),
            annotations: Rc::from([]),
            files: Rc::from([]),
            open_file: None,
            composer: None,
            host,
        }
    }

    pub fn bumped(&self) -> Self {
        Self {
            revision: self.revision + 1,
            cards: Rc::clone(&self.cards),
            annotations: Rc::clone(&self.annotations),
            files: Rc::clone(&self.files),
            open_file: self.open_file.clone(),
            composer: self.composer.clone(),
            host: self.host.clone(),
        }
    }

    pub(super) fn build(controller: &ReviewController, cx: &SharedContext) -> Self {
        let previous = controller.presentation();
        let composer = controller.composer.as_ref().map(|composer| ComposerView {
            card: composer.target.card(),
            submit_label: composer.target.submit_label(),
            test_id: composer.target.test_id(),
            preview: composer.preview,
        });
        let mut cards = Vec::new();
        let mut docs = Vec::new();
        let faces = controller.host.faces();
        let faces = faces.try_borrow().map(|f| f.clone()).unwrap_or_default();
        let composing = controller.composer.as_ref().and_then(|c| match &c.target {
            ComposerTarget::Inline { draft, .. } | ComposerTarget::General { draft } => *draft,
            _ => None,
        });
        if let Some(session) = controller.session() {
            for thread in &session.threads {
                let expanded = controller.thread_expanded(&thread.id);
                let model = thread_model(thread, session, expanded, cx);
                docs.push((model.key.clone(), comment_rows(thread, &faces)));
                cards.push(model);
            }
            for draft in session.drafts.values() {
                if matches!(draft.kind, ReviewDraftKind::Reply { .. })
                    || Some(draft.id) == composing
                {
                    continue;
                }
                let (model, rows) = draft_model(draft, cx);
                docs.push((model.key.clone(), rows));
                cards.push(model);
            }
        }
        let fresh = controller
            .composer
            .as_ref()
            .and_then(|composer| match &composer.target {
                ComposerTarget::Inline { path, target, .. } => {
                    let anchor =
                        crate::core::review::ReviewAnchor::from_source_target(path.clone(), target);
                    Some((
                        thread_line_label(Some(&anchor)),
                        Some(target.clone()),
                        path.clone(),
                    ))
                }
                ComposerTarget::General { .. } => {
                    Some(("Review summary".to_owned(), None, String::new()))
                }
                _ => None,
            });
        if let Some((title, target, path)) = fresh {
            let key = CardKey::NewComment;
            cards.push(CardModel {
                id: annotation_id(&key),
                target,
                path,
                accessible: format!("New review comment, {title}"),
                title,
                status: "New".to_owned(),
                tone: CardTone::New,
                expanded: true,
                summary: String::new(),
                can_reply: false,
                resolution: None,
                edit_node: None,
                delete_node: None,
                failure: None,
                retry_draft: None,
                file_level: None,
                test_id: card_test_id(&key),
                key,
            });
        }
        let preview = composer
            .as_ref()
            .filter(|c| c.preview)
            .map(|_| controller.host.composer_text());
        controller.host.sync(&docs, preview.as_deref());
        let annotations = cards
            .iter()
            .filter_map(|card| {
                let rows = docs
                    .iter()
                    .find(|(key, _)| *key == card.key)
                    .map_or(&[][..], |(_, rows)| rows.as_slice());
                Some(ReviewAnnotation {
                    id: card.id,
                    card: card.key.clone(),
                    target: card.target.clone()?,
                    revision: content_revision(card, rows, composer.as_ref()),
                })
            })
            .collect::<Vec<_>>();
        let mut files: Vec<(String, FileKey, Vec<CardKey>)> = Vec::new();
        for card in &cards {
            let Some(file) = card.file_level else {
                continue;
            };
            match files.iter_mut().find(|(_, key, _)| *key == file) {
                Some((_, _, keys)) => keys.push(card.key.clone()),
                None => files.push((card.path.clone(), file, vec![card.key.clone()])),
            }
        }
        Self {
            revision: previous.revision + 1,
            cards: cards.into(),
            annotations: annotations.into(),
            files: files.into(),
            open_file: controller.file_comments.clone(),
            composer,
            host: controller.host.clone(),
        }
    }

    pub fn file_comments(
        &self,
        source: Option<FileKey>,
        title: &str,
    ) -> Option<(&str, &[CardKey])> {
        self.files
            .iter()
            .find(|(path, key, _)| match source {
                Some(source) => source == *key,
                None => title == path || title.split(" \u{2192} ").any(|part| part == path),
            })
            .map(|(path, _, keys)| (path.as_str(), keys.as_slice()))
    }

    pub fn general_composer(&self) -> Option<&CardModel> {
        self.card(&CardKey::NewComment)
            .filter(|card| card.target.is_none() && card.path.is_empty())
    }

    pub fn annotations(&self) -> &[ReviewAnnotation] {
        &self.annotations
    }

    pub fn cards(&self) -> &[CardModel] {
        &self.cards
    }

    pub fn card(&self, key: &CardKey) -> Option<&CardModel> {
        self.cards.iter().find(|card| card.key == *key)
    }

    pub fn card_element(&self, key: &CardKey, width: f32) -> Option<AnyElement> {
        let card = self.card(key)?;
        let composing = self.composer.as_ref().is_some_and(|c| c.card == card.key);
        self.host.record_width(&card.key, width, composing);
        let mut inner = self.host.0.try_borrow_mut().ok()?;
        let inner = &mut *inner;
        let theme = inner.theme.clone().unwrap_or_else(Theme::default_dark);
        Some(card_view(
            card,
            self.composer.as_ref(),
            inner,
            &theme,
            width,
        ))
    }
}

impl DiffDecorator for ReviewPresentation {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn annotation(&self, cx: &AnnotationContext) -> Option<AnyElement> {
        let key = self.cards.iter().find(|card| card.id == cx.id)?.key.clone();
        self.card_element(&key, cx.width)
    }

    fn header_slot(&self, slot: HeaderSlot, cx: &HeaderContext) -> Option<AnyElement> {
        if slot != HeaderSlot::Metadata {
            return None;
        }
        self.file_comments_element(cx.source.map(|source| source.id), cx.title)
    }
}

impl ReviewPresentation {
    pub fn file_comments_element(
        &self,
        source: Option<FileKey>,
        title: &str,
    ) -> Option<AnyElement> {
        let (path, keys) = self.file_comments(source, title)?;
        let (theme, viewport, width) = self
            .host
            .with(|inner| (inner.theme.clone(), inner.viewport, inner.default_width))?;
        let theme = theme.unwrap_or_else(Theme::default_dark);
        let label = match keys.len() {
            1 => "1 file comment".to_owned(),
            n => format!("{n} file comments"),
        };
        let open = self.open_file.as_deref() == Some(path);
        let toggle = Button::new(action(ReviewAction::ToggleFileComments(path.to_owned())))
            .label(label)
            .style(ButtonStyle::Ghost)
            .size(ButtonSize::Compact)
            .active(open);
        let popover = open.then(|| {
            let cards: Vec<AnyElement> = keys
                .iter()
                .filter_map(|key| self.card_element(key, width))
                .collect();
            let scale = theme.metrics.ui_scale();
            let gap = (Sp::XS * scale).round();
            let panel = popover_panel(&theme)
                .flex_col()
                .gap(gap)
                .p(gap)
                .accessibility_role(AccessibilityRole::Group)
                .accessibility_label(format!("File comments on {path}"))
                .test_id(format!("review.file_comments.{path}"))
                .children(cards);
            anchored(panel, PopoverSide::Bottom, viewport)
        });
        Some(view! {
            <div class="relative" test_id={format!("review.file.{path}")}>
                {toggle}
                {?popover}
            </div>
        })
    }
}

fn action(action: ReviewAction) -> quark_ui::Action {
    action.into()
}

fn button(label: &str, on_click: ReviewAction, style: ButtonStyle, test_id: String) -> AnyElement {
    let button = Button::new(action(on_click))
        .label(label)
        .style(style)
        .size(ButtonSize::Compact);
    view! { <div test_id={test_id}>{button}</div> }
}

fn tone_color(tone: CardTone, theme: &Theme) -> quark_ui::theme::Color {
    let tc = &theme.colors;
    match tone {
        CardTone::Resolved => tc.text_muted,
        CardTone::Open | CardTone::New => tc.accent,
        CardTone::Pending | CardTone::Submitting => tc.status_warning,
        CardTone::Failed => tc.status_error,
    }
}

fn card_view(
    card: &CardModel,
    composer: Option<&ComposerView>,
    inner: &mut HostInner,
    theme: &Theme,
    width: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let small = TextStyle::BodySmall.font_size(theme);
    let composer = composer.filter(|c| c.card == card.key);
    let key = card.key.clone();
    let status_color = tone_color(card.tone, theme);
    let header = header_view(card, theme, status_color);
    let body = if !card.expanded {
        Some(view! { scale,
            <div class="flex-row" pl={Sp::MD} min_w={0.0}>
                <text size={small} class="truncate" color={tc.text_muted}>{card.summary.clone()}</text>
            </div>
        })
    } else {
        inner.docs.get_mut(&card.key).map(|doc| {
            let height = doc.height;
            let card_key = key.clone();
            let element = doc
                .doc
                .element(theme, move |event| {
                    action(ReviewAction::CardDocument {
                        card: card_key.clone(),
                        event,
                    })
                })
                .label(card.title.clone());
            view! { <div w={doc.width} h={height}>{element}</div> }
        })
    };
    let failure = card.failure.clone().map(|message| {
        let retry = card.retry_draft.map(|id| {
            button(
                "Retry",
                ReviewAction::EditDraft(id),
                ButtonStyle::Subtle,
                format!("review.retry.{}", id.0),
            )
        });
        view! { scale,
            <div class="flex-row items-center" gap={Sp::XS}>
                <icon svg={lucide::ALERT_CIRCLE} size={small} color={tc.status_error} />
                <div class="flex-1" min_w={0.0}>
                    <text size={small} class="truncate" color={tc.status_error}>{message}</text>
                </div>
                {?retry}
            </div>
        }
    });
    let composer_view = composer
        .filter(|_| card.expanded)
        .map(|composer| composer_view(composer, inner, theme));
    let footer = card
        .expanded
        .then(|| footer_view(card, composer.is_some()))
        .flatten();
    let label = card.accessible.clone();
    view! { scale,
        <div
            class="flex-col"
            w={width}
            bg={tc.elevated_surface}
            border={tc.border}
            rounded={Rad::LG}
            p={Sp::SM}
            gap={Sp::XS}
            accessibility_role={AccessibilityRole::Group}
            aria-label={label}
            test_id={card.test_id.clone()}
            on:key:mod+c={action(ReviewAction::CopyCardSelection(key))}
        >
            {header}
            {?body}
            {?failure}
            {?composer_view}
            {?footer}
        </div>
    }
}

fn header_view(
    card: &CardModel,
    theme: &Theme,
    status_color: quark_ui::theme::Color,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let small = TextStyle::BodySmall.font_size(theme);
    let toggle = match &card.key {
        CardKey::Thread(id) => {
            let label = if card.expanded {
                "Collapse thread"
            } else {
                "Expand thread"
            };
            let icon = if card.expanded {
                lucide::CHEVRON_DOWN
            } else {
                lucide::CHEVRON_RIGHT
            };
            let toggle = Button::new(action(ReviewAction::ToggleThread(id.clone())))
                .icon(icon)
                .tooltip(label)
                .style(ButtonStyle::Ghost)
                .size(ButtonSize::Compact);
            Some(view! { <div test_id={format!("review.toggle.{}", id.0)}>{toggle}</div> })
        }
        _ => None,
    };
    view! { scale,
        <div class="flex-row items-center" gap={Sp::XS}>
            {?toggle}
            <div class="flex-1" min_w={0.0}>
                <text size={small} class="font-medium truncate" color={tc.text_strong}>
                    {card.title.clone()}
                </text>
            </div>
            <div class="flex-row items-center" px={Sp::SM} py={Sp::XXS} rounded={Rad::SM} bg={tc.element_background}>
                <text size={small * 0.9} class="font-medium" color={status_color}>{card.status.clone()}</text>
            </div>
        </div>
    }
}

fn footer_view(card: &CardModel, composing: bool) -> Option<AnyElement> {
    let mut buttons = Vec::new();
    match &card.key {
        CardKey::Thread(id) => {
            if card.can_reply && !composing {
                buttons.push(button(
                    "Reply",
                    ReviewAction::ReplyToThread(id.clone()),
                    ButtonStyle::Subtle,
                    format!("review.reply.{}", id.0),
                ));
            }
            if let Some((resolved, label)) = card.resolution {
                buttons.push(button(
                    label,
                    ReviewAction::SetThreadResolved {
                        id: id.clone(),
                        resolved,
                    },
                    ButtonStyle::Subtle,
                    format!("review.resolve.{}", id.0),
                ));
            }
            if let Some(node) = card.edit_node.clone().filter(|_| !composing) {
                buttons.push(button(
                    "Edit",
                    ReviewAction::EditComment {
                        comment_node_id: node,
                    },
                    ButtonStyle::Ghost,
                    format!("review.edit.{}", id.0),
                ));
            }
            if let Some(node) = card.delete_node.clone() {
                buttons.push(button(
                    "Delete",
                    ReviewAction::DeleteComment {
                        comment_node_id: node,
                    },
                    ButtonStyle::Ghost,
                    format!("review.delete.{}", id.0),
                ));
            }
        }
        CardKey::Draft(id) if !composing && card.tone != CardTone::Submitting => {
            buttons.push(button(
                "Edit",
                ReviewAction::EditDraft(*id),
                ButtonStyle::Subtle,
                format!("review.draft.edit.{}", id.0),
            ));
            buttons.push(button(
                "Delete",
                ReviewAction::DeleteDraft(*id),
                ButtonStyle::Ghost,
                format!("review.draft.delete.{}", id.0),
            ));
        }
        CardKey::Draft(_) | CardKey::NewComment => {}
    }
    if buttons.is_empty() {
        return None;
    }
    Some(view! {
        <div class="flex-row items-center gap-1">
            {...buttons}
            <spacer />
        </div>
    })
}

fn composer_view(composer: &ComposerView, inner: &mut HostInner, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let small = TextStyle::BodySmall.font_size(theme);
    let tab = |label: &str, preview: bool| {
        Button::new(action(ReviewAction::SetComposerPreview(preview)))
            .label(label)
            .style(ButtonStyle::Ghost)
            .size(ButtonSize::Compact)
            .active(composer.preview == preview)
    };
    let format = |label: &str, tooltip: &str, format: ComposerFormat| {
        Button::new(action(ReviewAction::FormatComment(format)))
            .label(label)
            .tooltip(tooltip.to_owned())
            .style(ButtonStyle::Ghost)
            .size(ButtonSize::Compact)
            .disabled(composer.preview)
    };
    let content = if composer.preview {
        inner.preview.as_mut().map(|doc| {
            let height = doc.height;
            let element = doc
                .doc
                .element(theme, |event| {
                    action(ReviewAction::CardDocument {
                        card: CardKey::NewComment,
                        event,
                    })
                })
                .label("Comment preview");
            view! { <div w={doc.width} h={height}>{element}</div> }
        })
    } else {
        inner.composer.as_ref().map(|editor| {
            let element = text_editor_element(
                REVIEW_COMPOSER,
                ScrollActionBuilder::new(|lines| action(ReviewAction::ScrollComposer(lines))),
            )
            .editor_snapshot(&editor.editor)
            .label("Review comment")
            .placeholder("Leave a comment")
            .focused(editor.focused)
            .font_size(small)
            .text_color(tc.text);
            view! { scale,
                <div
                    class="w-full"
                    h={editor.height}
                    p={Sp::XS}
                    rounded={Rad::MD}
                    bg={tc.editor_surface}
                    border={if editor.focused { tc.focus_border } else { tc.control_border }}
                >
                    {element}
                </div>
            }
        })
    };
    let submit = Button::new(action(ReviewAction::SubmitComment))
        .label(composer.submit_label)
        .style(ButtonStyle::Filled)
        .size(ButtonSize::Compact);
    let cancel = Button::new(action(ReviewAction::CancelComment))
        .label("Cancel")
        .style(ButtonStyle::Ghost)
        .size(ButtonSize::Compact);
    view! { scale,
        <div
            class="flex-col"
            gap={Sp::XS}
            test_id={composer.test_id.clone()}
            on:key:mod+enter={action(ReviewAction::SubmitComment)}
            on:key:escape={action(ReviewAction::CancelComment)}
            on:key:mod+k={action(ReviewAction::FormatComment(ComposerFormat::Link))}
        >
            <div class="flex-row items-center" gap={Sp::XXS}>
                {tab("Write", false)}
                {tab("Preview", true)}
                <spacer />
                {format("B", "Bold", ComposerFormat::Bold)}
                {format("I", "Italic", ComposerFormat::Italic)}
                {format("<>", "Code", ComposerFormat::Code)}
                {format("Link", "Link", ComposerFormat::Link)}
                {format("List", "Bulleted list", ComposerFormat::BulletList)}
            </div>
            {?content}
            <div class="flex-row items-center" gap={Sp::XS}>
                <spacer />
                {cancel}
                {submit}
            </div>
        </div>
    }
}

pub fn relative_time_label(iso: Option<&str>) -> Option<String> {
    let then = parse_rfc3339_utc_seconds(iso?)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let elapsed = now.saturating_sub(then);
    Some(if elapsed < 60 {
        "just now".to_owned()
    } else if elapsed < 3_600 {
        plural_time(elapsed / 60, "minute")
    } else if elapsed < 86_400 {
        plural_time(elapsed / 3_600, "hour")
    } else if elapsed < 2_592_000 {
        plural_time(elapsed / 86_400, "day")
    } else if elapsed < 31_536_000 {
        plural_time(elapsed / 2_592_000, "month")
    } else {
        plural_time(elapsed / 31_536_000, "year")
    })
}

fn plural_time(value: u64, unit: &str) -> String {
    if value == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{value} {unit}s ago")
    }
}

fn parse_rfc3339_utc_seconds(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<u32> {
        bytes.get(range)?.iter().try_fold(0_u32, |value, byte| {
            byte.is_ascii_digit()
                .then(|| value * 10 + u32::from(byte - b'0'))
        })
    };
    let year = digits(0..4)? as i32;
    let month = digits(5..7)?;
    let day = digits(8..10)?;
    let hour = u64::from(digits(11..13)?);
    let minute = u64::from(digits(14..16)?);
    let second = u64::from(digits(17..19)?);
    if hour > 23 || minute > 59 || second > 60 || !(1..=12).contains(&month) || day == 0 || day > 31
    {
        return None;
    }
    let year = year - i32::from(month <= 2);
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = i64::from(era * 146_097 + doe - 719_468);
    (days >= 0).then(|| days as u64 * 86_400 + hour * 3_600 + minute * 60 + second)
}
