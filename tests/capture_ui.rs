//! Captures the app UI to PNG files for visual inspection.
//!
//! Run with: cargo test --test capture_ui -- --nocapture

use diffy::render::capture::scene_to_png;
use diffy::render::Scene;
use diffy::ui::element::*;
use diffy::ui::signals::SignalStore;
use diffy::ui::style::Styled;
use diffy::ui::theme::{Color, Theme};

fn render_to_png(name: &str, width: u32, height: u32, build: impl FnOnce(&Theme) -> AnyElement) {
    let theme = Theme::default_dark();
    let mut font_system = glyphon::FontSystem::new();
    let mut store = SignalStore::new();
    let mut cx = ElementContext::new(&theme, 1.0, &mut font_system, None, &mut store);

    let mut root = build(&theme);
    let mut scene = Scene::default();
    render_element(&mut root, &mut scene, &mut cx, width as f32, height as f32);

    let dir = std::path::Path::new("target/captures");
    std::fs::create_dir_all(dir).ok();
    let path = dir.join(format!("{name}.png"));
    scene_to_png(&scene, width, height, &path);
    eprintln!("captured: {}", path.display());
}

#[test]
fn capture_empty_state() {
    render_to_png("empty_state", 1320, 840, |theme| {
        div()
            .w(1320.0)
            .h(840.0)
            .flex_col()
            .bg(theme.colors.background)
            .p(8.0)
            .gap(8.0)
            // Title bar
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .h(52.0)
                    .w_full()
                    .px(20.0)
                    .bg(theme.colors.title_bar_background)
                    .rounded(10.0)
                    .child(text("diffy").text_lg().color(theme.colors.text_strong))
                    .child(spacer())
                    .child(
                        div()
                            .flex_row()
                            .gap(8.0)
                            .child(
                                div()
                                    .px(14.0)
                                    .py(6.0)
                                    .rounded(7.0)
                                    .bg(theme.colors.element_background)
                                    .child(text("Compare").text_sm().color(theme.colors.text)),
                            )
                            .child(
                                div()
                                    .px(14.0)
                                    .py(6.0)
                                    .rounded(7.0)
                                    .child(text("PR").text_sm().color(theme.colors.text_muted)),
                            ),
                    ),
            )
            // Body: sidebar + main
            .child(
                div()
                    .flex_row()
                    .flex_1()
                    .gap(8.0)
                    // Sidebar
                    .child(
                        div()
                            .w(260.0)
                            .flex_shrink_0()
                            .h_full()
                            .flex_col()
                            .bg(theme.colors.sidebar_background)
                            .rounded(10.0)
                            .p(12.0)
                            .child(text("Files").text_sm().color(theme.colors.text_muted))
                            .child(
                                text("Open a repository to start.")
                                    .text_sm()
                                    .color(theme.colors.text_muted),
                            ),
                    )
                    // Main
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .bg(theme.colors.editor_surface)
                            .rounded(10.0)
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .w(540.0)
                                    .p(28.0)
                                    .flex_col()
                                    .gap(12.0)
                                    .bg(theme.colors.empty_state_background)
                                    .border_b(theme.colors.empty_state_border)
                                    .rounded(10.0)
                                    .child(text("Start a new compare").text_lg().color(theme.colors.text))
                                    .child(
                                        text("Choose a repository, select refs, then open the native diff workspace.")
                                            .text_sm()
                                            .color(theme.colors.text_muted),
                                    )
                                    .child(
                                        div()
                                            .flex_row()
                                            .gap(16.0)
                                            .child(
                                                div()
                                                    .px(16.0)
                                                    .py(8.0)
                                                    .rounded(7.0)
                                                    .bg(theme.colors.accent)
                                                    .child(text("Open Compare").text_sm().color(theme.colors.text_strong)),
                                            )
                                            .child(
                                                div()
                                                    .px(16.0)
                                                    .py(8.0)
                                                    .rounded(7.0)
                                                    .bg(theme.colors.element_background)
                                                    .child(text("Folder Dialog").text_sm().color(theme.colors.text)),
                                            ),
                                    )
                                    .child(text("Recent repositories").text_sm().color(theme.colors.text_muted)),
                            ),
                    ),
            )
            // Status bar
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .h(30.0)
                    .w_full()
                    .px(16.0)
                    .bg(theme.colors.status_bar_background)
                    .rounded(10.0)
                    .child(text("idle").text_xs().color(theme.colors.text_muted))
                    .child(spacer())
                    .child(text("single-commit  ·  built-in").text_xs().color(theme.colors.text_muted)),
            )
            .into_any()
    });
}

#[test]
fn capture_with_files() {
    render_to_png("with_files", 1320, 840, |theme| {
        let files = [
            ("src/main.rs", "+42 −8"),
            ("src/lib.rs", "+156 −23"),
            ("src/render/renderer.rs", "+384 −12"),
            ("src/ui/element.rs", "+221 −0"),
            ("src/ui/shell.rs", "+861 −842"),
            ("Cargo.toml", "+3 −0"),
            ("README.md", "+12 −4"),
        ];

        div()
            .w(1320.0)
            .h(840.0)
            .flex_col()
            .bg(theme.colors.background)
            .p(8.0)
            .gap(8.0)
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .h(52.0)
                    .w_full()
                    .px(20.0)
                    .bg(theme.colors.title_bar_background)
                    .rounded(10.0)
                    .child(text("diffy").text_lg().color(theme.colors.text_strong))
                    .child(
                        text("  7 files  ·  abc1234 → def5678")
                            .text_sm()
                            .color(theme.colors.text_muted),
                    )
                    .child(spacer())
                    .child(
                        div()
                            .flex_row()
                            .gap(8.0)
                            .child(
                                div()
                                    .px(12.0)
                                    .py(4.0)
                                    .rounded(5.0)
                                    .bg(theme.colors.element_background)
                                    .child(text("Split").text_xs().color(theme.colors.text)),
                            )
                            .child(
                                div()
                                    .px(12.0)
                                    .py(4.0)
                                    .rounded(5.0)
                                    .child(text("Unified").text_xs().color(theme.colors.text_muted)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_row()
                    .flex_1()
                    .gap(8.0)
                    .child(
                        div()
                            .w(260.0)
                            .flex_shrink_0()
                            .h_full()
                            .flex_col()
                            .bg(theme.colors.sidebar_background)
                            .rounded(10.0)
                            .p(12.0)
                            .gap(8.0)
                            .child(text("Files  ·  7").text_sm().color(theme.colors.text_muted))
                            .children_from(files.iter().enumerate().map(|(i, (path, stats))| {
                                div()
                                    .w_full()
                                    .h(34.0)
                                    .flex_row()
                                    .items_center()
                                    .px(8.0)
                                    .rounded(7.0)
                                    .when(i == 2, |d| d.bg(theme.colors.sidebar_row_selected))
                                    .child(
                                        div()
                                            .flex_1()
                                            .flex_col()
                                            .child(text(*path).text_sm().color(theme.colors.text))
                                            .child(text(*stats).text_xs().color(theme.colors.text_muted)),
                                    )
                                    .into_any()
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex_col()
                            .bg(theme.colors.editor_surface)
                            .rounded(10.0)
                            .child(
                                div()
                                    .h(32.0)
                                    .px(16.0)
                                    .flex_row()
                                    .items_center()
                                    .child(text("src/render/renderer.rs").text_sm().color(theme.colors.text_muted)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .bg(theme.colors.editor_surface),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_row()
                    .items_center()
                    .h(30.0)
                    .w_full()
                    .px(16.0)
                    .bg(theme.colors.status_bar_background)
                    .rounded(10.0)
                    .child(text("ready").text_xs().color(theme.colors.text_muted))
                    .child(spacer())
                    .child(text("two-dot  ·  built-in").text_xs().color(theme.colors.text_muted)),
            )
            .into_any()
    });
}
