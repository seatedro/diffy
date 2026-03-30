use crate::ui::element::{div, text, AnyElement, ElementContext, IntoAnyElement, RenderOnce};
use crate::ui::style::Styled;
use crate::ui::theme::Color;

pub struct Avatar {
    name: String,
    size: f32,
    bg_color: Option<Color>,
}

pub fn avatar(name: impl Into<String>) -> Avatar {
    Avatar {
        name: name.into(),
        size: 32.0,
        bg_color: None,
    }
}

impl Avatar {
    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }

    pub fn bg(mut self, c: Color) -> Self {
        self.bg_color = Some(c);
        self
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

const PALETTE: [Color; 8] = [
    Color::rgba(99, 102, 241, 255),
    Color::rgba(168, 85, 247, 255),
    Color::rgba(236, 72, 153, 255),
    Color::rgba(239, 68, 68, 255),
    Color::rgba(249, 115, 22, 255),
    Color::rgba(234, 179, 8, 255),
    Color::rgba(34, 197, 94, 255),
    Color::rgba(6, 182, 212, 255),
];

fn name_to_color(name: &str) -> Color {
    let hash = name
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    PALETTE[(hash % PALETTE.len() as u32) as usize]
}

impl RenderOnce for Avatar {
    fn render(self, _cx: &ElementContext) -> AnyElement {
        let bg = self.bg_color.unwrap_or_else(|| name_to_color(&self.name));
        let inits = initials(&self.name);
        let font_size = (self.size * 0.4).round();

        div()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .w(self.size)
            .h(self.size)
            .bg(bg)
            .rounded(self.size / 2.0)
            .child(
                text(inits)
                    .size(font_size)
                    .color(Color::rgba(255, 255, 255, 255))
                    .bold()
                    .text_center(),
            )
            .into_any()
    }
}

pub struct AvatarGroup {
    names: Vec<String>,
    size: f32,
    max_show: usize,
}

pub fn avatar_group(names: Vec<impl Into<String>>) -> AvatarGroup {
    AvatarGroup {
        names: names.into_iter().map(|n| n.into()).collect(),
        size: 28.0,
        max_show: 4,
    }
}

impl AvatarGroup {
    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }

    pub fn max_show(mut self, n: usize) -> Self {
        self.max_show = n;
        self
    }
}

impl RenderOnce for AvatarGroup {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let tc = &cx.theme.colors;
        let overlap = -(self.size * 0.25).round();
        let shown = self.names.len().min(self.max_show);
        let remaining = self.names.len().saturating_sub(self.max_show);

        let mut row = div().flex_row().items_center();

        for (i, name) in self.names.into_iter().take(shown).enumerate() {
            let a = avatar(name).size(self.size);
            let mut wrapper = div()
                .flex_shrink_0()
                .border(tc.background)
                .rounded(self.size / 2.0)
                .child(a);
            if i > 0 {
                wrapper.element_style_mut().layout.margin.left =
                    taffy::LengthPercentageAuto::length(overlap);
            }
            row = row.child(wrapper);
        }

        if remaining > 0 {
            let count_size = self.size;
            let font_size = (count_size * 0.35).round();
            let mut count = div()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .w(count_size)
                .h(count_size)
                .bg(tc.element_background)
                .border(tc.background)
                .rounded(count_size / 2.0)
                .child(
                    text(format!("+{remaining}"))
                        .size(font_size)
                        .color(tc.text_muted)
                        .medium(),
                );
            count.element_style_mut().layout.margin.left =
                taffy::LengthPercentageAuto::length(overlap);
            row = row.child(count);
        }

        row.into_any()
    }
}
