use std::sync::{Arc, OnceLock};

use quark_ui::element::{AnyElement, IntoAnyElement, raster_image};

#[cfg(target_os = "macos")]
const LOGO_PNG: &[u8] = include_bytes!("../../assets/brand/diffy-logo-macos.png");
#[cfg(not(target_os = "macos"))]
const LOGO_PNG: &[u8] = include_bytes!("../../assets/brand/diffy-logo.png");

struct Logo {
    rgba: Arc<[u8]>,
    width: u32,
    height: u32,
}

fn decoded() -> Option<&'static Logo> {
    static LOGO: OnceLock<Option<Logo>> = OnceLock::new();
    LOGO.get_or_init(|| {
        let image = image::load_from_memory(LOGO_PNG).ok()?.to_rgba8();
        let (width, height) = image.dimensions();
        Some(Logo {
            rgba: Arc::from(image.into_raw()),
            width,
            height,
        })
    })
    .as_ref()
}

pub fn logo(size: f32) -> Option<AnyElement> {
    let logo = decoded()?;
    let key = quark::stable_hash("diffy.logo");
    Some(raster_image(logo.rgba.clone(), logo.width, logo.height, key, size).into_any())
}
