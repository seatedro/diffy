//! Renders the fixture review-thread card to `target/ui/review_card.png` using a
//! windowless wgpu renderer. Run with:
//!
//!   cargo run --example render_fixture --features headless-render
//!
//! Requires the `headless-render` feature (it gates the no-GPU renderer ctor and
//! the PNG readback path). If no GPU/adapter is available the run fails at device
//! creation; that is expected in headless sandboxes.

#[cfg(not(feature = "headless-render"))]
fn main() {
    eprintln!("re-run with: cargo run --example render_fixture --features headless-render");
}

#[cfg(feature = "headless-render")]
fn main() {
    use diffy::fonts::FontSettings;
    use diffy::render::Renderer;
    use diffy::ui::harness::{render_review_card_sized, sample_card_selection, sample_review_thread};

    // Render at a moderate width and scale so the PNG comes back full-resolution
    // (large images get downscaled on the way back). The card scene already bakes
    // this scale into its geometry, so render_to_png rasterizes 1:1 (scale_factor 1).
    let card_scale = std::env::var("CARD_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0_f32);
    let card_width = std::env::var("CARD_WIDTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(760.0_f32);

    // CARD_SELECT=1 draws a sample selection across comment 1's code/normal boundary
    // so the highlight rendering on a styled line can be eyeballed.
    let selection = (std::env::var("CARD_SELECT").as_deref() == Ok("1"))
        .then(sample_card_selection)
        .flatten();

    let thread = sample_review_thread();
    let rendered =
        render_review_card_sized(&thread, true, selection.as_ref(), card_width, card_scale);

    // The card is drawn at the origin; give the canvas extra room on the right and
    // bottom so its drop shadow and the footer button aren't clipped at the edge.
    let margin = 56u32;
    let scale = 1.0_f32;
    let width = rendered.width.ceil() as u32 + margin;
    let height = rendered.height.ceil() as u32 + margin;

    let font_settings = FontSettings::default();
    let mut renderer = match Renderer::new_headless(width, height, scale as f64, &font_settings) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("headless renderer unavailable: {e}");
            std::process::exit(1);
        }
    };

    let path = std::path::Path::new("target/ui/review_card.png");
    match renderer.render_to_png(&rendered.scene, width, height, scale, path) {
        Ok(()) => println!("wrote {} ({width}x{height})", path.display()),
        Err(e) => {
            eprintln!("render_to_png failed: {e}");
            std::process::exit(1);
        }
    }
}
