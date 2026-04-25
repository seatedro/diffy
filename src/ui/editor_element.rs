use crate::editor::SelectionRect;
use crate::render::scene::{EditorTextSlot, Rect};
use crate::render::{RoundedRectPrimitive, Scene};
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
    focus_target: FocusTarget,
    editor_id: u8,
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
        focus_target: FocusTarget::FileList,
        editor_id: 0,
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

    pub fn focus_target(mut self, target: FocusTarget) -> Self {
        self.focus_target = target;
        self
    }

    pub fn editor_id(mut self, id: u8) -> Self {
        self.editor_id = id;
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
        let font_size = self.font_size;
        let line_height = font_size * 1.35;
        let text_area_w = bounds.width.max(0.0);
        let text_x = bounds.x;
        let text_y = bounds.y;

        scene.clip(bounds.into());

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
                font_kind: crate::render::scene::FontKind::Ui,
                font_weight: crate::render::scene::FontWeight::Normal,
            });
        } else {
            let content_h = self.content_height.max(line_height);
            scene.editor_text(EditorTextSlot {
                rect: Rect {
                    x: text_x,
                    y: text_y,
                    width: text_area_w,
                    height: content_h,
                },
                color: self.text_color,
                font_size,
                scroll_y: self.scroll_y,
                editor_id: self.editor_id,
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
