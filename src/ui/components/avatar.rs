use std::sync::Arc;

use halogen::view;

use crate::ui::element::{
    AnyElement, ElementContext, IntoAnyElement, RenderOnce, div, raster_image, text,
};
use crate::ui::style::Styled;
use crate::ui::theme::Color;

/// RGBA bitmap input for an avatar. Already circular-masked on the CPU side.
#[derive(Debug, Clone)]
pub struct AvatarImage {
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub cache_key: u64,
}

pub struct Avatar {
    name: String,
    size: f32,
    bg_color: Option<Color>,
    image: Option<AvatarImage>,
}

pub fn avatar(name: impl Into<String>) -> Avatar {
    Avatar {
        name: name.into(),
        size: 32.0,
        bg_color: None,
        image: None,
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

    pub fn image(mut self, image: Option<AvatarImage>) -> Self {
        self.image = image;
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
    fn render(self, cx: &ElementContext) -> AnyElement {
        // `.size()` is a BASE (logical) value. The image path scales it by `ui_scale`
        // inside `raster_image` (like `SvgIcon`), so the initials path must scale too —
        // otherwise the two paths render at different physical sizes for the same
        // `.size()`, and a pre-scaled caller double-scales the image.
        let scale = cx.theme.metrics.ui_scale();
        if let Some(img) = self.image {
            return raster_image(img.rgba, img.width, img.height, img.cache_key, self.size)
                .into_any();
        }

        let px = self.size * scale;
        let bg = self.bg_color.unwrap_or_else(|| name_to_color(&self.name));
        let inits = initials(&self.name);
        let font_size = (px * 0.4).round();

        view! {
            <div class="shrink-0 items-center justify-center"
                 w={px} h={px}
                 bg={bg} rounded={px / 2.0}>
                <text class="bold text-center" size={font_size}
                      color={Color::rgba(255, 255, 255, 255)}>{inits}</text>
            </div>
        }
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
        // Match `Avatar`: `.size()` is base; scale the ring/overlap/count chip to the
        // physical avatar size so the group stays consistent with its members.
        let scale = cx.theme.metrics.ui_scale();
        let px = self.size * scale;
        let overlap = -(px * 0.25).round();
        let shown = self.names.len().min(self.max_show);
        let remaining = self.names.len().saturating_sub(self.max_show);
        let count_size = px;
        let font_size = (count_size * 0.35).round();

        view! {
            <div class="flex-row items-center">
                for (i, name) in self.names.into_iter().take(shown).enumerate() {
                    <div class="shrink-0"
                         border={tc.background}
                         rounded={px / 2.0}
                         @when {i > 0} { margin_left={overlap} }>
                        {avatar(name).size(self.size)}
                    </div>
                }
                if remaining > 0 {
                    <div class="shrink-0 items-center justify-center"
                         w={count_size} h={count_size}
                         bg={tc.element_background}
                         border={tc.background}
                         rounded={count_size / 2.0}
                         margin_left={overlap}>
                        <text color={tc.text_muted}
                              size={font_size}
                              medium>
                            {format!("+{remaining}")}
                        </text>
                    </div>
                }
            </div>
        }
    }
}
