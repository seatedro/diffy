use std::time::{Duration, Instant};

use quark::FontKind;
use quark::view;
use quark_app::ViewContext;
use quark_components::{HoverCardState, Side as CardSide};
use quark_text::{TextParams, TextStyle as GlyphStyle};
use quark_ui::element::*;
use quark_ui::style::Styled;

use crate::app::tokens::{self, Sp, TextStyle};
use crate::contracts::viewport::symbols::Description;

pub struct HoverController {
    epoch: Instant,
    pub pointer: Option<(f32, f32)>,
    pub primary: bool,
    pub(super) card: HoverCardState,
    pub(super) key: Option<u64>,
    pub(super) generation: u64,
    described: Option<(u64, Description)>,
}

impl Default for HoverController {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            pointer: None,
            primary: false,
            card: HoverCardState::new((0.0, 0.0), CardSide::Top),
            key: None,
            generation: 0,
            described: None,
        }
    }
}

impl std::fmt::Debug for HoverController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HoverController")
            .field("key", &self.key)
            .field("shown", &self.card.shown())
            .finish_non_exhaustive()
    }
}

impl HoverController {
    pub fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    pub fn described(&mut self, generation: u64, description: Option<Description>) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(key) = self.key else {
            return false;
        };
        self.described = description.map(|description| (key, description));
        self.card.shown() == Some(key)
    }

    pub fn dismiss(&mut self) -> bool {
        let shown = self.card.shown().is_some();
        self.card.hide();
        self.key = None;
        self.generation += 1;
        shown
    }

    pub fn card(&mut self, viewport: (f32, f32), cx: &mut ViewContext) -> Option<AnyElement> {
        let now = self.now_ms();
        self.card.tick(now);
        if let Some(wake) = self.card.next_wake_ms() {
            cx.frame
                .request_frame_in(Duration::from_millis(wake.saturating_sub(now)));
        }
        let key = self.card.shown()?;
        let (_, description) = self.described.as_ref().filter(|(k, _)| *k == key)?;
        let theme = cx.theme;
        let tc = &theme.colors;
        let scale = tokens::ui_scale(theme);
        let mono = TextStyle::Mono.font_size(theme);
        let small = TextStyle::BodySmall.font_size(theme);
        let gap = tokens::scaled(Sp::XS, scale);
        let pad = theme.metrics.spacing_md;
        let measure = |cx: &mut ViewContext, text: &str, size: f32, kind: FontKind| {
            let style = GlyphStyle::new(size).kind(kind);
            cx.frame
                .layout_text(&TextParams::new(text, style))
                .map_or((0.0, size), |layout| layout.size())
        };
        let footer = match description.more {
            0 => format!("{}:{}", description.path, description.line + 1),
            more => format!(
                "{}:{}  +{more} more",
                description.path,
                description.line + 1
            ),
        };
        let mut width: f32 = 0.0;
        let mut height = 0.0;
        for line in &description.signature {
            let (w, h) = measure(cx, line, mono, FontKind::Mono);
            width = width.max(w);
            height += h;
        }
        for line in &description.doc {
            let (w, h) = measure(cx, line, small, FontKind::Ui);
            width = width.max(w);
            height += h;
        }
        let (w, h) = measure(cx, &footer, small, FontKind::Ui);
        width = width.max(w);
        height += h;
        let sections = 2 + usize::from(!description.doc.is_empty());
        height += gap * sections as f32;
        let limit = (viewport.0 - pad * 4.0).max(pad);
        let slack = mono;
        self.card.size = (
            (width + slack + pad * 2.0).ceil().min(limit),
            (height + pad * 2.0).ceil(),
        );
        let signature: Vec<AnyElement> = description
            .signature
            .iter()
            .map(|line| {
                view! {
                    <text size={mono} class="font-mono truncate" color={tc.text}>{line.clone()}</text>
                }
            })
            .collect();
        let doc: Vec<AnyElement> = description
            .doc
            .iter()
            .map(|line| view! { <text size={small} class="truncate" color={tc.text_muted}>{line.clone()}</text> })
            .collect();
        let content = view! {
            <div class="flex-col" gap={gap} test_id="definition.hover">
                <div class="flex-col">{...signature}</div>
                if !doc.is_empty() {
                    <div class="flex-col">{...doc}</div>
                }
                <text size={small} class="truncate" color={tc.text_muted}>{footer}</text>
            </div>
        };
        self.card.render(content, viewport, theme)
    }
}
