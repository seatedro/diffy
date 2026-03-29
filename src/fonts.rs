use glyphon::{FontSystem, fontdb};

pub const UI_FAMILY: &str = "Geist";
pub const MONO_FAMILY: &str = "Geist Mono";

pub const UI_REGULAR_OTF: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.otf");
pub const UI_MEDIUM_OTF: &[u8] = include_bytes!("../assets/fonts/Geist-Medium.otf");
pub const UI_SEMIBOLD_OTF: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.otf");
pub const UI_BOLD_OTF: &[u8] = include_bytes!("../assets/fonts/Geist-Bold.otf");

pub const MONO_REGULAR_OTF: &[u8] = include_bytes!("../assets/fonts/GeistMono-Regular.otf");
pub const MONO_MEDIUM_OTF: &[u8] = include_bytes!("../assets/fonts/GeistMono-Medium.otf");
pub const MONO_SEMIBOLD_OTF: &[u8] = include_bytes!("../assets/fonts/GeistMono-SemiBold.otf");
pub const MONO_BOLD_OTF: &[u8] = include_bytes!("../assets/fonts/GeistMono-Bold.otf");

const VENDORED_FONT_BYTES: [&[u8]; 8] = [
    UI_REGULAR_OTF,
    UI_MEDIUM_OTF,
    UI_SEMIBOLD_OTF,
    UI_BOLD_OTF,
    MONO_REGULAR_OTF,
    MONO_MEDIUM_OTF,
    MONO_SEMIBOLD_OTF,
    MONO_BOLD_OTF,
];

pub fn new_font_system() -> FontSystem {
    let mut font_system = FontSystem::new();
    configure_font_system(&mut font_system);
    font_system
}

pub fn configure_font_system(font_system: &mut FontSystem) {
    let db = font_system.db_mut();
    load_vendored_fonts(db);
    db.set_sans_serif_family(UI_FAMILY);
    db.set_monospace_family(MONO_FAMILY);
}

fn load_vendored_fonts(db: &mut fontdb::Database) {
    for font_bytes in VENDORED_FONT_BYTES {
        db.load_font_data(font_bytes.to_vec());
    }
}
