use diffy::render::capture::scene_to_png;
use diffy::render::Scene;
use diffy::ui::element::*;
use diffy::ui::icons::lucide;
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
fn capture_app() {
    render_to_png("app", 1320, 840, |t| {
        let tc = &t.colors;
        let border = tc.border_variant;
        let files = [
            ("src/main.rs", 42u32, 8u32),
            ("src/lib.rs", 156, 23),
            ("src/render/renderer.rs", 384, 12),
            ("src/ui/element.rs", 221, 0),
            ("src/ui/shell.rs", 861, 842),
            ("Cargo.toml", 3, 0),
            ("README.md", 12, 4),
        ];

        div()
            .w(1320.0).h(840.0)
            .flex_col()
            .bg(tc.background)
            // Title bar
            .child(
                div().flex_row().items_center().h_12().w_full().px_5()
                    .bg(tc.title_bar_background).border_b(border)
                    .child(svg_icon(lucide::GIT_COMPARE, 18.0).color(tc.accent))
                    .child(div().w(10.0))
                    .child(text("diffy").semibold().color(tc.text_strong))
                    .child(div().px_4().child(
                        text("7 files  \u{00b7}  abc1234 \u{2192} def5678").text_sm().color(tc.text_muted)
                    ))
                    .child(spacer())
                    .child(div().flex_row().items_center().gap_1()
                        .child(div().flex_row().items_center().gap(6.0).px_3().py_1().rounded_md()
                            .bg(tc.element_background)
                            .child(svg_icon(lucide::GIT_COMPARE, 14.0).color(tc.text))
                            .child(text("Compare").text_sm().medium().color(tc.text)))
                        .child(div().flex_row().items_center().gap(6.0).px_3().py_1().rounded_md()
                            .child(svg_icon(lucide::GIT_PULL_REQUEST, 14.0).color(tc.text_muted))
                            .child(text("PR").text_sm().color(tc.text_muted)))
                        .child(div().w(1.0).h(20.0).bg(border))
                        .child(div().flex_row().rounded_md().bg(Color::rgba(255, 255, 255, 10)).p(2.0).gap(1.0)
                            .child(div().px_3().py_1().rounded_sm().bg(tc.element_background)
                                .child(text("Split").text_xs().medium().color(tc.text)))
                            .child(div().px_3().py_1().rounded_sm()
                                .child(text("Unified").text_xs().color(tc.text_muted))))
                        .child(div().flex_row().items_center().gap(6.0).px_3().py_1().rounded_md()
                            .child(svg_icon(lucide::WRAP_TEXT, 14.0).color(tc.text_muted))
                            .child(text("Wrap").text_sm().color(tc.text_muted)))
                        .child(div().px_2().py_1().rounded_md()
                            .child(svg_icon(lucide::MOON, 15.0).color(tc.text_muted)))
                    )
            )
            // Body
            .child(
                div().flex_row().flex_1()
                    // Sidebar
                    .child(
                        div().w(280.0).flex_shrink_0().h_full().flex_col()
                            .bg(tc.sidebar_background).border_r(border)
                            .child(div().px_4().py_3().flex_row().items_center()
                                .child(text("Files").text_xs().semibold().color(tc.text_muted))
                                .child(div().px_2().child(
                                    div().px(6.0).py(2.0).rounded_sm()
                                        .bg(Color::rgba(255, 255, 255, 10))
                                        .child(text("7").text_xs().color(tc.text_muted))
                                ))
                            )
                            .child(
                                div().flex_1().flex_col().gap(1.0).clip()
                                    .children_from(files.iter().enumerate().map(|(i, (path, add, del))| {
                                        let selected = i == 2;
                                        div()
                                            .w_full().h(36.0)
                                            .flex_row().items_center().px(10.0).gap_2()
                                            .when(selected, |d| d
                                                .bg(tc.sidebar_row_selected)
                                                .border_l(tc.accent))
                                            .child(svg_icon(lucide::FILE_CODE, 15.0).color(
                                                if selected { tc.text_accent } else { tc.text_muted }
                                            ))
                                            .child(div().flex_1().flex_col().gap(1.0)
                                                .child(text(*path).text_sm()
                                                    .color(if selected { tc.text_strong } else { tc.text })
                                                    .truncate())
                                            )
                                            .child(
                                                div().flex_row().gap(4.0).flex_shrink_0()
                                                    .child(text(format!("+{add}")).text_xs().color(tc.line_add_text))
                                                    .child(text(format!("\u{2212}{del}")).text_xs().color(tc.line_del_text))
                                            )
                                            .into_any()
                                    }))
                            )
                    )
                    // Main
                    .child(
                        div().flex_1().h_full().flex_col().bg(tc.editor_surface)
                            .child(
                                div().h(36.0).px_4().flex_row().items_center().border_b(border)
                                    .child(svg_icon(lucide::FILE_CODE, 14.0).color(tc.text_muted))
                                    .child(div().w(8.0))
                                    .child(text("src/render/renderer.rs").text_sm().color(tc.text_muted))
                            )
                            .child(div().flex_1())
                    )
            )
            // Status bar
            .child(
                div().flex_row().items_center().h(28.0).w_full().px_4()
                    .bg(tc.status_bar_background).border_t(border)
                    .child(svg_icon(lucide::CHECK, 12.0).color(tc.line_add_text))
                    .child(div().w(6.0))
                    .child(text("ready").text_xs().color(tc.text_muted))
                    .child(spacer())
                    .child(text("two-dot  \u{00b7}  built-in").text_xs().color(tc.text_muted))
            )
            .into_any()
    });
}
