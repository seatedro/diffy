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
    use diffy::ui::harness::{render_review_card, sample_review_thread};

    let thread = sample_review_thread();
    let rendered = render_review_card(&thread, true, None);

    // `render_review_card` already bakes the ui-scale into the scene geometry, so the
    // scene is in final device pixels — rasterize 1:1. Passing the ui-scale again would
    // shape/position content for a 2x-larger canvas and clip the right/bottom edges.
    let scale = 1.0_f32;
    let width = rendered.width.ceil() as u32;
    let height = rendered.height.ceil() as u32;

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
