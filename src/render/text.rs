use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use glyphon::{
    Attrs, Buffer, Color as GlyphonColor, Family, FontSystem, Metrics, Shaping, TextArea,
    TextBounds,
};

use crate::render::scene::{FontKind, FontWeight, Rect, RichTextPrimitive, TextPrimitive};
use crate::ui::theme::Color;

use super::renderer::{CachedTextBuffer, ClippedRichText, ClippedText};

pub(super) fn prepare_text_areas<'a>(
    font_system: &mut FontSystem,
    text_cache: &'a mut HashMap<u64, CachedTextBuffer>,
    text_cache_frame: &mut u64,
    texts: &[ClippedText],
    rich_texts: &[ClippedRichText],
    scale_factor: f64,
) -> Vec<TextArea<'a>> {
    *text_cache_frame = text_cache_frame.wrapping_add(1);
    let frame = *text_cache_frame;
    let mut keys = Vec::with_capacity(texts.len() + rich_texts.len());

    for text in texts {
        let key = plain_text_cache_key(&text.primitive, text.clip, scale_factor);
        if !text_cache.contains_key(&key) {
            let prepared = build_plain_text_buffer(
                font_system,
                &text.primitive,
                text.clip,
                scale_factor,
                frame,
            );
            text_cache.insert(key, prepared);
        }
        if let Some(entry) = text_cache.get_mut(&key) {
            entry.last_used_frame = frame;
        }
        keys.push(key);
    }

    for text in rich_texts {
        let key = rich_text_cache_key(&text.primitive, text.clip, scale_factor);
        if !text_cache.contains_key(&key) {
            let prepared = build_rich_text_buffer(
                font_system,
                &text.primitive,
                text.clip,
                scale_factor,
                frame,
            );
            text_cache.insert(key, prepared);
        }
        if let Some(entry) = text_cache.get_mut(&key) {
            entry.last_used_frame = frame;
        }
        keys.push(key);
    }

    if frame % 240 == 0 {
        trim_text_cache(text_cache, frame);
    }

    keys.iter()
        .filter_map(|key| text_cache.get(key).map(text_area_from_cache))
        .collect()
}

fn text_area_from_cache(prepared: &CachedTextBuffer) -> TextArea<'_> {
    TextArea {
        buffer: &prepared.buffer,
        left: prepared.left,
        top: prepared.top,
        scale: 1.0,
        bounds: TextBounds {
            left: prepared.clip.x.round() as i32,
            top: prepared.clip.y.round() as i32,
            right: prepared.clip.right().round() as i32,
            bottom: prepared.clip.bottom().round() as i32,
        },
        default_color: prepared.default_color,
        custom_glyphs: &[],
    }
}

fn build_plain_text_buffer(
    font_system: &mut FontSystem,
    primitive: &TextPrimitive,
    clip: Rect,
    scale_factor: f64,
    last_used_frame: u64,
) -> CachedTextBuffer {
    let metrics = Metrics::new(primitive.font_size, primitive.font_size * 1.35);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(
        font_system,
        Some((primitive.rect.width * scale_factor as f32).max(1.0)),
        Some((primitive.rect.height * scale_factor as f32).max(1.0)),
    );
    let attrs = attrs_for_font(primitive.font_kind, primitive.font_weight, primitive.color);
    buffer.set_text(
        font_system,
        primitive.text.as_ref(),
        &attrs,
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    CachedTextBuffer {
        buffer,
        left: primitive.rect.x,
        top: primitive.rect.y,
        clip,
        default_color: glyphon_color(primitive.color),
        last_used_frame,
    }
}

fn build_rich_text_buffer(
    font_system: &mut FontSystem,
    primitive: &RichTextPrimitive,
    clip: Rect,
    scale_factor: f64,
    last_used_frame: u64,
) -> CachedTextBuffer {
    let metrics = Metrics::new(primitive.font_size, primitive.font_size * 1.35);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(
        font_system,
        Some((primitive.rect.width * scale_factor as f32).max(1.0)),
        Some((primitive.rect.height * scale_factor as f32).max(1.0)),
    );
    let default_attrs = attrs_for_font(
        primitive.font_kind,
        primitive.font_weight,
        primitive.default_color,
    );
    let spans = primitive
        .spans
        .iter()
        .map(|span| {
            (
                span.text.as_ref(),
                attrs_for_font(primitive.font_kind, primitive.font_weight, span.color),
            )
        })
        .collect::<Vec<_>>();
    if spans.is_empty() {
        buffer.set_text(font_system, "", &default_attrs, Shaping::Advanced, None);
    } else {
        buffer.set_rich_text(
            font_system,
            spans.iter().map(|(text, attrs)| (*text, attrs.clone())),
            &default_attrs,
            Shaping::Advanced,
            None,
        );
    }
    buffer.shape_until_scroll(font_system, false);
    CachedTextBuffer {
        buffer,
        left: primitive.rect.x,
        top: primitive.rect.y,
        clip,
        default_color: glyphon_color(primitive.default_color),
        last_used_frame,
    }
}

fn trim_text_cache(cache: &mut HashMap<u64, CachedTextBuffer>, frame: u64) {
    const KEEP_UNUSED_FRAMES: u64 = 240;
    cache.retain(|_, entry| frame.saturating_sub(entry.last_used_frame) <= KEEP_UNUSED_FRAMES);
}

fn plain_text_cache_key(primitive: &TextPrimitive, clip: Rect, scale_factor: f64) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write_u8(0);
    hash_rect(&mut hasher, primitive.rect);
    hash_rect(&mut hasher, clip);
    hasher.write_u32(primitive.font_size.to_bits());
    hasher.write_u64(scale_factor.to_bits());
    hasher.write_u8(font_kind_tag(primitive.font_kind));
    hasher.write_u8(font_weight_tag(primitive.font_weight));
    hash_color(&mut hasher, primitive.color);
    primitive.text.hash(&mut hasher);
    hasher.finish()
}

fn rich_text_cache_key(primitive: &RichTextPrimitive, clip: Rect, scale_factor: f64) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write_u8(1);
    hash_rect(&mut hasher, primitive.rect);
    hash_rect(&mut hasher, clip);
    hasher.write_u32(primitive.font_size.to_bits());
    hasher.write_u64(scale_factor.to_bits());
    hasher.write_u8(font_kind_tag(primitive.font_kind));
    hasher.write_u8(font_weight_tag(primitive.font_weight));
    hash_color(&mut hasher, primitive.default_color);
    hasher.write_usize(primitive.spans.len());
    for span in primitive.spans.iter() {
        span.text.hash(&mut hasher);
        hash_color(&mut hasher, span.color);
    }
    hasher.finish()
}

fn hash_rect(hasher: &mut DefaultHasher, rect: Rect) {
    hasher.write_u32(rect.x.to_bits());
    hasher.write_u32(rect.y.to_bits());
    hasher.write_u32(rect.width.to_bits());
    hasher.write_u32(rect.height.to_bits());
}

fn hash_color(hasher: &mut DefaultHasher, color: Color) {
    hasher.write_u8(color.r);
    hasher.write_u8(color.g);
    hasher.write_u8(color.b);
    hasher.write_u8(color.a);
}

fn font_kind_tag(kind: FontKind) -> u8 {
    match kind {
        FontKind::Ui => 0,
        FontKind::Mono => 1,
    }
}

fn font_weight_tag(weight: FontWeight) -> u8 {
    match weight {
        FontWeight::Normal => 0,
        FontWeight::Medium => 1,
        FontWeight::Semibold => 2,
        FontWeight::Bold => 3,
    }
}

fn attrs_for_font(font_kind: FontKind, font_weight: FontWeight, color: Color) -> Attrs<'static> {
    let family = match font_kind {
        FontKind::Ui => Family::SansSerif,
        FontKind::Mono => Family::Monospace,
    };
    let weight = match font_weight {
        FontWeight::Normal => glyphon::Weight::NORMAL,
        FontWeight::Medium => glyphon::Weight(500),
        FontWeight::Semibold => glyphon::Weight(600),
        FontWeight::Bold => glyphon::Weight::BOLD,
    };
    Attrs::new()
        .family(family)
        .weight(weight)
        .color(glyphon_text_color(color))
}

pub(super) fn measure_mono_char_width(font_system: &mut FontSystem, font_size: f32) -> f32 {
    let metrics = Metrics::new(font_size, font_size * 1.35);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(font_size * 100.0), Some(font_size * 2.0));
    let attrs = Attrs::new().family(Family::Monospace);
    let sample = "0000000000";
    buffer.set_text(font_system, sample, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    let mut total_w = 0.0_f32;
    let mut glyph_count = 0_u32;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            total_w += glyph.w;
            glyph_count += 1;
        }
    }
    if glyph_count > 0 {
        total_w / glyph_count as f32
    } else {
        8.0
    }
}

pub(super) fn glyphon_color(color: Color) -> GlyphonColor {
    GlyphonColor::rgba(color.r, color.g, color.b, color.a)
}

fn glyphon_text_color(color: Color) -> glyphon::Color {
    glyphon::Color::rgba(color.r, color.g, color.b, color.a)
}

pub(super) fn color_to_linear(color: Color) -> [f32; 4] {
    [
        srgb_to_linear(color.r),
        srgb_to_linear(color.g),
        srgb_to_linear(color.b),
        color.a as f32 / 255.0,
    ]
}

fn srgb_to_linear(channel: u8) -> f32 {
    let value = channel as f32 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}
