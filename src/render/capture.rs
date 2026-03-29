//! Software rasterizer for capturing Scene to PNG — no GPU required.
//!
//! Uses tiny-skia for 2D rendering and fontdue for text. Produces pixel-perfect
//! layout/color output for visual debugging and design iteration.

use crate::render::scene::{FontKind, Primitive, Rect, Scene};
use crate::ui::theme::Color;

/// Render a Scene to RGBA pixel data at the given dimensions.
pub fn scene_to_rgba(scene: &Scene, width: u32, height: u32) -> Vec<u8> {
    let mut pixmap =
        tiny_skia::Pixmap::new(width, height).expect("failed to create pixmap");

    // Fill with black background.
    pixmap.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));

    let clip_stack: Vec<()> = Vec::new(); // Clip masking simplified for capture.
    // Try to load system fonts for text rendering.
    let font = load_system_font("segoeui.ttf")
        .or_else(|| load_system_font("arial.ttf"))
        .or_else(|| load_system_font("DejaVuSans.ttf"));
    let mono_font = load_system_font("consola.ttf")
        .or_else(|| load_system_font("CascadiaMono.ttf"))
        .or_else(|| load_system_font("DejaVuSansMono.ttf"));

    for prim in &scene.primitives {
        match prim {
            Primitive::Rect(r) => {
                fill_rect(&mut pixmap, r.rect, r.color, 0.0, None);
            }
            Primitive::RoundedRect(r) => {
                fill_rect(
                    &mut pixmap,
                    r.rect,
                    r.color,
                    r.corner_radii[0],
                    None,
                );
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
                let f = match t.font_kind {
                    FontKind::Mono => mono_font.as_ref(),
                    FontKind::Ui => font.as_ref(),
                };
                if let Some(font) = f {
                    draw_text(
                        &mut pixmap,
                        font,
                        &t.text,
                        t.rect,
                        t.color,
                        t.font_size,
                        None,
                    );
                } else {
                    // Fallback: colored rectangle for text bounds.
                    let text_width = t.text.len() as f32 * t.font_size * 0.55;
                    let text_rect = Rect {
                        x: t.rect.x,
                        y: t.rect.y + t.rect.height * 0.3,
                        width: text_width.min(t.rect.width),
                        height: t.rect.height * 0.5,
                    };
                    fill_rect(&mut pixmap, text_rect, t.color.with_alpha(100), 2.0, None);
                }
            }
            Primitive::RichTextRun(t) => {
                // Render rich text spans sequentially.
                let f = match t.font_kind {
                    FontKind::Mono => mono_font.as_ref(),
                    FontKind::Ui => font.as_ref(),
                };
                if let Some(font_ref) = f {
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
                            font_ref,
                            &span.text,
                            span_rect,
                            span.color,
                            t.font_size,
                            None,
                        );
                        x_offset += span.text.len() as f32 * t.font_size * 0.55;
                    }
                }
            }
            Primitive::EffectQuad(e) => {
                // Approximate: gradient from color_a to color_b.
                fill_rect(
                    &mut pixmap,
                    e.rect,
                    e.color_a,
                    e.corner_radius,
                    None,
                );
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
    writer.write_image_data(&rgba).expect("failed to write PNG data");
}

// ---------------------------------------------------------------------------
// tiny-skia drawing helpers
// ---------------------------------------------------------------------------

fn to_skia_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
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
        let r = radius.min(rect.width * 0.5).min(rect.height * 0.5);
        let path = {
            let mut pb = tiny_skia::PathBuilder::new();
            let skia_rect = tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.width, rect.height);
            if let Some(skia_rect) = skia_rect {
                // Approximate rounded rect with a regular rect + round corners
                pb.push_rect(skia_rect);
            }
            pb.finish()
        };
        if let Some(path) = path {
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

    let path = {
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
                if px < 0 || py < 0 || px >= pixmap.width() as i32 || py >= pixmap.height() as i32
                {
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

fn load_system_font(name: &str) -> Option<fontdue::Font> {
    let candidates = [
        format!("C:\\Windows\\Fonts\\{name}"),
        format!("/usr/share/fonts/truetype/dejavu/{name}"),
        format!("/System/Library/Fonts/{name}"),
    ];
    for path in &candidates {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(f) = fontdue::Font::from_bytes(data, fontdue::FontSettings::default()) {
                return Some(f);
            }
        }
    }
    None
}

