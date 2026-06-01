use std::sync::Arc;

use crate::core::text::{DiffTokenSpan, SyntaxTokenKind};
use crate::editor::{Editor, EditorMode, SelectionRect};
use crate::render::scene::{
    FontKind, FontStyle, FontWeight, Rect, RichTextPrimitive, RichTextSpan, TextPrimitive,
};
use crate::render::{RectPrimitive, RoundedRectPrimitive, Scene};
use crate::ui::accessibility::{AccessibilityAction, AccessibilityNode};
use crate::ui::design::{Alpha, Sz};
use crate::ui::element::*;
use crate::ui::state::FocusTarget;
use crate::ui::style::{ElementStyle, Styled};

pub struct CursorSnapshot {
    pub x: f32,
    pub y: f32,
    pub moved_at_ms: u64,
}

pub struct TextEditorElement {
    is_empty: bool,
    placeholder: String,
    focused: bool,
    cursor: Option<CursorSnapshot>,
    selection_rects: Vec<SelectionRect>,
    content_height: f32,
    scroll_y: f32,
    font_size: f32,
    text_color: crate::ui::theme::Color,
    mode: EditorMode,
    text: Arc<str>,
    syntax_spans: Vec<DiffTokenSpan>,
    line_tops: Vec<(usize, f32)>,
    focus_target: FocusTarget,
    base_style: ElementStyle,
}

pub fn text_editor_element() -> TextEditorElement {
    TextEditorElement {
        is_empty: true,
        placeholder: String::new(),
        focused: false,
        cursor: None,
        selection_rects: Vec::new(),
        content_height: 0.0,
        scroll_y: 0.0,
        font_size: 14.0,
        text_color: crate::ui::theme::Color::rgba(255, 255, 255, 255),
        mode: EditorMode::ProseInput,
        text: Arc::from(""),
        syntax_spans: Vec::new(),
        line_tops: Vec::new(),
        focus_target: FocusTarget::FileList,
        base_style: ElementStyle::default(),
    }
}

impl TextEditorElement {
    pub fn placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = p.into();
        self
    }

    pub fn focused(mut self, f: bool) -> Self {
        self.focused = f;
        self
    }

    pub fn is_empty(mut self, empty: bool) -> Self {
        self.is_empty = empty;
        self
    }

    pub fn cursor(mut self, snap: CursorSnapshot) -> Self {
        self.cursor = Some(snap);
        self
    }

    pub fn selection(mut self, rects: Vec<SelectionRect>) -> Self {
        self.selection_rects = rects;
        self
    }

    pub fn content_height(mut self, h: f32) -> Self {
        self.content_height = h;
        self
    }

    pub fn scroll_y(mut self, offset: f32) -> Self {
        self.scroll_y = offset;
        self
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn text_color(mut self, color: crate::ui::theme::Color) -> Self {
        self.text_color = color;
        self
    }

    pub fn mode(mut self, mode: EditorMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn text(mut self, text: Arc<str>) -> Self {
        self.text = text;
        self
    }

    pub fn syntax_spans(mut self, spans: &[DiffTokenSpan]) -> Self {
        self.syntax_spans.clear();
        self.syntax_spans.extend_from_slice(spans);
        self
    }

    pub fn line_tops(mut self, line_tops: Vec<(usize, f32)>) -> Self {
        self.line_tops = line_tops;
        self
    }

    pub fn editor_snapshot(mut self, editor: &Editor) -> Self {
        self.is_empty = editor.is_empty();
        self.cursor = Some(CursorSnapshot {
            x: editor.cursor_pos.x,
            y: editor.cursor_pos.y,
            moved_at_ms: editor.cursor_moved_at_ms,
        });
        self.selection_rects = editor.selection_rects();
        self.content_height = editor.content_height();
        self.scroll_y = editor.scroll_y;
        self.mode = editor.mode();
        self.text = editor.text_arc();
        self.syntax_spans = editor.syntax_spans().to_vec();
        self.line_tops = editor.logical_line_tops();
        self
    }

    pub fn focus_target(mut self, target: FocusTarget) -> Self {
        self.focus_target = target;
        self
    }
}

impl Styled for TextEditorElement {
    fn element_style_mut(&mut self) -> &mut ElementStyle {
        &mut self.base_style
    }
}

impl Element for TextEditorElement {
    type LayoutState = ();
    type PrepaintState = ();

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        _cx: &mut ElementContext,
    ) -> (LayoutId, ()) {
        let id = engine.request_layout(self.base_style.layout.clone(), &[]);
        (id, ())
    }

    fn prepaint(
        &mut self,
        bounds: Bounds,
        _layout_state: &mut (),
        _engine: &LayoutEngine,
        cx: &mut ElementContext,
    ) {
        cx.insert_hitbox(bounds, HitboxBehavior::Normal);
        cx.scroll_regions.push(ScrollRegion {
            bounds,
            action_builder: ScrollActionBuilder::Custom(crate::actions::editor_scroll_px),
        });
    }

    fn paint(
        &mut self,
        bounds: Bounds,
        _layout_state: &mut (),
        _prepaint_state: &mut (),
        _engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        let theme = cx.theme;
        let accessibility_label = if self.placeholder.is_empty() {
            format!("{:?}", self.focus_target)
        } else {
            self.placeholder.clone()
        };
        let font_size = self.font_size;
        let line_height = font_size * 1.35;
        let gutter_w = if self.mode.is_code() {
            code_gutter_width(
                font_size,
                self.line_tops.last().map(|(line, _)| *line).unwrap_or(0),
            )
            .min((bounds.width * 0.35).max(0.0))
        } else {
            0.0
        };
        let text_area_w = (bounds.width - gutter_w).max(0.0);
        let text_x = bounds.x + gutter_w;
        let text_y = bounds.y;
        let font_kind = if self.mode.is_code() {
            FontKind::Mono
        } else {
            FontKind::Ui
        };

        scene.clip(bounds.into());

        if gutter_w > 0.0 {
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: bounds.x,
                    y: bounds.y,
                    width: gutter_w,
                    height: bounds.height,
                },
                color: theme.colors.editor_surface,
            });
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: text_x - 1.0,
                    y: bounds.y,
                    width: 1.0,
                    height: bounds.height,
                },
                color: theme.colors.border_soft,
            });
            let gutter_digits =
                gutter_digits(self.line_tops.last().map(|(line, _)| *line).unwrap_or(0));
            for (line_no, y) in &self.line_tops {
                let painted_y = text_y - self.scroll_y + *y;
                if painted_y + line_height < bounds.y || painted_y > bounds.bottom() {
                    continue;
                }
                scene.text(TextPrimitive {
                    rect: Rect {
                        x: bounds.x,
                        y: painted_y,
                        width: (gutter_w - 8.0).max(1.0),
                        height: line_height,
                    },
                    text: format!("{line_no:>gutter_digits$}").into(),
                    color: theme.colors.gutter_text,
                    font_size,
                    font_kind: FontKind::Mono,
                    font_weight: FontWeight::Normal,
                });
            }
        }

        if self.focused && !self.is_empty {
            let sel_color = theme.colors.accent.with_alpha(Alpha::SOFT);
            for rect in &self.selection_rects {
                scene.rounded_rect(RoundedRectPrimitive::uniform(
                    Rect {
                        x: text_x + rect.x,
                        y: text_y - self.scroll_y + rect.y,
                        width: rect.w.min(text_area_w),
                        height: rect.h,
                    },
                    2.0,
                    sel_color,
                ));
            }
        }

        if self.is_empty {
            let placeholder_color = theme.colors.text_muted.with_alpha(Alpha::PLACEHOLDER);
            let content_h = line_height;
            scene.text(crate::render::scene::TextPrimitive {
                rect: Rect {
                    x: text_x,
                    y: text_y,
                    width: text_area_w,
                    height: content_h,
                },
                text: std::mem::take(&mut self.placeholder).into(),
                color: placeholder_color,
                font_size,
                font_kind,
                font_weight: FontWeight::Normal,
            });
        } else {
            let content_h = self.content_height.max(line_height);
            scene.rich_text(RichTextPrimitive {
                rect: Rect {
                    x: text_x,
                    y: text_y - self.scroll_y,
                    width: text_area_w,
                    height: content_h,
                },
                spans: build_editor_spans(
                    self.text.as_ref(),
                    &self.syntax_spans,
                    self.text_color,
                    theme,
                ),
                default_color: self.text_color,
                font_size,
                font_kind,
                font_weight: FontWeight::Normal,
            });
        }

        if self.focused {
            if let Some(ref cur) = self.cursor {
                let elapsed = cx.clock_ms.saturating_sub(cur.moved_at_ms);
                let visible = elapsed < 530 || (elapsed / 530) % 2 == 0;
                if visible {
                    scene.rounded_rect(RoundedRectPrimitive::uniform(
                        Rect {
                            x: text_x + cur.x,
                            y: text_y - self.scroll_y + cur.y + 1.0,
                            width: Sz::CURSOR_WIDTH,
                            height: line_height - Sz::CURSOR_WIDTH,
                        },
                        1.0,
                        theme.colors.text,
                    ));
                }
            }
        }

        scene.pop_clip();

        let target = self.focus_target;
        cx.accessibility.push(
            AccessibilityNode::new(
                format!("text-editor:{target:?}"),
                accesskit::Role::MultilineTextInput,
                bounds.into(),
            )
            .label(accessibility_label)
            .action(AccessibilityAction::Focus(target)),
        );
        cx.text_input_hit_areas.push(TextInputHitArea {
            bounds: bounds.into(),
            text_x,
            text_y,
            text_width: text_area_w,
            text_height: bounds.height,
            value: String::new(),
            font_size,
            focus_target: target,
            multiline: true,
        });
    }
}

impl IntoAnyElement for TextEditorElement {
    fn into_any(self) -> AnyElement {
        AnyElement::new(self)
    }
}

fn code_gutter_width(font_size: f32, max_line: usize) -> f32 {
    let digits = gutter_digits(max_line);
    let char_w = (font_size * 0.62).max(1.0);
    (digits as f32 * char_w + 18.0).ceil()
}

fn gutter_digits(max_line: usize) -> usize {
    max_line.max(1).ilog10() as usize + 1
}

fn build_editor_spans(
    text: &str,
    syntax_spans: &[DiffTokenSpan],
    default_color: crate::ui::theme::Color,
    theme: &crate::ui::theme::Theme,
) -> Arc<[RichTextSpan]> {
    if text.is_empty() {
        return Arc::from(Vec::new());
    }
    if syntax_spans.is_empty() {
        return Arc::from(vec![RichTextSpan {
            text: Arc::from(text),
            color: default_color,
            font_weight: None,
            font_style: None,
        }]);
    }

    let mut out = Vec::new();
    let mut cursor = 0_usize;
    for span in syntax_spans {
        let raw_start = span.offset as usize;
        let raw_end = raw_start
            .saturating_add(span.length as usize)
            .min(text.len());
        let Some((start, end)) = valid_text_range(text, raw_start, raw_end) else {
            continue;
        };
        if cursor < start {
            push_editor_span(&mut out, &text[cursor..start], default_color, None, None);
        }
        let (color, weight, style) = syntax_style(span.kind, default_color, theme);
        push_editor_span(&mut out, &text[start..end], color, weight, style);
        cursor = cursor.max(end);
    }
    if cursor < text.len() {
        push_editor_span(&mut out, &text[cursor..], default_color, None, None);
    }

    if out.is_empty() {
        out.push(RichTextSpan {
            text: Arc::from(text),
            color: default_color,
            font_weight: None,
            font_style: None,
        });
    }
    Arc::from(out)
}

fn valid_text_range(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    if start >= end || end > text.len() {
        return None;
    }
    if text.is_char_boundary(start) && text.is_char_boundary(end) {
        Some((start, end))
    } else {
        None
    }
}

fn push_editor_span(
    out: &mut Vec<RichTextSpan>,
    text: &str,
    color: crate::ui::theme::Color,
    font_weight: Option<FontWeight>,
    font_style: Option<FontStyle>,
) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.color == color
        && last.font_weight == font_weight
        && last.font_style == font_style
    {
        let mut merged = String::with_capacity(last.text.len() + text.len());
        merged.push_str(last.text.as_ref());
        merged.push_str(text);
        last.text = Arc::from(merged);
        return;
    }
    out.push(RichTextSpan {
        text: Arc::from(text),
        color,
        font_weight,
        font_style,
    });
}

fn syntax_style(
    syntax_kind: SyntaxTokenKind,
    default_color: crate::ui::theme::Color,
    theme: &crate::ui::theme::Theme,
) -> (
    crate::ui::theme::Color,
    Option<FontWeight>,
    Option<FontStyle>,
) {
    use SyntaxTokenKind::*;
    let color = match syntax_kind {
        Keyword | Builtin => theme.colors.syntax_keyword,
        String => theme.colors.syntax_string,
        Comment | Label | Preprocessor => theme.colors.syntax_comment,
        Function => theme.colors.syntax_function,
        Number | Constant => theme.colors.syntax_number,
        Type | Namespace | Tag => theme.colors.syntax_type,
        Attribute | Property => theme.colors.syntax_property,
        Operator | Punctuation => theme.colors.syntax_operator,
        Variable | Normal => default_color,
    };
    let (font_weight, font_style) = match syntax_kind {
        Comment => (None, Some(FontStyle::Italic)),
        Keyword | Builtin => (Some(FontWeight::Semibold), None),
        Type | Function | Constant | Attribute | Tag | Property | Namespace | Label
        | Preprocessor => (Some(FontWeight::Medium), None),
        Normal | String | Number | Operator | Punctuation | Variable => (None, None),
    };
    (color, font_weight, font_style)
}
