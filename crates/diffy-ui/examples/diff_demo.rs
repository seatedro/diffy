//! A diff viewer. `diff_demo OLD NEW` diffs two files, `diff_demo FILE`
//! shows a unified diff (`git diff > FILE`), and with no arguments it shows
//! a built-in sample of several files.
//!
//! The file list on the left jumps to a file. In the view: `n` and `p` (or
//! Alt+Down and Alt+Up) step through hunks, `]` and `[` through files, drag
//! selects (side by side, the side you start on), Ctrl/Cmd+C copies,
//! Ctrl/Cmd+A selects all. The toolbar switches between unified and side by
//! side and turns word wrap on. Build with `--features syntax` for syntax
//! colors, after building the grammar packs with
//! `cargo run -p syntax-pack -- build` (`DIFFY_SYNTAX_PACKS` names another
//! pack root).

use std::rc::Rc;

use diffy_diff::{DiffDocument, Mode, diff_texts, parse_unified};
use diffy_ui::{DiffEvent, DiffOutcome, DiffStyle, DiffViewState, diff_view};
use quark::view;
use quark_app::quark_ui::element::{AnyElement, IntoAnyElement, cached, div, inputs_hash, text};
use quark_app::quark_ui::style::Styled;
use quark_app::quark_ui::{Action, FocusId};
use quark_app::{UiApp, UiContext, ViewContext, WindowOptions};
use quark_components::CollectionEnv;

const DIFF_FOCUS: FocusId = FocusId::from_key("demo.diff");
const TOOLBAR_H: f32 = 36.0;
const SIDEBAR_W: f32 = 240.0;

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Diff(DiffEvent),
    Mode(Mode),
    ToggleWrap,
    OpenFile(u32),
}

impl From<Msg> for Action {
    fn from(msg: Msg) -> Self {
        Action::new(msg)
    }
}

struct Demo {
    diff: DiffViewState,
    /// File list rows: path, status, additions, deletions.
    files: Rc<[(String, &'static str, u32, u32)]>,
    title: Rc<str>,
}

impl Demo {
    fn new(doc: DiffDocument, title: String) -> Self {
        let files = doc
            .summaries()
            .map(|s| {
                (
                    s.path.to_string(),
                    s.status.name(),
                    s.additions,
                    s.deletions,
                )
            })
            .collect();
        Self {
            diff: DiffViewState::new("demo.diff", DIFF_FOCUS, doc).with_label("Changes"),
            files,
            title: title.into(),
        }
    }
}

impl UiApp for Demo {
    type Action = Msg;
    type Message = ();

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        let sidebar = if width > SIDEBAR_W * 2.5 {
            SIDEBAR_W
        } else {
            0.0
        };
        self.diff
            .set_viewport((width - sidebar).max(0.0), (height - TOOLBAR_H).max(0.0));
        let scale = cx.frame.scale_factor();
        let now_ms = cx.frame.elapsed().as_millis() as u64;
        let text_cx = cx.frame.text();
        self.diff
            .prepare(&mut text_cx.system, &mut text_cx.layouts, scale, now_ms);
        let env = CollectionEnv {
            focused: cx.is_focused(DIFF_FOCUS),
            accessible: cx.frame.accessibility_active(),
        };
        let view = diff_view(&mut self.diff, cx.theme, env, |e| Msg::Diff(e).into());
        let colors = cx.theme.colors;

        let (mode, wrap) = (self.diff.mode(), self.diff.style().wrap);
        let title = self.title.clone();
        let toolbar = move || {
            let toggle = |label: &'static str, on: bool, msg: Msg| {
                view! {
                    <div
                        class="px-[10] h-6 rounded-[4] flex-row items-center"
                        hover_bg={colors.element_hover}
                        on:click={msg}
                        @when {on} { bg={colors.element_selected} }
                    >
                        <text class="text-sm" color={colors.text}>{label}</text>
                    </div>
                }
            };
            view! {
                <div
                    w={width}
                    h={TOOLBAR_H}
                    class="flex-row items-center gap-[6] px-[10] bg-[colors.title_bar_background]
                            border-b-[colors.border]"
                >
                    <text class="text-sm font-semibold" color={colors.text}>{&*title}</text>
                    <div class="flex-1" />
                    {toggle("Unified", mode == Mode::Unified, Msg::Mode(Mode::Unified))}
                    {toggle("Side by side", mode == Mode::Split, Msg::Mode(Mode::Split))}
                    {toggle("Wrap", wrap, Msg::ToggleWrap)}
                </div>
            }
        };
        let files = self.files.clone();
        let list = move || {
            view! {
                <div
                    w={sidebar}
                    h={height - TOOLBAR_H}
                    class="flex-col overflow-clip bg-[colors.sidebar_background] border-r-[colors.border]"
                >
                    for (i, (path, status, adds, dels)) in files.iter().enumerate() {
                        <div
                            class="w-full h-7 flex-row items-center gap-[6] px-[10]"
                            hover_bg={colors.sidebar_row_hover}
                            on:click={Msg::OpenFile(i as u32)}
                        >
                            <text class="text-xs" color={colors.text_muted}>{*status}</text>
                            <text class="text-sm" color={colors.text} class="truncate">
                                {path.as_str()}
                            </text>
                            <div class="flex-1" />
                            <text class="text-xs" color={colors.line_add_text}>"+{adds}"</text>
                            <text class="text-xs" color={colors.line_del_text}>"-{dels}"</text>
                        </div>
                    }
                </div>
            }
        };
        view! {
            <div w={width} h={height} class="flex-col bg-[colors.background]">
                <cached("demo.toolbar", inputs_hash(&(mode, wrap, width.to_bits())), toolbar)
                    w={width}
                    h={TOOLBAR_H}
                />
                <div w={width} h={height - TOOLBAR_H} class="flex-row">
                    if sidebar > 0.0 {
                        <cached(
                            "demo.files",
                            inputs_hash(&(sidebar.to_bits(), height.to_bits())),
                            list
                        )
                            w={sidebar}
                            h={height - TOOLBAR_H}
                        />
                    }
                    {view}
                </div>
            </div>
        }
    }

    fn update(&mut self, msg: Msg, cx: &mut UiContext) {
        match msg {
            Msg::Diff(event) => {
                if let DiffOutcome::Copy(copied) = self.diff.handle(event)
                    && !copied.is_empty()
                {
                    cx.window.set_clipboard_text(&copied);
                }
            }
            Msg::Mode(mode) => self.diff.set_mode(mode),
            Msg::ToggleWrap => {
                let style = self.diff.style();
                self.diff.set_style(DiffStyle {
                    wrap: !style.wrap,
                    ..style
                });
            }
            Msg::OpenFile(file) => {
                let row = self.diff.projection().file_rows[file as usize];
                self.diff.scroll_to_row(row);
            }
        }
    }
}

/// Two versions of a small crate: a modified source file, a renamed and
/// edited README, a new file, a deleted one, and a script made executable.
fn sample() -> DiffDocument {
    let old_lib = r#"//! A tiny tokenizer.

use std::fmt;

/// A token of the input.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(i64),
    Word(String),
    Symbol(char),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Number(n) => write!(f, "{n}"),
            Token::Word(w) => write!(f, "{w}"),
            Token::Symbol(c) => write!(f, "{c}"),
        }
    }
}

/// Splits `input` into tokens.
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c.is_ascii_digit() {
            let mut n = c.to_digit(10).unwrap() as i64;
            while let Some(d) = chars.peek().and_then(|d| d.to_digit(10)) {
                n = n * 10 + d as i64;
                chars.next();
            }
            tokens.push(Token::Number(n));
        } else if c.is_alphabetic() {
            let mut w = String::from(c);
            while let Some(&d) = chars.peek() {
                if !d.is_alphanumeric() {
                    break;
                }
                w.push(d);
                chars.next();
            }
            tokens.push(Token::Word(w));
        } else {
            tokens.push(Token::Symbol(c));
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_and_numbers() {
        assert_eq!(tokenize("a 1").len(), 2);
    }
}
"#;
    let new_lib = old_lib
        .replace("//! A tiny tokenizer.", "//! A tiny tokenizer with spans.")
        .replace(
            "    Symbol(char),\n}",
            "    Symbol(char),\n    /// Text in double quotes, without them.\n    Quoted(String),\n}",
        )
        .replace(
            "            Token::Symbol(c) => write!(f, \"{c}\"),\n",
            "            Token::Symbol(c) => write!(f, \"{c}\"),\n            Token::Quoted(q) => write!(f, \"\\\"{q}\\\"\"),\n",
        )
        .replace("let mut n = c.to_digit(10).unwrap() as i64;", "let mut n = i64::from(c.to_digit(10).unwrap_or(0));")
        .replace("n = n * 10 + d as i64;", "n = n * 10 + i64::from(d);")
        .replace(
            "        } else {\n            tokens.push(Token::Symbol(c));",
            "        } else if c == '\"' {\n            let quoted: String = chars.by_ref().take_while(|&d| d != '\"').collect();\n            tokens.push(Token::Quoted(quoted));\n        } else {\n            tokens.push(Token::Symbol(c));",
        )
        .replace("assert_eq!(tokenize(\"a 1\").len(), 2);", "assert_eq!(tokenize(\"a 1 \\\"x y\\\"\").len(), 3);");
    let old_readme = "# tok\n\nSplits text into tokens.\n\nUsage: call `tokenize`.\n";
    let new_readme = "# tok\n\nSplits text into tokens: numbers, words, symbols, and quoted text.\n\nUsage: call `tokenize` — it never fails.\n";
    let mut doc = diff_texts(
        Some("src/lib.rs"),
        Some("src/lib.rs"),
        Some(old_lib),
        Some(&new_lib),
        3,
    );
    doc.append(diff_texts(
        Some("README"),
        Some("README.md"),
        Some(old_readme),
        Some(new_readme),
        3,
    ));
    doc.append(diff_texts(
        None,
        Some("src/span.rs"),
        None,
        Some("/// A byte range of the input.\npub type Span = std::ops::Range<usize>;\n"),
        3,
    ));
    doc.append(diff_texts(
        Some("TODO"),
        None,
        Some("quoted strings\n"),
        None,
        3,
    ));
    // A change with no text: the view shows it as a metadata row.
    let mode_only = "diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n";
    doc.append(parse_unified(mode_only).expect("valid mode-only patch"));
    doc
}

fn load(args: &[String]) -> std::io::Result<(DiffDocument, String)> {
    match args {
        [old, new] => {
            let (a, b) = (std::fs::read_to_string(old)?, std::fs::read_to_string(new)?);
            let doc = diff_texts(Some(old), Some(new), Some(&a), Some(&b), 3);
            Ok((doc, format!("{old} \u{2192} {new}")))
        }
        [patch] => {
            let text = std::fs::read_to_string(patch)?;
            let doc = parse_unified(&text)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            Ok((doc, patch.clone()))
        }
        _ => Ok((sample(), "Sample changes".to_owned())),
    }
}

/// Grammars from the pack root `$DIFFY_SYNTAX_PACKS`, or the one
/// `cargo run -p syntax-pack -- build` writes (`target/syntax-packs`).
#[cfg(feature = "syntax")]
fn grammar_store() -> diffy_syntax::GrammarStore {
    use diffy_syntax::{GrammarStore, StoreConfig};
    let root = std::env::var_os("DIFFY_SYNTAX_PACKS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/syntax-packs")
        });
    let config = StoreConfig::new().local_packs(root);
    // With `syntax-download`, grammars missing locally come from quark's
    // pack host, verified against its index key.
    #[cfg(feature = "syntax-download")]
    let config = {
        use diffy_syntax::{Downloads, PublicKey};
        const INDEX: &str = "https://quark.seated.ro/v1/{target}/index.json";
        const KEY: &str = "2194429b3227f613ac19401deddbb0bdc2b4b283e1ecec0d0d38892c28d63955";
        let key = PublicKey::from_hex(KEY).expect("valid pack index key");
        config.downloads(Downloads::new("diffy-demo", INDEX, &[key]))
    };
    GrammarStore::new(config)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (doc, title) = load(&args)?;
    #[cfg_attr(not(feature = "syntax"), allow(unused_mut))]
    let mut demo = Demo::new(doc, title);
    #[cfg(feature = "syntax")]
    demo.diff.enable_syntax(grammar_store());
    quark_app::run_ui(
        demo,
        WindowOptions {
            title: "Diff".into(),
            size: (1100.0, 720.0),
            ..WindowOptions::default()
        },
    )?;
    Ok(())
}

/// The view driven headlessly the way a user drives it: drags, clicks,
/// and keys on lines found by role and name.
#[cfg(test)]
mod tests {
    use accesskit::Role;
    use diffy_ui::CopySide;
    use diffy_ui::diff_view::prepared::{PreparedKind, SearchMark, WordDetail};
    use diffy_ui::diff_view::presentation::{DiffLayout, DiffPresentation};
    use diffy_ui::diff_view::{
        AnnotationId, CopyContent, DiffAnchor, DiffAnnotation, DiffPreviewLimit, DiffTarget,
        FileId, FindOptions, RevealAlign, Revision, SearchCoverage, SearchDirection, SearchSides,
        SourcePoint,
    };
    use quark_app::quark_ui::test_alloc::{self, Counting};
    use quark_app::testing::{By, UiTestHarness};
    use std::sync::Arc;

    use diffy_diff::{
        DiffSession, DiffUpdate, FileDiffSnapshot, InlineDetail, RowKind, Side, SourceRemap,
        UpdateError,
    };
    use diffy_ui::diff_view::{DiffSessionViewState, diff_session_view};

    use super::*;

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    /// Narrow enough that the file list is hidden.
    const SIZE: (f32, f32) = (600.0, 400.0);

    fn harness(doc: DiffDocument) -> UiTestHarness<Demo> {
        UiTestHarness::new(Demo::new(doc, "test".into()), SIZE, 1.0)
    }

    fn line(name: &str) -> By {
        By::role_name(Role::ListItem, name)
    }

    fn numbered(lines: std::ops::Range<u32>) -> String {
        lines.map(|i| format!("line {i}\n")).collect()
    }

    /// Lines 0..60 with lines 10 and 40 changed, one line of context: two
    /// hunks with a collapsed gap between them.
    fn two_hunks() -> DiffDocument {
        let old = numbered(0..60);
        let new = old
            .replace("line 10\n", "line ten\n")
            .replace("line 40\n", "line forty\n");
        diff_texts(Some("f.txt"), Some("f.txt"), Some(&old), Some(&new), 1)
    }

    #[test]
    fn a_hundred_thousand_line_file_builds_only_the_window() {
        let lines = numbered(0..100_000);
        let doc = diff_texts(None, Some("big.txt"), None, Some(&lines), 3);
        let mut ui = harness(doc);
        // 364 points of body, 20-point lines, eight lines of overscan below.
        let built = ui.find_all(By::role(Role::ListItem)).len();
        assert!((18..=30).contains(&built), "{built} lines built");
        ui.click_node(line("line 3"));
        ui.key("end");
        let items = ui.find_all(By::role(Role::ListItem));
        assert!(items.len() <= 30, "{} lines built", items.len());
        assert_eq!(items.last().unwrap().name.as_deref(), Some("line 99999"));
    }

    #[test]
    fn side_by_side_puts_changed_lines_on_one_row_and_pads_the_shorter_side() {
        let doc = diff_texts(
            Some("f"),
            Some("f"),
            Some("keep\nold a\nold b\nend\n"),
            Some("keep\nnew a\nnew b\nnew c\nend\n"),
            3,
        );
        let mut ui = harness(doc);
        ui.app_mut().diff.set_mode(Mode::Split);
        ui.frame();
        let y = |name: &str| ui.find(line(name)).bounds.y;
        let x = |name: &str| ui.find(line(name)).bounds.x;
        assert_eq!(y("old a"), y("new a"));
        assert_eq!(y("old b"), y("new b"));
        assert!(x("old a") < x("new a"));
        // Unchanged lines show on both sides, level with each other.
        let keeps = ui.find_all(line("end"));
        assert_eq!(keeps.len(), 2);
        assert_eq!(keeps[0].bounds.y, keeps[1].bounds.y);
        // "new c" has no old line beside it: "end" sits one row lower.
        assert!(y("new c") < keeps[0].bounds.y);
        assert!(
            ui.find_all(By::role(Role::ListItem))
                .iter()
                .all(|n| n.bounds.y != y("new c") || n.name.as_deref() == Some("new c"))
        );
    }

    /// Clicks the `n`th button named `name`, in tree order.
    fn click_nth(ui: &mut UiTestHarness<Demo>, name: &str, n: usize) {
        let at = ui.find_all(By::role_name(Role::Button, name))[n].center();
        ui.click(at);
    }

    #[test]
    fn expand_controls_reveal_hidden_lines() {
        let mut ui = harness(two_hunks());
        assert!(ui.try_find(line("line 25")).is_none());
        // The gap between the hunks hides lines 12 to 38.
        click_nth(&mut ui, "Show 20 more lines below the previous change", 1);
        assert!(ui.try_find(line("line 25")).is_some());
        // Of 27 hidden lines, 7 are left.
        ui.app_mut().diff.scroll_to_row(10);
        ui.frame();
        let gap = By::role_name(Role::Label, "7 unchanged lines    @@ -40,3 +40,3 @@");
        let label_y = ui.find(gap.clone()).bounds.y;
        let all = ui
            .find_all(By::role_name(Role::Button, "Show all unchanged lines"))
            .into_iter()
            .find(|b| (b.bounds.y - label_y).abs() < 20.0)
            .unwrap();
        ui.click(all.center());
        assert!(ui.try_find(gap).is_none());
        ui.app_mut().diff.scroll_to_row(30);
        ui.frame();
        assert!(ui.try_find(line("line 37")).is_some());
    }

    #[test]
    fn dragging_across_hunks_copies_the_new_text() {
        let mut ui = harness(two_hunks());
        let from = ui.find(line("line 9")).bounds;
        let to = ui.find(line("line forty")).bounds;
        ui.drag(
            (from.x + 1.0, from.y + 5.0),
            (to.x + to.width - 1.0, to.y + 5.0),
        );
        ui.key("ctrl+c");
        // Removed lines and hidden lines are not copied.
        assert_eq!(
            ui.clipboard_text().as_deref(),
            Some("line 9\nline ten\nline 11\nline 39\nline forty")
        );
    }

    // Selection drags past the view's top or bottom edge scroll it on the
    // clock, with the selection end following, until release.
    #[test]
    fn a_drag_held_past_an_edge_scrolls_until_release() {
        let lines = numbered(0..400);
        let cases = [
            ("top", TOOLBAR_H - 30.0, -1.0),
            ("bottom", SIZE.1 + 30.0, 1.0),
        ];
        for (edge, pointer_y, direction) in cases {
            let mut ui = harness(diff_texts(None, Some("f"), None, Some(&lines), 3));
            ui.app_mut().diff.scroll_to_row(200);
            ui.frame();
            let pressed = ui.find(line("line 205")).center();
            ui.pointer_down(pressed);
            ui.pointer_move((pressed.0, pointer_y));
            let start = ui.app().diff.scroll_offset();
            ui.advance(500);
            let held = ui.app().diff.scroll_offset();
            ui.pointer_up((pressed.0, pointer_y));
            ui.advance(500);

            assert!(
                (held - start) * direction > 50.0,
                "{edge}: {start} -> {held}"
            );
            assert_eq!(
                ui.app().diff.scroll_offset(),
                held,
                "{edge}: stopped on release"
            );
            ui.key("ctrl+c");
            let copied = ui.clipboard_text().unwrap_or_default().lines().count();
            assert!(
                copied > 10,
                "{edge}: the selection followed, {copied} lines"
            );
        }
    }

    #[test]
    fn side_by_side_copies_the_side_the_drag_started_on() {
        let mut ui = harness(two_hunks());
        ui.app_mut().diff.set_mode(Mode::Split);
        ui.frame();
        let olds = ui.find_all(line("line 9"));
        let from = olds[0].bounds;
        let to = ui.find_all(line("line 11"))[0].bounds;
        ui.drag(
            (from.x + 1.0, from.y + 5.0),
            (to.x + to.width - 1.0, to.y + 5.0),
        );
        ui.key("ctrl+c");
        assert_eq!(
            ui.clipboard_text().as_deref(),
            Some("line 9\nline 10\nline 11")
        );
    }

    #[test]
    fn hunk_keys_step_between_hunks() {
        let lines = numbered(0..600);
        let mut new = lines.clone();
        for n in [100, 200, 300, 400, 500] {
            new = new.replace(&format!("line {n}\n"), &format!("changed {n}\n"));
        }
        let mut ui = harness(diff_texts(
            Some("f"),
            Some("f"),
            Some(&lines),
            Some(&new),
            3,
        ));
        ui.click_node(line("line 98"));
        let top_kind = |ui: &UiTestHarness<Demo>| {
            let state = &ui.app().diff;
            let row = state.top_row().unwrap();
            let p = state.projection();
            (p.kind[row as usize], p.hunk.get(row as usize + 1).copied())
        };
        // Each hunk starts at the collapsed gap above it, under the file
        // header.
        ui.key("n");
        assert_eq!(top_kind(&ui), (RowKind::Gap, Some(0)));
        ui.key("n");
        assert_eq!(top_kind(&ui), (RowKind::Gap, Some(1)));
        assert!(ui.try_find(line("changed 200")).is_some());
        ui.key("alt+arrowdown");
        assert_eq!(top_kind(&ui), (RowKind::Gap, Some(2)));
        ui.key("p");
        assert_eq!(top_kind(&ui), (RowKind::Gap, Some(1)));
    }

    fn long_lines() -> DiffDocument {
        let long = "word ".repeat(80);
        let old = format!("short\nold {long}\n");
        let new = format!("short\nnew {long}\n");
        diff_texts(Some("f"), Some("f"), Some(&old), Some(&new), 3)
    }

    #[test]
    fn each_side_scrolls_sideways_on_its_own() {
        let mut ui = harness(long_lines());
        ui.app_mut().diff.set_mode(Mode::Split);
        ui.frame();
        let old = ui.find_all(line("short"))[0].bounds;
        ui.pointer_move((old.x + 20.0, old.y + 5.0));
        ui.wheel(120.0, 0.0);
        ui.frame();
        let offset = |side| ui.app().diff.horizontal_scroll(side).offset().0;
        assert!(offset(diffy_diff::Side::Old) > 0.0);
        assert_eq!(offset(diffy_diff::Side::New), 0.0);
    }

    #[test]
    fn wrapped_lines_grow_their_row() {
        let mut ui = harness(long_lines());
        let style = ui.app().diff.style();
        ui.app_mut().diff.set_style(DiffStyle {
            wrap: true,
            ..style
        });
        ui.frame();
        let short = ui.find(line("short")).bounds.height;
        let long = ui
            .find_all(By::role(Role::ListItem))
            .into_iter()
            .find(|n| n.name.as_deref().is_some_and(|n| n.starts_with("new word")))
            .unwrap();
        assert!(
            long.bounds.height >= short * 3.0,
            "{} vs {short}",
            long.bounds.height
        );
    }

    /// A frame that repeats the last one replays the view from the element
    /// cache: side by side without a screen reader, and unified with a
    /// search match, an annotation, and the accessibility tree on.
    #[test]
    fn a_repeated_frame_allocates_nothing() {
        let lines = numbered(0..100_000);
        let new = lines.replace("line 5\n", "line five\n");
        let doc = || diff_texts(Some("f"), Some("f"), Some(&lines), Some(&new), 3);
        // (mode, accessibility, decorated)
        for (mode, accessible, decorated) in
            [(Mode::Split, false, false), (Mode::Unified, true, true)]
        {
            let mut ui = harness(doc());
            ui.set_accessibility_active(accessible);
            let diff = &mut ui.app_mut().diff;
            diff.set_mode(mode);
            if decorated {
                diff.set_find_query("line 3", FindOptions::default());
                diff.next_match(SearchDirection::Forward);
                diff.set_annotations(vec![note(1, Side::New, 4)]);
            }
            for _ in 0..3 {
                ui.frame();
            }
            let ((), sites) = test_alloc::profile(|| {
                ui.frame();
            });
            assert!(sites.is_empty(), "{mode:?} {accessible}: {sites:#?}");
        }
    }

    // The session view replays a repeated frame the same way.
    #[test]
    fn a_repeated_session_frame_allocates_nothing() {
        let (a, _, _) = revisions(None);
        let mut ui = session_ui(vec![a, snap(2, 1, "x\n", "y\n")]);
        for _ in 0..3 {
            ui.frame();
        }
        let ((), sites) = test_alloc::profile(|| {
            ui.frame();
        });
        assert!(sites.is_empty(), "{sites:#?}");
    }

    fn names(ui: &UiTestHarness<Demo>, role: Role) -> Vec<String> {
        ui.find_all(By::role(role))
            .into_iter()
            .filter_map(|n| n.name)
            .collect()
    }

    fn has_label(ui: &UiTestHarness<Demo>, label: &str) -> bool {
        names(ui, Role::Label).iter().any(|n| n == label)
    }

    const FILE: FileId = FileId(0);

    // Catches exact copy losing line endings or the final-newline state,
    // or claiming a whole file the view holds only as patch lines.
    #[test]
    fn whole_file_copy_is_exact_and_refused_for_patch_lines() {
        let (old, new) = ("a\r\nb\r\nc", "a\r\nB\r\nc");
        let ui = harness(diff_texts(Some("f"), Some("f"), Some(old), Some(new), 3));
        let copy = |ui: &UiTestHarness<Demo>, side| {
            ui.app()
                .diff
                .copy(CopyContent::WholeFile { file: FILE, side })
        };
        assert_eq!(copy(&ui, Side::Old).as_deref(), Some(old));
        assert_eq!(copy(&ui, Side::New).as_deref(), Some(new));
        let patch = harness(parse_unified("--- a/f\n+++ b/f\n@@ -2 +2 @@\n-b\n+B\n").unwrap());
        assert_eq!(copy(&patch, Side::New), None);
    }

    // Catches a split selection's byte offsets being applied to the other
    // side's different text: an endpoint inside a changed line takes that
    // line whole when the other side is copied.
    #[test]
    fn copying_the_other_side_of_a_partial_selection_takes_whole_lines() {
        let doc = diff_texts(
            Some("f"),
            Some("f"),
            Some("keep\nold alpha\nold beta\nend\n"),
            Some("keep\nnew alpha one\nnew beta two\nend\n"),
            3,
        );
        let mut ui = harness(doc);
        ui.app_mut().diff.set_mode(Mode::Split);
        ui.frame();
        let from = ui.find(line("new alpha one")).bounds;
        let to = ui.find(line("new beta two")).bounds;
        ui.drag((from.x + 40.0, from.y + 5.0), (to.x + 40.0, to.y + 5.0));
        let copy = |side| ui.app().diff.copy(CopyContent::Selection(side)).unwrap();

        assert_eq!(copy(CopySide::Old), "old alpha\nold beta");
        assert!(copy(CopySide::New).len() < "new alpha one\nnew beta two".len());
    }

    /// A 1 MiB line of numbered eight-byte cells (`0000000 0000001 ...`)
    /// below a short line, and the long line.
    fn huge_line() -> (DiffDocument, String) {
        let long: String = (0..1u32 << 17).map(|i| format!("{i:07} ")).collect();
        let doc = diff_texts(None, Some("f"), None, Some(&format!("short\n{long}\n")), 3);
        (doc, long)
    }

    /// The long line's row label: the text its layout holds.
    fn long_label(ui: &UiTestHarness<Demo>) -> String {
        names(ui, Role::ListItem)
            .into_iter()
            .find(|n| n.len() > 100)
            .unwrap()
    }

    /// Scrolls the new column sideways to `x` and paints the frame that
    /// applies it and the one after.
    fn scroll_sideways(ui: &mut UiTestHarness<Demo>, x: f32) {
        ui.app()
            .diff
            .horizontal_scroll(Side::New)
            .set_offset(x, 0.0);
        ui.frame();
        ui.frame();
    }

    fn char_w(ui: &UiTestHarness<Demo>) -> f32 {
        ui.app().diff.frame().unwrap().metrics.char_w
    }

    /// Where the new side's sideways content is laid out from.
    fn origin(ui: &UiTestHarness<Demo>) -> f32 {
        ui.app().diff.frame().unwrap().content_origin[Side::New as usize] as f32
    }

    // Catches a huge line reaching shaping whole, or shaping only a prefix
    // that scrolling cannot get past: the layout holds the columns around
    // the view, at the start and 900,000 columns in.
    #[test]
    fn a_huge_line_shapes_the_window_in_view() {
        let (doc, long) = huge_line();
        let mut ui = harness(doc);
        let at_start = long_label(&ui);
        let x = 900_000.0 * char_w(&ui);
        scroll_sideways(&mut ui, x);
        let scrolled = long_label(&ui);

        assert_eq!(at_start, long[..512]);
        // Grid steps of 256 columns, one either side of the view.
        let from = (900_000 / 256 - 1) * 256;
        assert!(
            scrolled.starts_with(&long[from..from + 64]),
            "{}",
            &scrolled[..64]
        );
        assert!(scrolled.len() < 1_024, "{}", scrolled.len());
    }

    // Catches a window painted where its bytes are not: the run of the
    // scrolled window starts at its column, less the scroll.
    #[test]
    fn a_window_paints_at_its_column() {
        let (doc, long) = huge_line();
        let mut ui = harness(doc);
        let scroll = 500_000.0 * char_w(&ui);
        scroll_sideways(&mut ui, scroll);
        let window = long_label(&ui);
        let start = long.find(&window).unwrap();
        let run = ui
            .painted_texts()
            .into_iter()
            .find(|t| t.text == window)
            .unwrap();
        let line = ui.find(line("short")).bounds;
        let pad = ui.app().diff.frame().unwrap().metrics.text_pad;

        // The cell's bounds are in the scrolled content, laid out from the
        // column's origin.
        let expected = line.x - origin(&ui) + pad + start as f32 * char_w(&ui);
        assert!(
            (run.bounds.x - expected).abs() < 1.0,
            "{} vs {expected}",
            run.bounds.x
        );
    }

    // Catches hit-testing a scrolled window as if it started the line: a
    // drag over cells 100,000 and 100,001 copies exactly them.
    #[test]
    fn dragging_in_a_scrolled_window_copies_its_source_bytes() {
        let (doc, _) = huge_line();
        let mut ui = harness(doc);
        let w = char_w(&ui);
        let cell = 100_000.0 * 8.0;
        scroll_sideways(&mut ui, (cell - 20.0) * w);
        // Bounds in the scrolled content, laid out from the column's
        // origin: the text starts one pad in.
        let row = ui.find(line("short")).bounds;
        let long_y = row.y + row.height + 5.0;
        let pad = ui.app().diff.frame().unwrap().metrics.text_pad;
        let start = row.x - origin(&ui);
        // Just right of where a column starts.
        let x = |column: f32| start + pad + column * w + 1.0;
        ui.drag((x(cell), long_y), (x(cell + 16.0), long_y));
        ui.key("ctrl+c");

        assert_eq!(ui.clipboard_text().as_deref(), Some("0100000 0100001 "));
    }

    // Catches long pairs losing word highlights (they stopped at 2,000
    // bytes) or computing them on the UI thread: a 160 KB pair shows none
    // at first, then, once the word thread wakes the app, its one changed
    // word on each side.
    #[test]
    fn a_long_pair_gets_its_changed_words_from_the_word_thread() {
        let line: String = (0..20_000u32).map(|i| format!("{i:07} ")).collect();
        let edited = line.replacen("0000003 ", "changed ", 1);
        let doc = diff_texts(Some("f"), Some("f"), Some(&line), Some(&edited), 3);
        let mut demo = Demo::new(doc, "test".into());
        let (woke, wake) = std::sync::mpsc::channel();
        demo.diff.set_syntax_wake(move || {
            let _ = woke.send(());
        });
        let mut ui = UiTestHarness::new(demo, SIZE, 1.0);
        let words = |ui: &UiTestHarness<Demo>| -> Vec<String> {
            let frame = ui.app().diff.frame().unwrap().clone();
            frame
                .rows
                .iter()
                .flat_map(|row| row.paint.sides.iter().flatten())
                .flat_map(|l| {
                    let text = l.layout.text().to_owned();
                    l.words.iter().map(move |w| text[w.clone()].to_owned())
                })
                .collect()
        };
        let before = (words(&ui), word_details(&ui));
        wake.recv_timeout(std::time::Duration::from_secs(60))
            .expect("the word thread finishes");
        ui.frame();

        assert_eq!(before, (vec![], vec![WordDetail::Pending; 2]));
        assert_eq!(
            (words(&ui), word_details(&ui)),
            (
                vec!["0000003".to_owned(), "changed".to_owned()],
                vec![WordDetail::Done(InlineDetail::Exact); 2]
            )
        );
    }

    /// The word detail of each changed line in the frame, in order.
    fn word_details(ui: &UiTestHarness<Demo>) -> Vec<WordDetail> {
        let frame = ui.app().diff.frame().unwrap().clone();
        frame
            .rows
            .iter()
            .flat_map(|row| row.paint.sides.iter().flatten())
            .map(|l| l.word_detail)
            .filter(|d| *d != WordDetail::Unpaired)
            .collect()
    }

    // Catches a lowered word limit dropping highlights silently: a pair
    // past it says its detail is limited.
    #[test]
    fn a_lowered_word_limit_reports_limited_detail() {
        let doc = diff_texts(
            Some("f"),
            Some("f"),
            Some("one two\n"),
            Some("one three\n"),
            3,
        );
        let mut ui = harness(doc);
        let limits = ui.app().diff.limits();
        ui.app_mut().diff.set_limits(diffy_diff::DiffLimits {
            inline_line_bytes: 4,
            ..limits
        });
        ui.frame();

        assert_eq!(
            word_details(&ui),
            [WordDetail::Done(InlineDetail::Limited); 2]
        );
    }

    // Catches rows positioned from their own offsets 70 million points
    // down, where f32 rounds to multiples of eight: after jumping to the
    // end of 3.5 million lines, every row starts where the one above it
    // ends.
    #[test]
    fn rows_far_down_a_long_diff_stack_without_gaps() {
        let lines: String = (0..3_500_000).map(|i| format!("{i}\n")).collect();
        let doc = diff_texts(None, Some("big.txt"), None, Some(&lines), 3);
        let mut ui = harness(doc);
        ui.click_node(line("3"));
        ui.key("end");
        let frame = ui.app().diff.frame().unwrap().clone();
        let gaps: Vec<f32> = frame
            .rows
            .windows(2)
            .map(|w| w[1].top - (w[0].top + w[0].height))
            .filter(|gap| *gap != 0.0)
            .collect();

        assert!(frame.scroll > 67_108_864.0, "{}", frame.scroll);
        assert_eq!(gaps, Vec::<f32>::new());
    }

    // Catches select-all copying the shaped part of a huge line: it copies
    // the whole line, and whole-line copy reads it exactly.
    #[test]
    fn a_huge_line_copies_whole() {
        let (doc, long) = huge_line();
        let mut ui = harness(doc);
        ui.click_node(line("short"));
        ui.key("ctrl+a");
        ui.key("ctrl+c");
        let selected = ui.clipboard_text().unwrap();
        let full = ui.app().diff.copy(CopyContent::Line {
            file: FILE,
            side: Side::New,
            line: 1,
        });

        assert_eq!(selected, format!("short\n{long}"));
        assert_eq!(full.as_deref(), Some(long.as_str()));
    }

    // Catches a search hit far along a long line staying out of view (or
    // marked at window bytes as if they were line bytes): moving to it
    // scrolls the line there and marks exactly the hit.
    #[test]
    fn a_search_hit_far_along_a_long_line_scrolls_into_view_and_is_marked() {
        let (doc, _) = huge_line();
        let mut ui = harness(doc);
        ui.app_mut()
            .diff
            .set_find_query("0123456 ", FindOptions::default());
        ui.app_mut().diff.next_match(SearchDirection::Forward);
        ui.frame();
        ui.frame();
        let frame = ui.app().diff.frame().unwrap().clone();
        let marked: Vec<String> = frame
            .rows
            .iter()
            .flat_map(|row| {
                let text = row.paint.sides[Side::New as usize]
                    .as_ref()
                    .map(|l| l.layout.text().to_owned());
                row.search[Side::New as usize]
                    .iter()
                    .map(move |m| text.as_deref().unwrap_or("")[m.range.clone()].to_owned())
            })
            .collect();

        assert_eq!(marked, ["0123456 "]);
        let scrolled = ui.app().diff.horizontal_scroll(Side::New).offset().0;
        assert!(
            scrolled > 123_456.0 * 8.0 * char_w(&ui) - 600.0,
            "{scrolled}"
        );
    }

    // Catches mode, binary, rename-only, and final-newline changes showing
    // as an empty file with nothing but `+0 -0`.
    #[test]
    fn metadata_only_changes_show_a_fact_row() {
        let patch = "diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
diff --git a/logo.png b/logo.png
Binary files a/logo.png and b/logo.png differ
diff --git a/old.txt b/new.txt
similarity index 100%
rename from old.txt
rename to new.txt
diff --git a/f.txt b/f.txt
--- a/f.txt
+++ b/f.txt
@@ -1 +1 @@
-a
+b
\\ No newline at end of file
";
        let ui = harness(parse_unified(patch).unwrap());
        for fact in [
            "File mode changed from 100644 to 100755",
            "Binary file not shown",
            "Renamed without changes",
            "No newline at end of the new file",
        ] {
            assert!(
                has_label(&ui, fact),
                "{fact}: {:?}",
                names(&ui, Role::Label)
            );
        }
    }

    /// Lines 0..200 with lines 10, 100, and 190 changed, one line of
    /// context: gaps between the three hunks.
    fn three_hunks() -> DiffDocument {
        let old = numbered(0..200);
        let mut new = old.clone();
        for n in [10, 100, 190] {
            new = new.replace(&format!("line {n}\n"), &format!("changed {n}\n"));
        }
        diff_texts(Some("f"), Some("f"), Some(&old), Some(&new), 1)
    }

    // Catches revealing a hidden line scrolling to nothing, or expanding
    // more collapsed context than the target needs.
    #[test]
    fn revealing_a_hidden_line_expands_only_its_gap() {
        let mut ui = harness(three_hunks());
        let target = DiffTarget::Source(SourcePoint {
            file: FILE,
            side: Side::New,
            line: 50,
            byte: 0,
        });
        assert!(ui.app_mut().diff.reveal_target(target, RevealAlign::Center));
        ui.frame();
        assert!(ui.try_find(line("line 50")).is_some());
        ui.key("end");
        assert!(has_label(&ui, "87 unchanged lines    @@ -190,3 +190,3 @@"));
    }

    // Catches search reading only rendered rows, or moving to a match
    // without exposing it inside collapsed context.
    #[test]
    fn search_finds_and_reveals_a_match_in_collapsed_context() {
        let mut ui = harness(two_hunks());
        ui.app_mut()
            .diff
            .set_find_query("LINE 25", FindOptions::default());
        assert_eq!(ui.app().diff.search_summary().matches, 1);
        let at = ui.app_mut().diff.next_match(SearchDirection::Forward);
        ui.frame();

        assert_eq!(at.map(|p| (p.line, p.byte)), Some((25, 0)));
        assert!(ui.try_find(line("line 25")).is_some());
        let frame = ui.app().diff.frame().unwrap().clone();
        let row = frame
            .rows
            .iter()
            .find(|r| r.paint.source_lines[1] == Some(25))
            .unwrap();
        assert_eq!(
            row.search[1],
            [SearchMark {
                range: 0..7,
                active: true
            }]
        );
    }

    // Catches side scope, case, or the unchanged-line option counting the
    // wrong lines, and a patch's partial coverage going unreported.
    #[test]
    fn search_options_scope_the_matches() {
        let doc = || {
            diff_texts(
                Some("f"),
                Some("f"),
                Some("Alpha\nbeta\nalpha\n"),
                Some("Alpha\nBETA two\nalpha\n"),
                3,
            )
        };
        let mut ui = harness(doc());
        let opts = |sides, case_sensitive, include_unchanged| FindOptions {
            sides,
            case_sensitive,
            include_unchanged,
            ..FindOptions::default()
        };
        // query, options, (old matches, new matches)
        let cases = [
            ("alpha", opts(SearchSides::New, false, true), (0, 2)),
            ("alpha", opts(SearchSides::New, true, true), (0, 1)),
            ("alpha", opts(SearchSides::Both, false, true), (2, 2)),
            ("alpha", opts(SearchSides::Both, false, false), (0, 0)),
            ("beta", opts(SearchSides::Both, false, false), (1, 1)),
            ("beta", opts(SearchSides::Old, true, true), (1, 0)),
        ];
        for (query, options, expected) in cases {
            ui.app_mut().diff.set_find_query(query, options);
            let s = ui.app().diff.search_summary();
            assert_eq!(
                (s.old_matches, s.new_matches),
                expected,
                "{query} {options:?}"
            );
            assert_eq!(s.coverage, SearchCoverage::Full);
        }
        let mut patch = harness(parse_unified("--- a/f\n+++ b/f\n@@ -2 +2 @@\n-b\n+B\n").unwrap());
        patch
            .app_mut()
            .diff
            .set_find_query("b", FindOptions::default());
        assert_eq!(
            patch.app().diff.search_summary().coverage,
            SearchCoverage::PatchOnly
        );
    }

    // Catches a compact preview materializing the whole diff, or its open
    // action losing the first row it left out.
    #[test]
    fn a_preview_shows_twelve_rows_and_opens_at_the_first_hidden_line() {
        let lines = numbered(0..100);
        let mut ui = harness(diff_texts(None, Some("big.txt"), None, Some(&lines), 3));
        ui.app_mut()
            .diff
            .set_preview_limit(Some(DiffPreviewLimit::default()));
        ui.frame();
        // The file header and eleven lines.
        assert_eq!(names(&ui, Role::ListItem).len(), 11);
        assert!(
            has_label(&ui, "89 more rows"),
            "{:?}",
            names(&ui, Role::Label)
        );
        assert!(ui.app().diff.content_height() < SIZE.1 - TOOLBAR_H);
        let outcome = ui.app_mut().diff.handle(DiffEvent::OpenFull);
        let target = DiffTarget::Source(SourcePoint {
            file: FILE,
            side: Side::New,
            line: 11,
            byte: 0,
        });
        assert_eq!(outcome, DiffOutcome::OpenFull { target });
    }

    /// The frame's rows in order: line rows as their text, annotation
    /// rows as `note <side> <first line number>`.
    fn frame_rows(ui: &UiTestHarness<Demo>) -> Vec<String> {
        let frame = ui.app().diff.frame().unwrap().clone();
        frame
            .rows
            .iter()
            .filter_map(|r| match &r.paint.kind {
                PreparedKind::Annotation(slot) => {
                    let side = if slot.side == Side::Old { "old" } else { "new" };
                    Some(format!("note {side} {}", slot.lines.start + 1))
                }
                _ => r
                    .paint
                    .sides
                    .iter()
                    .flatten()
                    .next()
                    .map(|l| l.layout.text().to_owned()),
            })
            .collect()
    }

    fn note(id: u64, side: Side, line: u32) -> DiffAnnotation {
        DiffAnnotation {
            id: AnnotationId(id),
            anchor: DiffAnchor {
                file: FILE,
                revision: Revision(0),
                side,
                lines: line..line + 1,
            },
            revision: 0,
        }
    }

    // Catches an annotation's measured height moving the code above it or
    // the top of the viewport.
    #[test]
    fn a_measured_annotation_grows_without_moving_the_code_above() {
        let lines = numbered(0..100);
        let mut ui = harness(diff_texts(None, Some("f"), None, Some(&lines), 3));
        ui.app_mut()
            .diff
            .set_annotations(vec![note(1, Side::New, 8)]);
        ui.app_mut().diff.scroll_to_row(3);
        ui.frame();
        let y = |ui: &UiTestHarness<Demo>, name| ui.find(line(name)).bounds.y;
        let (top, anchor, below) = (y(&ui, "line 3"), y(&ui, "line 8"), y(&ui, "line 9"));
        ui.app_mut().diff.handle(DiffEvent::AnnotationMeasured {
            id: AnnotationId(1),
            revision: 0,
            height: 140.0,
        });
        ui.frame();

        assert_eq!((y(&ui, "line 3"), y(&ui, "line 8")), (top, anchor));
        assert_eq!(y(&ui, "line 9") - below, 140.0 - 60.0);
    }

    // Catches annotations attaching to the wrong side's line: each sits
    // below its own side's row and names its side.
    #[test]
    fn annotation_rows_sit_below_their_sides_line() {
        let mut ui = harness(two_hunks());
        ui.app_mut()
            .diff
            .set_annotations(vec![note(1, Side::Old, 10), note(2, Side::New, 10)]);
        ui.frame();
        let rows = frame_rows(&ui);
        let at = |name: &str| rows.iter().position(|r| r == name).unwrap();
        assert_eq!(at("note old 11"), at("line 10") + 1);
        assert_eq!(at("note new 11"), at("line ten") + 1);
    }

    // Catches an annotation inside collapsed context vanishing: the fold
    // counts it, and revealing its line opens it.
    #[test]
    fn an_annotation_in_collapsed_context_is_counted_and_revealed() {
        let mut ui = harness(two_hunks());
        ui.app_mut()
            .diff
            .set_annotations(vec![note(1, Side::New, 25)]);
        ui.frame();
        assert!(has_label(
            &ui,
            "27 unchanged lines, 1 annotation    @@ -40,3 +40,3 @@"
        ));
        let target = DiffTarget::Source(SourcePoint {
            file: FILE,
            side: Side::New,
            line: 25,
            byte: 0,
        });
        ui.app_mut().diff.reveal_target(target, RevealAlign::Center);
        ui.frame();
        let rows = frame_rows(&ui);
        let at = |name: &str| rows.iter().position(|r| r == name);
        assert_eq!(at("note new 26"), at("line 25").map(|i| i + 1));
    }

    // Catches a folded file keeping its rows (or losing its header), and
    // navigation into it leaving it folded.
    #[test]
    fn a_collapsed_file_keeps_only_its_header_until_revealed() {
        let mut ui = harness(two_hunks());
        ui.app_mut().diff.set_file_collapsed(FILE, true);
        ui.frame();
        assert!(names(&ui, Role::ListItem).is_empty());
        assert!(ui.try_find(By::role(Role::Heading)).is_some());
        let target = DiffTarget::Hunk {
            file: FILE,
            hunk: 1,
        };
        ui.app_mut().diff.reveal_target(target, RevealAlign::Top);
        ui.frame();
        assert!(ui.try_find(line("line forty")).is_some());
    }

    // Catches the automatic layout flipping without hysteresis or losing
    // the reader's line when it switches.
    #[test]
    fn automatic_layout_follows_the_width_and_keeps_the_top_line() {
        let mut ui = harness(three_hunks());
        let presentation = DiffPresentation {
            layout: DiffLayout::AUTO,
            ..DiffPresentation::default()
        };
        ui.app_mut().diff.set_presentation(presentation);
        ui.app_mut().diff.scroll_to_row(5);
        ui.frame();
        let before = names(&ui, Role::ListItem)[0].clone();
        let mut seen = Vec::new();
        // 1020 points leaves each side between the 40 columns split keeps
        // and the 44 it needs to start.
        for width in [600.0, 1020.0, 1200.0, 1020.0, 600.0] {
            ui.resize(width, SIZE.1);
            seen.push(ui.app().diff.mode());
            assert_eq!(names(&ui, Role::ListItem)[0], before, "at {width}");
        }
        assert_eq!(
            seen,
            [
                Mode::Unified,
                Mode::Unified,
                Mode::Split,
                Mode::Split,
                Mode::Unified
            ]
        );
    }

    /// The session view in the same window as [`Demo`].
    struct SessionDemo {
        diff: DiffSessionViewState,
    }

    impl UiApp for SessionDemo {
        type Action = Msg;
        type Message = ();

        fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
            let (width, height) = cx.frame.size();
            self.diff.set_viewport(width, height);
            let scale = cx.frame.scale_factor();
            let now_ms = cx.frame.elapsed().as_millis() as u64;
            let text_cx = cx.frame.text();
            self.diff
                .prepare(&mut text_cx.system, &mut text_cx.layouts, scale, now_ms);
            let env = CollectionEnv {
                focused: cx.is_focused(DIFF_FOCUS),
                accessible: cx.frame.accessibility_active(),
            };
            diff_session_view(&mut self.diff, cx.theme, env, |e| Msg::Diff(e).into())
        }

        fn update(&mut self, msg: Msg, cx: &mut UiContext) {
            if let Msg::Diff(event) = msg
                && let DiffOutcome::Copy(copied) = self.diff.handle(event)
            {
                cx.window.set_clipboard_text(&copied);
            }
        }
    }

    /// Revision `rev` of file `id`: `old` against `new`, every line shown.
    fn snap(id: u64, rev: u64, old: &str, new: &str) -> FileDiffSnapshot {
        let path = format!("file{id}.txt");
        let doc = diff_texts(Some(&path), Some(&path), Some(old), Some(new), 100_000);
        FileDiffSnapshot::new(FileId(id), Revision(rev), Arc::new(doc)).unwrap()
    }

    fn upsert(file: FileDiffSnapshot, remap: Option<SourceRemap>) -> DiffUpdate {
        DiffUpdate::Upsert { file, remap }
    }

    fn session_ui(files: Vec<FileDiffSnapshot>) -> UiTestHarness<SessionDemo> {
        let mut session = DiffSession::new();
        for file in files {
            session.apply(upsert(file, None)).unwrap();
        }
        let diff = DiffSessionViewState::new("demo.diff", DIFF_FOCUS, session);
        UiTestHarness::new(SessionDemo { diff }, SIZE, 1.0)
    }

    fn session_lines(ui: &UiTestHarness<SessionDemo>) -> Vec<(String, f32)> {
        ui.find_all(By::role(Role::ListItem))
            .into_iter()
            .map(|n| (n.name.unwrap_or_default(), n.bounds.y))
            .collect()
    }

    /// Scrolls file 1's new line `line` to the top.
    fn scroll_to(ui: &mut UiTestHarness<SessionDemo>, line: u32) {
        let target = DiffTarget::Source(SourcePoint {
            file: FileId(1),
            side: Side::New,
            line,
            byte: 0,
        });
        assert!(ui.app_mut().diff.reveal_target(target, RevealAlign::Top));
        ui.frame();
    }

    /// File 1 revision 1 (line 5 changed) and revision 2 (five lines
    /// inserted at the top, and line `replaced` edited when given).
    fn revisions(replaced: Option<u32>) -> (FileDiffSnapshot, FileDiffSnapshot, SourceRemap) {
        let old = numbered(0..300);
        let first = old.replace("line 5\n", "five\n");
        let mut second = format!("{}{first}", numbered(1000..1005));
        if let Some(n) = replaced {
            second = second.replace(&format!("line {n}\n"), &format!("edited {n}\n"));
        }
        let (a, b) = (snap(1, 1, &old, &first), snap(1, 2, &old, &second));
        let remap = SourceRemap::between(&a, &b).unwrap();
        (a, b, remap)
    }

    // Catches a file arriving before the one being read moving the
    // reader's line or dropping the selection.
    #[test]
    fn a_file_inserted_above_keeps_the_top_line_and_selection() {
        let (a, _, _) = revisions(None);
        let mut ui = session_ui(vec![a]);
        scroll_to(&mut ui, 100);
        let from = ui.find(line("line 101")).bounds;
        let to = ui.find(line("line 102")).bounds;
        ui.drag(
            (from.x + 1.0, from.y + 5.0),
            (to.x + to.width - 1.0, to.y + 5.0),
        );
        let before = (session_lines(&ui)[0].clone(), ui.app().diff.selected_text());

        let b = snap(2, 1, "x\n", "y\n");
        ui.app_mut().diff.apply_update(upsert(b, None)).unwrap();
        let order = DiffUpdate::Order {
            revision: Revision(1),
            files: Arc::from([FileId(2), FileId(1)]),
        };
        ui.app_mut().diff.apply_update(order).unwrap();
        ui.frame();

        let after = (session_lines(&ui)[0].clone(), ui.app().diff.selected_text());
        assert_eq!(after, before);
        assert_eq!(before.1, "line 101\nline 102");
    }

    // Catches an update above the viewport shifting the reader's line: the
    // remap carries the top line to its new position.
    #[test]
    fn an_update_above_the_viewport_keeps_the_top_line() {
        let (a, b, remap) = revisions(None);
        let mut ui = session_ui(vec![a]);
        scroll_to(&mut ui, 200);
        let before = session_lines(&ui)[0].clone();
        ui.app_mut()
            .diff
            .apply_update(upsert(b, Some(remap)))
            .unwrap();
        ui.frame();
        assert_eq!(session_lines(&ui)[0], before);
    }

    // Catches a selection surviving on replaced text, or not following
    // unchanged text to its new lines.
    #[test]
    fn a_selection_follows_unchanged_lines_and_clears_on_replaced_ones() {
        // (line replaced by the update, selection copied after it)
        // An edit inside the selection clears it, ends unchanged or not.
        for (replaced, expected) in [(None, "line 150\nline 151\nline 152"), (Some(151), "")] {
            let (a, b, remap) = revisions(replaced);
            let mut ui = session_ui(vec![a]);
            scroll_to(&mut ui, 148);
            let from = ui.find(line("line 150")).bounds;
            let to = ui.find(line("line 152")).bounds;
            ui.drag(
                (from.x + 1.0, from.y + 5.0),
                (to.x + to.width - 1.0, to.y + 5.0),
            );
            ui.app_mut()
                .diff
                .apply_update(upsert(b, Some(remap)))
                .unwrap();
            ui.frame();
            assert_eq!(ui.app().diff.selected_text(), expected, "{replaced:?}");
        }
    }

    // Catches annotations silently attaching to whatever line now has
    // their old number: unchanged anchors move, replaced ones outdate.
    #[test]
    fn annotations_follow_unchanged_lines_and_outdate_on_replaced_ones() {
        let (a, b, remap) = revisions(Some(3));
        let mut ui = session_ui(vec![a]);
        let note = |id, line: u32| DiffAnnotation {
            id: AnnotationId(id),
            anchor: DiffAnchor {
                file: FileId(1),
                revision: Revision(1),
                side: Side::New,
                lines: line..line + 1,
            },
            revision: 0,
        };
        ui.app_mut()
            .diff
            .set_annotations(vec![note(1, 100), note(2, 3)]);
        ui.app_mut()
            .diff
            .apply_update(upsert(b, Some(remap)))
            .unwrap();
        let state: Vec<_> = ui
            .app()
            .diff
            .annotations()
            .map(|(a, outdated)| (a.anchor.lines.clone(), a.anchor.revision, outdated))
            .collect();
        assert_eq!(
            state,
            [(105..106, Revision(2), false), (3..4, Revision(1), true)]
        );
    }

    // Catches a late upsert resurrecting a removed file.
    #[test]
    fn a_removed_file_refuses_a_late_upsert_and_stays_gone() {
        let mut ui = session_ui(vec![snap(1, 1, "a\n", "b\n"), snap(2, 1, "c\n", "d\n")]);
        let remove = DiffUpdate::Remove {
            file: FileId(2),
            revision: Revision(2),
        };
        ui.app_mut().diff.apply_update(remove).unwrap();
        let late = ui
            .app_mut()
            .diff
            .apply_update(upsert(snap(2, 3, "c\n", "e\n"), None));
        ui.frame();
        assert!(matches!(late, Err(UpdateError::Removed { .. })), "{late:?}");
        let names: Vec<String> = session_lines(&ui).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["a", "b"]);
    }

    // Catches an update to a file off screen reshaping the visible rows.
    #[test]
    fn updating_an_offscreen_file_keeps_visible_rows_shaped() {
        let lines = numbered(0..100);
        let mut ui = session_ui(vec![
            snap(1, 1, &lines, &lines.replace("line 1\n", "one\n")),
            snap(2, 1, "x\n", "y\n"),
        ]);
        let paints = |ui: &UiTestHarness<SessionDemo>| {
            let frame = ui.app().diff.frame().unwrap().clone();
            frame
                .rows
                .iter()
                .map(|r| r.paint.clone())
                .collect::<Vec<_>>()
        };
        let before = paints(&ui);
        ui.app_mut()
            .diff
            .apply_update(upsert(snap(2, 2, "x\n", "z\n"), None))
            .unwrap();
        ui.frame();
        let after = paints(&ui);
        assert_eq!(before.len(), after.len());
        assert!(before.iter().zip(&after).all(|(a, b)| Rc::ptr_eq(a, b)));
    }

    // Catches incremental updates leaving the view different from showing
    // the final files from scratch.
    #[test]
    fn a_sequence_of_updates_converges_to_the_final_input() {
        let (a1, a2) = (
            snap(1, 1, "a\nb\n", "a\nB\n"),
            snap(1, 2, "a\nb\n", "A\nB\nc\n"),
        );
        let (b1, c1) = (snap(2, 1, "x\n", "y\n"), snap(3, 1, "p\n", "q\n"));
        let mut ui = session_ui(vec![a1]);
        for update in [
            upsert(b1.clone(), None),
            upsert(a2.clone(), None),
            DiffUpdate::Order {
                revision: Revision(1),
                files: Arc::from([FileId(2), FileId(1)]),
            },
            upsert(c1, None),
            DiffUpdate::Remove {
                file: FileId(3),
                revision: Revision(2),
            },
        ] {
            ui.app_mut().diff.apply_update(update).unwrap();
        }
        ui.frame();
        let fresh = session_ui(vec![b1, a2]);
        let shown = |ui: &UiTestHarness<SessionDemo>| {
            let names: Vec<String> = session_lines(ui).into_iter().map(|(n, _)| n).collect();
            (names, ui.app().diff.copy(CopyContent::Patch))
        };
        assert_eq!(shown(&ui), shown(&fresh));
    }
}
