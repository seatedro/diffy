//! The diff view's presentation: change markers, gutter modes, separators,
//! headers and their slots, the missing-side fill, the split divider,
//! automatic layout, and local colors, checked through painted pixels,
//! scene geometry, and the accessibility tree at several scales.
//!
//! Pixel tests skip on hosts without a wgpu adapter unless
//! `QUARK_REQUIRE_GPU` is set (Linux: point `VK_DRIVER_FILES` at Mesa's
//! lavapipe ICD). `DIFFY_DIFF_SHOTS=<dir> cargo test --test diff_paint --
//! --ignored shots` writes review crops.

use std::cell::RefCell;
use std::rc::Rc;

use accesskit::Role;
use diffy_diff::{DiffDocument, Mode, diff_texts};
use diffy_ui::diff_view::decorator::{DiffDecorator, HeaderContext, HeaderSlot};
use diffy_ui::diff_view::diff_view_with;
use diffy_ui::diff_view::presentation::{
    DiffAppearance, DiffColorOverrides, DiffLayout, DiffNumbers, DiffPresentation, EmptySideFill,
    FileHeaders, HunkSeparator,
};
use diffy_ui::{DiffEvent, DiffViewState};
use quark::path::PathVerb;
use quark::view;
use quark_app::quark_ui::element::{AnyElement, IntoAnyElement, div, text};
use quark_app::quark_ui::style::Styled;
use quark_app::quark_ui::theme::{Color, Theme};
use quark_app::quark_ui::{Action, FocusId};
use quark_app::testing::{By, Node, Pixels, UiTestHarness};
use quark_app::{UiApp, UiContext, ViewContext};
use quark_components::CollectionEnv;
use quark_render::Primitive;

const FOCUS: FocusId = FocusId::from_key("paint.diff");
const SCALES: [f32; 3] = [1.0, 1.5, 2.0];
const MAGENTA: Color = Color::rgba(255, 0, 255, 255);

const OLD: &str = "import { round } from \"./math.js\";\n\nexport function subtotal(items) {\n  return items.reduce((sum, item) => sum + item.price, 0);\n}\n\nexport function tax(total) {\n  return round(total * 0.2);\n}\n\nexport function applyDiscount(total, percent) {\n  return total - percent;\n}\n";
const NEW: &str = "import { round } from \"./math.js\";\n\nexport function subtotal(items) {\n  return items.reduce((sum, item) => sum + item.price * item.qty, 0);\n}\n\nexport function tax(total) {\n  return round(total * 0.2);\n}\n\nexport function applyDiscount(total, percent) {\n  const off = total * percent / 100;\n  return total - off;\n}\n";

/// `cart.js` with two hunks and a collapsed gap between them.
fn cart() -> DiffDocument {
    diff_texts(Some("cart.js"), Some("cart.js"), Some(OLD), Some(NEW), 1)
}

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Diff(DiffEvent),
    Stage(u32),
}

impl From<Msg> for Action {
    fn from(msg: Msg) -> Self {
        Action::new(msg)
    }
}

struct Host {
    diff: DiffViewState,
    decorator: Option<Rc<dyn DiffDecorator>>,
    staged: Vec<u32>,
}

impl UiApp for Host {
    type Action = Msg;
    type Message = Theme;

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        self.diff.set_viewport(width, height);
        let scale = cx.frame.scale_factor();
        let text = cx.frame.text();
        self.diff
            .prepare(&mut text.system, &mut text.layouts, scale, 0);
        let env = CollectionEnv {
            focused: cx.is_focused(FOCUS),
            accessible: cx.frame.accessibility_active(),
        };
        let body = diff_view_with(
            &mut self.diff,
            cx.theme,
            env,
            |e| Msg::Diff(e).into(),
            self.decorator.clone(),
        );
        view! { <div w={width} h={height}>{body}</div> }
    }

    fn update(&mut self, msg: Msg, _cx: &mut UiContext) {
        match msg {
            Msg::Diff(event) => {
                self.diff.handle(event);
            }
            Msg::Stage(file) => self.staged.push(file),
        }
    }

    fn message(&mut self, theme: Theme, cx: &mut UiContext) {
        cx.set_theme(theme);
    }
}

fn open(
    doc: DiffDocument,
    presentation: DiffPresentation,
    size: (f32, f32),
    scale: f32,
) -> UiTestHarness<Host> {
    let mut diff = DiffViewState::new("paint.diff", FOCUS, doc);
    diff.set_presentation(presentation);
    let mut ui = UiTestHarness::new(
        Host {
            diff,
            decorator: None,
            staged: Vec::new(),
        },
        size,
        scale,
    );
    ui.frame();
    ui
}

fn split(presentation: DiffPresentation) -> DiffPresentation {
    DiffPresentation {
        layout: DiffLayout::Split,
        ..presentation
    }
}

/// The line rows whose text contains `needle`, in tree order.
fn lines(ui: &UiTestHarness<Host>, needle: &str) -> Vec<Node> {
    ui.find_all(By::role(Role::ListItem))
        .into_iter()
        .filter(|n| n.name.as_deref().is_some_and(|name| name.contains(needle)))
        .collect()
}

fn line(ui: &UiTestHarness<Host>, needle: &str) -> Node {
    let mut found = lines(ui, needle);
    assert_eq!(found.len(), 1, "one line contains {needle:?}");
    found.remove(0)
}

fn pixels(ui: &mut UiTestHarness<Host>) -> Option<Pixels> {
    match ui.render_rgba() {
        Ok(pixels) => Some(pixels),
        Err(quark_render::RenderError::NoAdapter) => {
            assert!(
                std::env::var_os("QUARK_REQUIRE_GPU").is_none(),
                "QUARK_REQUIRE_GPU is set but no wgpu adapter is available"
            );
            None
        }
        Err(error) => panic!("render failed: {error}"),
    }
}

fn is(px: [u8; 4], color: Color) -> bool {
    px[..3] == [color.r, color.g, color.b]
}

/// Physical pixel rows fully inside the `top..top + height` points.
fn pixel_rows(top: f32, height: f32, scale: f32) -> std::ops::Range<u32> {
    (top * scale).ceil() as u32..((top + height) * scale).floor() as u32
}

/// Lengths of the runs of `color` and of other colors down column `x`,
/// alternating, starting with whichever comes first.
fn runs(px: &Pixels, x: u32, rows: std::ops::Range<u32>, color: Color) -> Vec<(bool, u32)> {
    let mut out: Vec<(bool, u32)> = Vec::new();
    for y in rows {
        let on = is(px.pixel(x, y), color);
        match out.last_mut() {
            Some((last, n)) if *last == on => *n += 1,
            _ => out.push((on, 1)),
        }
    }
    out
}

#[test]
fn changed_lines_get_a_solid_bar_against_the_code() {
    let same = DiffAppearance::both(DiffColorOverrides {
        add_marker: Some(MAGENTA),
        del_marker: Some(MAGENTA),
        ..Default::default()
    });
    for scale in SCALES {
        let mut ui = open(cart(), DiffPresentation::compact(), (420.0, 300.0), scale);
        ui.app_mut().diff.set_appearance(same);
        ui.frame();
        let removed = line(&ui, "total - percent;").bounds;
        let added = line(&ui, "total - off;").bounds;
        let Some(px) = pixels(&mut ui) else { return };
        for row in [removed, added] {
            let y = ((row.y + row.height / 2.0) * scale) as u32;
            let code = (row.x * scale).round() as u32;
            let bar: Vec<u32> = (0..code + 2)
                .filter(|&x| is(px.pixel(x, y), MAGENTA))
                .collect();
            assert_eq!(
                bar.len(),
                (3.0 * scale).round() as usize,
                "{scale}x: {bar:?}"
            );
            assert_eq!(bar.last().copied(), Some(code - 1), "{scale}x: {bar:?}");
            let column = runs(&px, bar[0], pixel_rows(row.y, row.height, scale), MAGENTA);
            assert!(matches!(column[..], [(true, _)]), "{scale}x: {column:?}");
        }
    }
}

// Catches the split divider going soft or wide: exactly one physical
// pixel column of the border color separates the sides at every scale.
#[test]
fn split_sides_are_divided_by_one_pixel_line() {
    for scale in SCALES {
        let mut ui = open(
            cart(),
            split(DiffPresentation::default()),
            (760.0, 300.0),
            scale,
        );
        let border = ui.theme().colors.border_variant;
        let row = lines(&ui, "applyDiscount")[0].bounds;
        let Some(px) = pixels(&mut ui) else { return };
        let y = ((row.y + row.height / 2.0) * scale) as u32;
        let middle = (380.0 * scale) as u32;
        let columns: Vec<u32> = (middle - 6..middle + 6)
            .filter(|&x| is(px.pixel(x, y), border))
            .collect();
        assert_eq!(columns.len(), 1, "{scale}x: {columns:?}");
    }
}

/// Old `a b`, new `a x1 x2 x3 b`: three new lines with nothing beside them.
fn insertion() -> DiffDocument {
    diff_texts(
        Some("f.txt"),
        Some("f.txt"),
        Some("a\nb\n"),
        Some("a\nx1\nx2\nx3\nb\n"),
        3,
    )
}

/// `x + y` along the first hatch line of each path in the scene, modulo
/// the hatch spacing, with each path's stroke width in physical pixels.
fn hatch_phases(ui: &UiTestHarness<Host>, scale: f32) -> Vec<(f32, f32)> {
    ui.scene()
        .expanded()
        .iter()
        .filter_map(|p| match p {
            Primitive::Path(path) => {
                let stroke = path.stroke?;
                let to = path.path.verbs().iter().find_map(|v| match v {
                    PathVerb::LineTo([x, y]) => Some((*x, *y)),
                    _ => None,
                })?;
                let c = path.origin[0] + to.0 + path.origin[1] + to.1;
                Some((c.rem_euclid(6.0), stroke.style.width * scale))
            }
            _ => None,
        })
        .collect()
}

// Catches the missing side of split rows losing its hatch or the hatch
// breaking between rows: every missing cell carries hairline diagonals
// on one continuous lattice, whatever the row heights.
#[test]
fn missing_split_sides_are_hatched_in_one_continuous_pattern() {
    let hatched = split(DiffPresentation {
        empty_side: EmptySideFill::Hatch,
        ..Default::default()
    });
    for scale in SCALES {
        let ui = open(insertion(), hatched, (600.0, 300.0), scale);
        let phases = hatch_phases(&ui, scale);
        assert_eq!(phases.len(), 3, "{scale}x: one per missing cell");
        for &(phase, stroke) in &phases {
            assert!((phase - phases[0].0).abs() < 1e-3, "{scale}x: {phases:?}");
            assert!((stroke - 1.0).abs() < 1e-3, "{scale}x: {phases:?}");
        }
    }
    let solid = open(
        insertion(),
        split(DiffPresentation::default()),
        (600.0, 300.0),
        1.0,
    );
    assert!(hatch_phases(&solid, 1.0).is_empty());
}

/// Line numbers painted left of `row`'s text, left to right.
fn gutter_numbers(ui: &UiTestHarness<Host>, row: &Node) -> Vec<String> {
    let mut found: Vec<_> = ui
        .painted_texts()
        .into_iter()
        .filter(|t| {
            let mid = t.bounds.y + t.bounds.height / 2.0;
            mid > row.bounds.y
                && mid < row.bounds.y + row.bounds.height
                && t.bounds.x < row.bounds.x
                && t.text.chars().all(|c| c.is_ascii_digit())
        })
        .collect();
    found.sort_by(|a, b| a.bounds.x.total_cmp(&b.bounds.x));
    found.into_iter().map(|t| t.text).collect()
}

// Catches the gutter ignoring its number mode: unified shows both
// numbers, the shown line's only, or none, and the code starts further
// left as columns go away.
#[test]
fn the_gutter_shows_the_numbers_its_mode_asks_for() {
    let cases = [
        (DiffNumbers::Both, vec!["11", "11"], vec!["13"]),
        (DiffNumbers::RelevantSide, vec!["11"], vec!["13"]),
        (DiffNumbers::None, vec![], vec![]),
    ];
    let mut code_x = Vec::new();
    for (numbers, context, added) in cases {
        for scale in SCALES {
            let presentation = DiffPresentation {
                numbers,
                ..Default::default()
            };
            let ui = open(cart(), presentation, (420.0, 300.0), scale);
            let context_row = line(&ui, "applyDiscount");
            assert_eq!(
                gutter_numbers(&ui, &context_row),
                context,
                "{numbers:?} {scale}x"
            );
            let added_row = line(&ui, "total - off;");
            assert_eq!(
                gutter_numbers(&ui, &added_row),
                added,
                "{numbers:?} {scale}x"
            );
            if scale == 1.0 {
                code_x.push(context_row.bounds.x);
            }
        }
    }
    assert!(code_x[0] > code_x[1] && code_x[1] > code_x[2], "{code_x:?}");
}

fn auto() -> DiffPresentation {
    DiffPresentation {
        layout: DiffLayout::AUTO,
        ..Default::default()
    }
}

/// Smallest whole-point width at which a fresh automatic view shows split.
fn split_threshold(scale: f32) -> f32 {
    let mut ui = open(cart(), auto(), (400.0, 300.0), scale);
    for width in 400..1400 {
        ui.resize(width as f32, 300.0);
        ui.frame();
        if ui.app().diff.mode() == Mode::Split {
            return width as f32;
        }
    }
    panic!("never split");
}

// Catches automatic layout flipping at one width, or forgetting it is
// automatic: from unified it splits only with hysteresis columns to
// spare, and once split it stays split until the sides drop below the
// threshold. An explicit choice ignores the width.
#[test]
fn automatic_layout_switches_with_hysteresis_and_explicit_layouts_stay() {
    for scale in SCALES {
        let on = split_threshold(scale);
        let mut ui = open(cart(), auto(), (on, 300.0), scale);
        assert_eq!(ui.app().diff.mode(), Mode::Split, "{scale}x");
        // A few columns narrower: inside the hysteresis band.
        ui.resize(on - 30.0, 300.0);
        ui.frame();
        assert_eq!(ui.app().diff.mode(), Mode::Split, "{scale}x: band");
        let fresh = open(cart(), auto(), (on - 30.0, 300.0), scale);
        assert_eq!(fresh.app().diff.mode(), Mode::Unified, "{scale}x: fresh");
        ui.resize(on - 120.0, 300.0);
        ui.frame();
        assert_eq!(ui.app().diff.mode(), Mode::Unified, "{scale}x: narrow");

        let mut explicit = open(cart(), split(auto()), (300.0, 300.0), scale);
        explicit.resize(320.0, 300.0);
        explicit.frame();
        assert_eq!(
            explicit.app().diff.mode(),
            Mode::Split,
            "{scale}x: explicit"
        );
    }
}

/// Lines 0..200 with lines 50 and 150 changed.
fn long_file() -> DiffDocument {
    let old: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let new = old
        .replace("line 50\n", "line fifty\n")
        .replace("line 150\n", "line one fifty\n");
    diff_texts(Some("f.txt"), Some("f.txt"), Some(&old), Some(&new), 200)
}

/// The text of the topmost line row in view.
fn top_line(ui: &UiTestHarness<Host>) -> String {
    let rows = ui.find_all(By::role(Role::ListItem));
    rows.into_iter()
        .filter(|n| n.bounds.y >= -0.5)
        .min_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y))
        .and_then(|n| n.name)
        .unwrap()
}

// Catches an automatic switch losing the reader's place: crossing the
// threshold and back keeps the same source line at the top.
#[test]
fn automatic_switches_keep_the_top_line() {
    let on = split_threshold(1.0);
    let mut ui = open(long_file(), auto(), (on - 120.0, 300.0), 1.0);
    ui.app_mut().diff.scroll_to_row(90);
    ui.frame();
    let before = top_line(&ui);
    ui.resize(on + 40.0, 300.0);
    ui.frame();
    assert_eq!(ui.app().diff.mode(), Mode::Split);
    assert_eq!(top_line(&ui), before);
    ui.resize(on - 120.0, 300.0);
    ui.frame();
    assert_eq!(ui.app().diff.mode(), Mode::Unified);
    assert_eq!(top_line(&ui), before);
}

fn rect_colors(ui: &UiTestHarness<Host>) -> Vec<Color> {
    ui.scene()
        .expanded()
        .iter()
        .filter_map(|p| match p {
            Primitive::Rect(r) => Some(r.color),
            _ => None,
        })
        .collect()
}

// Catches local colors leaking between modes: the light theme paints the
// light overrides and the dark theme the dark ones.
#[test]
fn light_and_dark_overrides_resolve_independently() {
    let light_marker = Color::rgba(1, 2, 3, 255);
    let dark_marker = Color::rgba(250, 251, 252, 255);
    let appearance = DiffAppearance {
        light: DiffColorOverrides {
            add_marker: Some(light_marker),
            ..Default::default()
        },
        dark: DiffColorOverrides {
            add_marker: Some(dark_marker),
            ..Default::default()
        },
    };
    for (theme, shown, hidden) in [
        (Theme::default_light(), light_marker, dark_marker),
        (Theme::default_dark(), dark_marker, light_marker),
    ] {
        let mut ui = open(cart(), DiffPresentation::compact(), (420.0, 300.0), 1.0);
        ui.app_mut().diff.set_appearance(appearance);
        ui.send_message(theme);
        ui.frame();
        let colors = rect_colors(&ui);
        assert!(colors.contains(&shown));
        assert!(!colors.contains(&hidden));
    }
}

// Catches the compact separator being decoration only: the thin bar is a
// named button, and pressing it shows the lines it stands for.
#[test]
fn a_compact_separator_is_a_button_that_reveals_its_lines() {
    let presentation = DiffPresentation {
        separators: HunkSeparator::Compact,
        ..Default::default()
    };
    for scale in SCALES {
        let mut ui = open(cart(), presentation, (420.0, 400.0), scale);
        assert!(lines(&ui, "export function tax").is_empty());
        let bar = ui.find(By::role_name(Role::Button, "Show all unchanged lines"));
        assert!(bar.bounds.height < 10.0, "{scale}x: {:?}", bar.bounds);
        ui.click(bar.center());
        assert_eq!(lines(&ui, "export function tax").len(), 1, "{scale}x");
    }
}

/// Adds a Stage button to each header's actions slot.
struct StageAction;

impl DiffDecorator for StageAction {
    fn revision(&self) -> u64 {
        0
    }

    fn header_slot(&self, slot: HeaderSlot, cx: &HeaderContext) -> Option<AnyElement> {
        let file = cx.file;
        (slot == HeaderSlot::Actions).then(|| {
            view! {
                <div
                    class="px-[6]"
                    on:click={Action::from(Msg::Stage(file))}
                    accessibility_role={Role::Button}
                    aria-label={"Stage"}
                >
                    <text>"Stage"</text>
                </div>
            }
        })
    }
}

// Catches header slots not reaching the screen or their actions not
// reaching the app: the decorator's Stage button sits in the file's
// header and emits the app's action for that file.
#[test]
fn header_action_slots_show_in_the_header_and_emit_app_actions() {
    let mut ui = open(cart(), DiffPresentation::default(), (420.0, 300.0), 1.0);
    ui.app_mut().decorator = Some(Rc::new(StageAction));
    ui.frame();
    let header = ui.find(By::role(Role::Heading)).bounds;
    let stage = ui.find(By::role_name(Role::Button, "Stage"));
    assert!(stage.bounds.y >= header.y && stage.bounds.y < header.y + header.height);
    ui.click(stage.center());
    assert_eq!(ui.app().staged, [0]);
}

/// A custom header naming its file.
struct Banner(RefCell<u32>);

impl DiffDecorator for Banner {
    fn revision(&self) -> u64 {
        0
    }

    fn header(&self, cx: &HeaderContext) -> Option<AnyElement> {
        *self.0.borrow_mut() += 1;
        let label = format!("Banner for {}", cx.title);
        Some(view! {
            <div accessibility_role={Role::Heading} aria-label={label}>
                <text>"banner"</text>
            </div>
        })
    }
}

// Catches the header choice being ignored: hidden headers take no space
// and leave no heading, and custom headers replace the built-in one.
#[test]
fn file_headers_hide_or_come_from_the_decorator() {
    let hidden = DiffPresentation {
        headers: FileHeaders::Hidden,
        ..Default::default()
    };
    let ui = open(cart(), hidden, (420.0, 300.0), 1.5);
    assert!(ui.try_find(By::role(Role::Heading)).is_none());
    assert_eq!(line(&ui, "import").bounds.y, 0.0);

    let custom = DiffPresentation {
        headers: FileHeaders::Custom,
        ..Default::default()
    };
    let mut ui = open(cart(), custom, (420.0, 300.0), 1.0);
    ui.app_mut().decorator = Some(Rc::new(Banner(RefCell::new(0))));
    ui.frame();
    let headings = ui.find_all(By::role(Role::Heading));
    assert_eq!(headings.len(), 1);
    assert_eq!(headings[0].name.as_deref(), Some("Banner for cart.js"));
}

#[test]
#[ignore = "writes PNG crops to DIFFY_DIFF_SHOTS"]
fn shots() {
    let Some(dir) = std::env::var_os("DIFFY_DIFF_SHOTS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let scenes = [
        ("unified", DiffPresentation::default(), 420.0),
        ("split", split(DiffPresentation::default()), 760.0),
        ("compact", DiffPresentation::compact(), 420.0),
        ("review-split", split(DiffPresentation::review()), 760.0),
    ];
    for (name, presentation, width) in scenes {
        for (theme_name, theme) in [
            ("dark", Theme::default_dark()),
            ("light", Theme::default_light()),
        ] {
            for scale in SCALES {
                let mut ui = open(cart(), presentation, (width, 300.0), scale);
                ui.send_message(theme.clone());
                ui.frame();
                let px = pixels(&mut ui).expect("a wgpu adapter");
                let img = image::RgbaImage::from_raw(px.width, px.height, px.rgba).unwrap();
                img.save(dir.join(format!("{name}-{theme_name}-{scale}x.png")))
                    .unwrap();
            }
        }
    }
}
