//! Software rasterizer for capturing Scene to PNG — no GPU required.
//!
//! Uses tiny-skia for 2D rendering and fontdue for text. Produces pixel-perfect
//! layout/color output for visual debugging and design iteration.

use crate::render::scene::{FontKind, FontWeight, Primitive, Rect, Scene};
use crate::ui::theme::Color;

struct CaptureFonts {
    ui_regular: fontdue::Font,
    ui_medium: fontdue::Font,
    ui_semibold: fontdue::Font,
    ui_bold: fontdue::Font,
    mono_regular: fontdue::Font,
    mono_medium: fontdue::Font,
    mono_semibold: fontdue::Font,
    mono_bold: fontdue::Font,
}

impl CaptureFonts {
    fn new() -> Self {
        Self {
            ui_regular: load_vendored_font(crate::fonts::UI_REGULAR_OTF),
            ui_medium: load_vendored_font(crate::fonts::UI_MEDIUM_OTF),
            ui_semibold: load_vendored_font(crate::fonts::UI_SEMIBOLD_OTF),
            ui_bold: load_vendored_font(crate::fonts::UI_BOLD_OTF),
            mono_regular: load_vendored_font(crate::fonts::MONO_REGULAR_OTF),
            mono_medium: load_vendored_font(crate::fonts::MONO_MEDIUM_OTF),
            mono_semibold: load_vendored_font(crate::fonts::MONO_SEMIBOLD_OTF),
            mono_bold: load_vendored_font(crate::fonts::MONO_BOLD_OTF),
        }
    }

    fn for_style(&self, font_kind: FontKind, font_weight: FontWeight) -> &fontdue::Font {
        match (font_kind, font_weight) {
            (FontKind::Ui, FontWeight::Normal) => &self.ui_regular,
            (FontKind::Ui, FontWeight::Medium) => &self.ui_medium,
            (FontKind::Ui, FontWeight::Semibold) => &self.ui_semibold,
            (FontKind::Ui, FontWeight::Bold) => &self.ui_bold,
            (FontKind::Mono, FontWeight::Normal) => &self.mono_regular,
            (FontKind::Mono, FontWeight::Medium) => &self.mono_medium,
            (FontKind::Mono, FontWeight::Semibold) => &self.mono_semibold,
            (FontKind::Mono, FontWeight::Bold) => &self.mono_bold,
        }
    }
}

/// Render a Scene to RGBA pixel data at the given dimensions.
pub fn scene_to_rgba(scene: &Scene, width: u32, height: u32) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("failed to create pixmap");
    let fonts = CaptureFonts::new();

    // Fill with black background.
    pixmap.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));

    for prim in &scene.primitives {
        match prim {
            Primitive::Rect(r) => {
                fill_rect(&mut pixmap, r.rect, r.color, 0.0, None);
            }
            Primitive::RoundedRect(r) => {
                fill_rect(&mut pixmap, r.rect, r.color, r.corner_radii[0], None);
            }
            Primitive::Border(b) => {
                stroke_rect(
                    &mut pixmap,
                    b.rect,
                    b.color,
                    b.widths[0].max(1.0),
                    b.corner_radii[0],
                    None,
                );
            }
            Primitive::Shadow(s) => {
                // Approximate shadow as a blurred offset rect.
                let shadow_rect = Rect {
                    x: s.rect.x + s.offset[0],
                    y: s.rect.y + s.offset[1],
                    width: s.rect.width,
                    height: s.rect.height,
                };
                let expanded = Rect {
                    x: shadow_rect.x - s.blur_radius,
                    y: shadow_rect.y - s.blur_radius,
                    width: shadow_rect.width + s.blur_radius * 2.0,
                    height: shadow_rect.height + s.blur_radius * 2.0,
                };
                fill_rect(
                    &mut pixmap,
                    expanded,
                    Color::rgba(s.color.r, s.color.g, s.color.b, s.color.a / 3),
                    s.corner_radius + s.blur_radius * 0.5,
                    None,
                );
            }
            Primitive::TextRun(t) => {
                // Detect text crushed into bounds too small for its content.
                // This catches flex-shrink layout bugs that glyphon renders as
                // garbled glyphs but fontdue silently truncates.
                let char_w = if t.font_kind == FontKind::Mono {
                    t.font_size * 0.6
                } else {
                    t.font_size * 0.55
                };
                let natural_width = t.text.len() as f32 * char_w;
                let is_crushed =
                    !t.text.is_empty() && t.rect.width > 0.0 && t.rect.width < natural_width * 0.5;

                if is_crushed {
                    // Paint a red underline to flag the layout bug visually.
                    let indicator = Rect {
                        x: t.rect.x,
                        y: t.rect.y + t.rect.height - 2.0,
                        width: t.rect.width.max(natural_width),
                        height: 2.0,
                    };
                    fill_rect(
                        &mut pixmap,
                        indicator,
                        Color::rgba(255, 60, 60, 200),
                        1.0,
                        None,
                    );
                }

                draw_text(
                    &mut pixmap,
                    fonts.for_style(t.font_kind, t.font_weight),
                    t.text.as_ref(),
                    t.rect,
                    t.color,
                    t.font_size,
                    None,
                );
            }
            Primitive::RichTextRun(t) => {
                // Render rich text spans sequentially.
                let font = fonts.for_style(t.font_kind, t.font_weight);
                let mut x_offset = 0.0;
                for span in &t.spans {
                    let span_rect = Rect {
                        x: t.rect.x + x_offset,
                        y: t.rect.y,
                        width: t.rect.width - x_offset,
                        height: t.rect.height,
                    };
                    draw_text(
                        &mut pixmap,
                        font,
                        span.text.as_ref(),
                        span_rect,
                        span.color,
                        t.font_size,
                        None,
                    );
                    x_offset += span.text.len() as f32 * t.font_size * 0.55;
                }
            }
            Primitive::Image(img) => {
                let pw = pixmap.width();
                let ph = pixmap.height();
                blit_rgba(
                    pixmap.data_mut(),
                    pw,
                    ph,
                    &img.rgba,
                    img.width,
                    img.height,
                    img.rect,
                );
            }
            Primitive::EffectQuad(e) => {
                // Approximate: gradient from color_a to color_b.
                fill_rect(&mut pixmap, e.rect, e.color_a, e.corner_radius, None);
            }
            Primitive::BlurRegion(_) => {
                // Can't software-blur; skip.
            }
            Primitive::Icon(_) => {}
            Primitive::ClipStart(_) | Primitive::ClipEnd => {
                // Clip masking omitted in software capture.
            }
            Primitive::ZIndexPush(_) | Primitive::ZIndexPop => {
                // Z-ordering is visual only; software rasterizer paints in order.
            }
            Primitive::LayerBoundary => {}
        }
    }

    pixmap.data().to_vec()
}

/// Render a Scene to a PNG file.
pub fn scene_to_png(scene: &Scene, width: u32, height: u32, path: &std::path::Path) {
    let rgba = scene_to_rgba(scene, width, height);

    let file = std::fs::File::create(path).expect("failed to create PNG file");
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("failed to write PNG header");
    writer
        .write_image_data(&rgba)
        .expect("failed to write PNG data");
}

// ---------------------------------------------------------------------------
// tiny-skia drawing helpers
// ---------------------------------------------------------------------------

fn to_skia_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn rounded_rect_path(rect: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.min(rect.width * 0.5).min(rect.height * 0.5);
    let x = rect.x;
    let y = rect.y;
    let w = rect.width;
    let h = rect.height;

    let mut pb = tiny_skia::PathBuilder::new();
    // Top-left corner
    pb.move_to(x + r, y);
    // Top edge → top-right corner
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    // Right edge → bottom-right corner
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    // Bottom edge → bottom-left corner
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    // Left edge → top-left corner
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

fn fill_rect(
    pixmap: &mut tiny_skia::Pixmap,
    rect: Rect,
    color: Color,
    radius: f32,
    _clip: Option<&()>,
) {
    if color.a == 0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }

    let paint = tiny_skia::Paint {
        shader: tiny_skia::Shader::SolidColor(to_skia_color(color)),
        anti_alias: true,
        blend_mode: tiny_skia::BlendMode::SourceOver,
        ..Default::default()
    };

    if radius > 0.5 {
        if let Some(path) = rounded_rect_path(rect, radius) {
            pixmap.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
    } else {
        let skia_rect = tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
        if let Some(skia_rect) = skia_rect {
            pixmap.fill_rect(skia_rect, &paint, tiny_skia::Transform::identity(), None);
        }
    }
}

fn stroke_rect(
    pixmap: &mut tiny_skia::Pixmap,
    rect: Rect,
    color: Color,
    width: f32,
    radius: f32,
    _clip: Option<&()>,
) {
    if color.a == 0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }

    let paint = tiny_skia::Paint {
        shader: tiny_skia::Shader::SolidColor(to_skia_color(color)),
        anti_alias: true,
        ..Default::default()
    };

    let inset = width * 0.5;
    let r = Rect {
        x: rect.x + inset,
        y: rect.y + inset,
        width: (rect.width - width).max(0.0),
        height: (rect.height - width).max(0.0),
    };

    let path = if radius > 0.5 {
        rounded_rect_path(r, radius)
    } else {
        let mut pb = tiny_skia::PathBuilder::new();
        if let Some(skia_rect) = tiny_skia::Rect::from_xywh(r.x, r.y, r.width, r.height) {
            pb.push_rect(skia_rect);
        }
        pb.finish()
    };

    if let Some(path) = path {
        let stroke = tiny_skia::Stroke {
            width,
            ..Default::default()
        };
        pixmap.stroke_path(
            &path,
            &paint,
            &stroke,
            tiny_skia::Transform::identity(),
            None,
        );
    }
}

fn draw_text(
    pixmap: &mut tiny_skia::Pixmap,
    font: &fontdue::Font,
    text: &str,
    rect: Rect,
    color: Color,
    font_size: f32,
    _clip: Option<&()>,
) {
    if color.a == 0 || text.is_empty() || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }

    let px_size = font_size.max(6.0);
    let baseline_y = rect.y + (rect.height + px_size * 0.7) * 0.5;
    let mut x = rect.x;
    let max_x = rect.x + rect.width;

    for ch in text.chars() {
        if x >= max_x {
            break;
        }
        let (metrics, bitmap) = font.rasterize(ch, px_size);
        if metrics.width == 0 || metrics.height == 0 {
            x += metrics.advance_width;
            continue;
        }

        let glyph_x = x + metrics.xmin as f32;
        let glyph_y = baseline_y - metrics.height as f32 - metrics.ymin as f32;

        for gy in 0..metrics.height {
            for gx in 0..metrics.width {
                let coverage = bitmap[gy * metrics.width + gx];
                if coverage == 0 {
                    continue;
                }
                let px = (glyph_x + gx as f32) as i32;
                let py = (glyph_y + gy as f32) as i32;
                if px < 0 || py < 0 || px >= pixmap.width() as i32 || py >= pixmap.height() as i32 {
                    continue;
                }

                let alpha = (coverage as u16 * color.a as u16 / 255) as u8;
                if alpha == 0 {
                    continue;
                }

                // Alpha-blend onto the pixmap.
                let idx = (py as u32 * pixmap.width() + px as u32) as usize * 4;
                let data = pixmap.data_mut();
                if idx + 3 < data.len() {
                    let a = alpha as f32 / 255.0;
                    let inv_a = 1.0 - a;
                    data[idx] = (color.r as f32 * a + data[idx] as f32 * inv_a) as u8;
                    data[idx + 1] = (color.g as f32 * a + data[idx + 1] as f32 * inv_a) as u8;
                    data[idx + 2] = (color.b as f32 * a + data[idx + 2] as f32 * inv_a) as u8;
                    data[idx + 3] = (alpha.max(data[idx + 3])) as u8;
                }
            }
        }

        x += metrics.advance_width;
    }
}

fn blit_rgba(
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
    rect: Rect,
) {
    if src.is_empty() || src_w == 0 || src_h == 0 {
        return;
    }
    let scale_x = src_w as f32 / rect.width;
    let scale_y = src_h as f32 / rect.height;

    let x0 = rect.x.max(0.0) as u32;
    let y0 = rect.y.max(0.0) as u32;
    let x1 = (rect.x + rect.width).min(dst_w as f32) as u32;
    let y1 = (rect.y + rect.height).min(dst_h as f32) as u32;

    for dy in y0..y1 {
        for dx in x0..x1 {
            let sx = ((dx as f32 - rect.x) * scale_x) as u32;
            let sy = ((dy as f32 - rect.y) * scale_y) as u32;
            if sx >= src_w || sy >= src_h {
                continue;
            }
            let si = (sy * src_w + sx) as usize * 4;
            let di = (dy * dst_w + dx) as usize * 4;
            if si + 3 >= src.len() || di + 3 >= dst.len() {
                continue;
            }
            let sa = src[si + 3] as f32 / 255.0;
            if sa < 0.004 {
                continue;
            }
            let inv = 1.0 - sa;
            dst[di] = (src[si] as f32 * sa + dst[di] as f32 * inv) as u8;
            dst[di + 1] = (src[si + 1] as f32 * sa + dst[di + 1] as f32 * inv) as u8;
            dst[di + 2] = (src[si + 2] as f32 * sa + dst[di + 2] as f32 * inv) as u8;
            dst[di + 3] = (src[si + 3]).max(dst[di + 3]);
        }
    }
}

fn load_vendored_font(bytes: &[u8]) -> fontdue::Font {
    fontdue::Font::from_bytes(bytes.to_vec(), fontdue::FontSettings::default())
        .expect("failed to load vendored capture font")
}
