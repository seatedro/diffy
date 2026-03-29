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
    render_to_png("empty_state", 1320, 840, |t| {
        let tc = &t.colors;
        div()
            .w(1320.0).h(840.0)
            .flex_col()
            .bg(tc.background)
            .p(4.0).gap(4.0)
            .child(
                div().flex_row().items_center().h_12().w_full().px_5()
                    .bg(tc.title_bar_background).rounded_lg().border_b(tc.border_variant)
                    .child(text("diffy").text_lg().color(tc.text_strong))
                    .child(spacer())
                    .child(div().flex_row().gap_2()
                        .child(div().px_3().py_1().rounded_md().bg(tc.element_background)
                            .child(text("Compare").text_sm().color(tc.text)))
                        .child(div().px_3().py_1().rounded_md()
                            .child(text("PR").text_sm().color(tc.text_muted)))
                        .child(div().w(8.0).h(20.0).border_b(tc.border_variant))
                        .child(div().flex_row().rounded_md().bg(Color::rgba(0,0,0,40)).p(2.0).gap(1.0)
                            .child(div().px_3().py_1().rounded_sm().bg(tc.element_background)
                                .child(text("Split").text_xs().color(tc.text)))
                            .child(div().px_3().py_1().rounded_sm()
                                .child(text("Unified").text_xs().color(tc.text_muted))))
                    ),
            )
            .child(
                div().flex_row().flex_1().gap(4.0)
                    .child(
                        div().w(280.0).flex_shrink_0().h_full().flex_col()
                            .bg(tc.sidebar_background).rounded_lg().border_b(tc.border_variant)
                            .child(div().px_4().py_3().child(text("Files").text_xs().color(tc.text_muted)))
                            .child(div().px_4().child(text("Open a repository to start.").text_sm().color(tc.text_muted)))
                    )
                    .child(
                        div().flex_1().h_full().flex_col()
                            .bg(tc.editor_surface).rounded_lg().border_b(tc.border_variant)
                            .items_center().justify_center()
                            .child(
                                div().w(520.0).p_8().flex_col().gap_4()
                                    .bg(tc.elevated_surface).rounded_xl().border_b(tc.border)
                                    .shadow(20.0, 8.0, Color::rgba(0,0,0,80))
                                    .shadow(4.0, 2.0, Color::rgba(0,0,0,40))
                                    .child(text("Start a new compare").text_lg().color(tc.text_strong))
                                    .child(text("Choose a repository, select refs, then open the native diff workspace.").text_sm().color(tc.text_muted))
                                    .child(div().flex_row().gap_3().pt(4.0)
                                        .child(div().px_4().py_2().rounded_md().bg(tc.accent)
                                            .child(text("Open Compare").text_sm().color(tc.text_strong)))
                                        .child(div().px_4().py_2().rounded_md().bg(tc.element_background)
                                            .child(text("Folder Dialog").text_sm().color(tc.text))))
                                    .child(div().pt(8.0).flex_col().gap_1()
                                        .child(text("Recent repositories").text_xs().color(tc.text_muted)))
                            )
                    )
            )
            .child(
                div().flex_row().items_center().h(28.0).w_full().px_4()
                    .bg(tc.status_bar_background).rounded_lg()
                    .child(text("idle").text_xs().color(tc.text_muted))
                    .child(spacer())
                    .child(text("single-commit  \u{00b7}  built-in").text_xs().color(tc.text_muted))
            )
            .into_any()
    });
}

#[test]
fn capture_with_files() {
    render_to_png("with_files", 1320, 840, |t| {
        let tc = &t.colors;
        let files = [
            ("src/main.rs", "+42 \u{2212}8"),
            ("src/lib.rs", "+156 \u{2212}23"),
            ("src/render/renderer.rs", "+384 \u{2212}12"),
            ("src/ui/element.rs", "+221 \u{2212}0"),
            ("src/ui/shell.rs", "+861 \u{2212}842"),
            ("Cargo.toml", "+3 \u{2212}0"),
            ("README.md", "+12 \u{2212}4"),
        ];

        div()
            .w(1320.0).h(840.0)
            .flex_col()
            .bg(tc.background)
            .p(4.0).gap(4.0)
            .child(
                div().flex_row().items_center().h_12().w_full().px_5()
                    .bg(tc.title_bar_background).rounded_lg().border_b(tc.border_variant)
                    .child(text("diffy").text_lg().color(tc.text_strong))
                    .child(div().px_4().child(text("7 files  \u{00b7}  abc1234 \u{2192} def5678").text_sm().color(tc.text_muted)))
                    .child(spacer())
                    .child(div().flex_row().gap_2()
                        .child(div().flex_row().rounded_md().bg(Color::rgba(0,0,0,40)).p(2.0).gap(1.0)
                            .child(div().px_3().py_1().rounded_sm().bg(tc.element_background)
                                .child(text("Split").text_xs().color(tc.text)))
                            .child(div().px_3().py_1().rounded_sm()
                                .child(text("Unified").text_xs().color(tc.text_muted))))
                    )
            )
            .child(
                div().flex_row().flex_1().gap(4.0)
                    .child(
                        div().w(280.0).flex_shrink_0().h_full().flex_col()
                            .bg(tc.sidebar_background).rounded_lg().border_b(tc.border_variant)
                            .child(div().px_4().py_3()
                                .child(text("Files  \u{00b7}  7").text_xs().color(tc.text_muted)))
                            .child(
                                div().flex_1().flex_col().px(6.0).gap_1().clip()
                                    .children_from(files.iter().enumerate().map(|(i, (path, stats))| {
                                        let selected = i == 2;
                                        div()
                                            .w_full().h(36.0)
                                            .flex_row().items_center().px_3().rounded_md()
                                            .when(selected, |d| d.bg(tc.sidebar_row_selected))
                                            .child(div().flex_1().flex_col().gap(2.0)
                                                .child(text(*path).text_sm().color(tc.text))
                                                .child(text(*stats).text_xs().color(tc.text_muted)))
                                            .into_any()
                                    }))
                            )
                    )
                    .child(
                        div().flex_1().h_full().flex_col()
                            .bg(tc.editor_surface).rounded_lg().border_b(tc.border_variant)
                            .child(
                                div().h(36.0).px_4().flex_row().items_center()
                                    .border_b(tc.border_variant)
                                    .child(text("src/render/renderer.rs").text_sm().color(tc.text_muted))
                            )
                            .child(div().flex_1())
                    )
            )
            .child(
                div().flex_row().items_center().h(28.0).w_full().px_4()
                    .bg(tc.status_bar_background).rounded_lg()
                    .child(text("ready").text_xs().color(tc.text_muted))
                    .child(spacer())
                    .child(text("two-dot  \u{00b7}  built-in").text_xs().color(tc.text_muted))
            )
            .into_any()
    });
}
