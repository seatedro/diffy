//! Presentation choices for the diff view: comparison layout, change
//! markers, line numbers, hunk separators, file headers, and the fill of a
//! missing split side, plus local color overrides for the light and the
//! dark theme.
//!
//! The state stores these and hands them to the renderer through
//! [`super::prepared::ViewFrame`]; the defaults reproduce the view as it
//! was before these options existed (unified, signs, two unified number
//! columns, gap controls, built-in headers, solid filler). Changing a
//! marker, filler, or color choice invalidates paint only; changing the
//! layout or numbers moves columns, so the state re-measures.

use std::hash::{Hash, Hasher};

use diffy_diff::{Mode, RowKind, Side};
use diffy_syntax::HighlightKind;
use quark_ui::design::Alpha;
use quark_ui::theme::{Color, Theme, ThemeMode, contrast_ratio};

use super::prepared::{Columns, Metrics};

/// Which comparison layout to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffLayout {
    Unified,
    Split,
    /// Split while each side keeps at least `min_text_columns` monospace
    /// columns of usable text width, else unified. Switching back needs
    /// `hysteresis_columns` more (or fewer) than the threshold, so a
    /// resize near it does not flip the layout every frame.
    Auto {
        min_text_columns: u16,
        hysteresis_columns: u16,
    },
}

impl DiffLayout {
    /// The recommended automatic layout: 40 columns, four of hysteresis.
    pub const AUTO: Self = Self::Auto {
        min_text_columns: 40,
        hysteresis_columns: 4,
    };
}

/// How a changed line is marked in the gutter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DiffMarkers {
    /// `+` and `-` signs.
    #[default]
    Signs,
    /// A strip at the text edge: solid for added lines, repeated
    /// horizontal segments for removed ones.
    Bars,
    None,
}

/// Which line numbers the gutter shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DiffNumbers {
    /// Unified shows old and new numbers; split shows each side's own.
    #[default]
    Both,
    /// One column: the number of the side the row shows.
    RelevantSide,
    None,
}

/// What sits between hunks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum HunkSeparator {
    /// The `@@` header text.
    Metadata,
    /// The collapsed-gap row with reveal controls and the header text.
    #[default]
    ContextControls,
    /// A thin fold bar with reveal controls and no header text.
    Compact,
}

/// How file headers are drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FileHeaders {
    #[default]
    BuiltIn,
    Hidden,
    /// Drawn by the app's decorator.
    Custom,
}

/// Fill of the side of a split row that has no line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum EmptySideFill {
    #[default]
    Solid,
    Hatch,
}

/// Every presentation choice. See the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiffPresentation {
    pub layout: DiffLayout,
    pub markers: DiffMarkers,
    pub numbers: DiffNumbers,
    pub separators: HunkSeparator,
    pub headers: FileHeaders,
    pub empty_side: EmptySideFill,
    pub sticky_headers: bool,
    pub wrap_column: Option<u16>,
}

impl Default for DiffPresentation {
    fn default() -> Self {
        Self {
            layout: DiffLayout::Unified,
            markers: DiffMarkers::default(),
            numbers: DiffNumbers::default(),
            separators: HunkSeparator::default(),
            headers: FileHeaders::default(),
            empty_side: EmptySideFill::default(),
            sticky_headers: false,
            wrap_column: None,
        }
    }
}

impl DiffPresentation {
    /// An embedded preview, as in a transcript card: unified rows, bar
    /// markers, the relevant side's number only, thin fold bars, and no
    /// file header (the card around it names the file).
    pub const fn compact() -> Self {
        Self {
            layout: DiffLayout::Unified,
            markers: DiffMarkers::Bars,
            numbers: DiffNumbers::RelevantSide,
            separators: HunkSeparator::Compact,
            headers: FileHeaders::Hidden,
            empty_side: EmptySideFill::Hatch,
            sticky_headers: false,
            wrap_column: None,
        }
    }

    /// A review panel: [`DiffLayout::AUTO`], bar markers, one number per
    /// side, gap controls, built-in headers, and hatched missing sides.
    pub const fn review() -> Self {
        Self {
            layout: DiffLayout::AUTO,
            markers: DiffMarkers::Bars,
            numbers: DiffNumbers::RelevantSide,
            separators: HunkSeparator::ContextControls,
            headers: FileHeaders::BuiltIn,
            empty_side: EmptySideFill::Hatch,
            sticky_headers: false,
            wrap_column: None,
        }
    }
}

/// The mode an automatic `layout` wants for a view `width` points wide
/// that shows `current` now, or `None` when `layout` is explicit (the
/// state keeps whatever the app chose).
///
/// Split needs each side's text column, after its gutter and padding, to
/// hold `min_text_columns` monospace columns; entering split from unified
/// needs `hysteresis_columns` more, so a width near the threshold keeps
/// whichever layout it has.
pub fn auto_mode(
    layout: DiffLayout,
    current: Mode,
    width: f32,
    m: &Metrics,
    presentation: &DiffPresentation,
) -> Option<Mode> {
    let DiffLayout::Auto {
        min_text_columns,
        hysteresis_columns,
    } = layout
    else {
        return None;
    };
    let split = Columns::new(Mode::Split, width, m, presentation);
    // The narrower side decides: the old side is a point narrower.
    let text_w = (split.of(Side::Old).text_w - m.text_pad * 2.0).max(0.0);
    let columns = (text_w / m.char_w.max(f32::EPSILON)).floor();
    let needed = match current {
        Mode::Split => f32::from(min_text_columns),
        Mode::Unified => f32::from(min_text_columns) + f32::from(hysteresis_columns),
    };
    Some(if columns >= needed {
        Mode::Split
    } else {
        Mode::Unified
    })
}

/// Height of a header, separator, or line row of `kind` before its text
/// is measured.
pub fn band_height(kind: RowKind, m: &Metrics, presentation: &DiffPresentation) -> f32 {
    match kind {
        RowKind::FileHeader => match presentation.headers {
            FileHeaders::Hidden => 0.0,
            FileHeaders::BuiltIn | FileHeaders::Custom => (m.line_h * 2.0).round(),
        },
        RowKind::HunkHeader | RowKind::Gap if presentation.separators == HunkSeparator::Compact => {
            compact_separator_height(m)
        }
        RowKind::HunkHeader | RowKind::Gap => (m.line_h * 1.4).round(),
        _ => m.line_h,
    }
}

/// A compact fold bar: a third of a line, at least four points.
fn compact_separator_height(m: &Metrics) -> f32 {
    (m.line_h * 0.3).round().max(4.0)
}

pub fn wrap_width(
    columns: &Columns,
    side: Side,
    m: &Metrics,
    presentation: &DiffPresentation,
) -> f32 {
    let width = columns.wrap_width(side, m);
    match presentation.wrap_column {
        Some(column) if column > 0 => width.min(f32::from(column) * m.char_w).max(m.char_w),
        _ => width,
    }
}

/// Colors the app sets over the theme's diff tokens; `None` keeps the
/// theme's. Each applies to every row it names; the view still marks
/// changes by shape (signs, or striped versus solid bars) and in its
/// accessible description, whatever the colors.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DiffColorOverrides {
    /// The view's background behind unchanged lines.
    pub surface: Option<Color>,
    /// Code text without a syntax color, and file header titles.
    pub text: Option<Color>,
    /// Background of added and removed lines, gutter included.
    pub add_line: Option<Color>,
    pub del_line: Option<Color>,
    /// Background of changed words, over the line's.
    pub add_word: Option<Color>,
    pub del_word: Option<Color>,
    /// Bars and signs.
    pub add_marker: Option<Color>,
    pub del_marker: Option<Color>,
    /// Line numbers of added and removed lines.
    pub add_number: Option<Color>,
    pub del_number: Option<Color>,
    pub gutter: Option<Color>,
    pub gutter_text: Option<Color>,
    /// The missing side of a split row, and its hatch lines.
    pub empty_side: Option<Color>,
    pub hatch: Option<Color>,
    pub file_header: Option<Color>,
    /// Gap rows and hunk headers.
    pub separator: Option<Color>,
    pub search_match: Option<Color>,
    pub search_active: Option<Color>,
    pub selection: Option<Color>,
    /// Outline of the row holding the keyboard focus.
    pub focused_row: Option<Color>,
    /// Text of each [`HighlightKind`], indexed by its discriminant.
    pub syntax: [Option<Color>; HIGHLIGHT_KINDS],
}

impl DiffColorOverrides {
    /// Paint `kind` in `color`.
    pub fn with_syntax(mut self, kind: HighlightKind, color: Color) -> Self {
        self.syntax[kind as usize] = Some(color);
        self
    }
}

/// Number of [`HighlightKind`]s.
pub const HIGHLIGHT_KINDS: usize = HighlightKind::Preprocessor as usize + 1;

/// Local colors for the light and the dark theme, resolved independently:
/// the view picks the overrides of the theme's mode.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DiffAppearance {
    pub light: DiffColorOverrides,
    pub dark: DiffColorOverrides,
}

impl DiffAppearance {
    /// The same overrides in both modes.
    pub fn both(overrides: DiffColorOverrides) -> Self {
        Self {
            light: overrides,
            dark: overrides,
        }
    }

    pub fn for_mode(&self, mode: ThemeMode) -> &DiffColorOverrides {
        match mode {
            ThemeMode::Light => &self.light,
            ThemeMode::Dark => &self.dark,
        }
    }
}

/// Every color the view paints with: the theme's diff tokens with the
/// appearance's overrides for the theme's mode applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiffColors {
    pub surface: Color,
    pub empty_side: Color,
    pub hatch: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub gutter: Color,
    pub gutter_text: Color,
    pub add_line: Color,
    pub del_line: Color,
    pub add_word: Color,
    pub del_word: Color,
    pub add_marker: Color,
    pub del_marker: Color,
    pub add_number: Color,
    pub del_number: Color,
    /// Counts in file headers.
    pub add_text: Color,
    pub del_text: Color,
    pub file_header: Color,
    pub separator: Color,
    pub hover: Color,
    pub search_match: Color,
    pub search_active: Color,
    pub search_outline: Color,
    pub selection: Color,
    pub focused_row: Color,
    pub accent: Color,
    pub warning: Color,
    pub strong: Color,
    pub faint: Color,
    pub picked_wash: Color,
    pub picked_gutter: Color,
    pub picked_number: Color,
    pub syntax: [Color; HIGHLIGHT_KINDS],
    /// `syntax` lifted where needed to stay legible on `add_line` and
    /// `del_line`.
    pub syntax_on_add: [Color; HIGHLIGHT_KINDS],
    pub syntax_on_del: [Color; HIGHLIGHT_KINDS],
}

impl DiffColors {
    pub fn resolve(theme: &Theme, appearance: &DiffAppearance) -> Self {
        let c = &theme.colors;
        let o = appearance.for_mode(theme.mode);
        let pick = |over: Option<Color>, base: Color| over.unwrap_or(base);
        let text = pick(o.text, c.text);
        let syntax_base = |kind: HighlightKind| match kind {
            HighlightKind::Keyword | HighlightKind::Preprocessor => c.syntax_keyword,
            HighlightKind::String => c.syntax_string,
            HighlightKind::Comment => c.syntax_comment,
            HighlightKind::Function => c.syntax_function,
            HighlightKind::Type | HighlightKind::Namespace => c.syntax_type,
            HighlightKind::Number | HighlightKind::Constant | HighlightKind::Builtin => {
                c.syntax_number
            }
            HighlightKind::Property
            | HighlightKind::Attribute
            | HighlightKind::Tag
            | HighlightKind::Label => c.syntax_property,
            HighlightKind::Operator => c.syntax_operator,
            HighlightKind::Normal | HighlightKind::Punctuation | HighlightKind::Variable => text,
        };
        let mut syntax = [text; HIGHLIGHT_KINDS];
        for kind in ALL_KINDS {
            syntax[kind as usize] = pick(o.syntax[kind as usize], syntax_base(kind));
        }
        let gutter_text = pick(o.gutter_text, c.gutter_text);
        let surface = pick(o.surface, c.editor_surface);
        let gutter = pick(o.gutter, surface);
        let add_marker = pick(o.add_marker, c.line_add_text);
        let del_marker = pick(o.del_marker, c.line_del_text);
        let add_line = pick(o.add_line, c.line_add);
        let del_line = pick(o.del_line, c.line_del);
        let on = |line: Color| syntax.map(|tone| legible(tone, opaque(line, surface), text));
        let (syntax_on_add, syntax_on_del) = (on(add_line), on(del_line));
        Self {
            surface,
            empty_side: pick(o.empty_side, c.background),
            hatch: pick(o.hatch, c.border_variant),
            text,
            muted: c.text_muted,
            border: c.border_variant,
            gutter,
            gutter_text,
            add_line,
            del_line,
            add_word: pick(o.add_word, c.line_add_word_bg),
            del_word: pick(o.del_word, c.line_del_word_bg),
            add_marker,
            del_marker,
            add_number: pick(o.add_number, gutter_text.lerp(add_marker, NUMBER_TINT)),
            del_number: pick(o.del_number, gutter_text.lerp(del_marker, NUMBER_TINT)),
            add_text: c.line_add_text,
            del_text: c.line_del_text,
            file_header: pick(o.file_header, c.file_header_bg),
            separator: pick(o.separator, surface.lerp(text, SEPARATOR_LIFT)),
            hover: c.ghost_element_hover,
            search_match: pick(o.search_match, c.search_match_bg),
            search_active: pick(o.search_active, c.search_match_active_bg),
            search_outline: c.focus_border,
            selection: pick(o.selection, c.accent.with_alpha(Alpha::SOFT)),
            focused_row: pick(o.focused_row, c.focus_border),
            accent: c.accent,
            warning: c.status_warning,
            strong: c.text_strong,
            faint: c.text_disabled,
            picked_wash: surface.lerp(c.accent, PICKED_WASH),
            picked_gutter: gutter.lerp(c.accent, PICKED_GUTTER),
            picked_number: gutter_text.lerp(c.accent, PICKED_NUMBER),
            syntax,
            syntax_on_add,
            syntax_on_del,
        }
    }

    /// Identifies these colors in cache keys.
    pub fn key(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        let colors = [
            self.surface,
            self.empty_side,
            self.hatch,
            self.text,
            self.muted,
            self.border,
            self.gutter,
            self.gutter_text,
            self.add_line,
            self.del_line,
            self.add_word,
            self.del_word,
            self.add_marker,
            self.del_marker,
            self.add_number,
            self.del_number,
            self.add_text,
            self.del_text,
            self.file_header,
            self.separator,
            self.hover,
            self.search_match,
            self.search_active,
            self.search_outline,
            self.selection,
            self.focused_row,
            self.accent,
            self.warning,
            self.strong,
            self.faint,
            self.picked_wash,
            self.picked_gutter,
            self.picked_number,
        ];
        for c in colors.iter().chain(&self.syntax) {
            [c.r, c.g, c.b, c.a].hash(&mut h);
        }
        h.finish()
    }

    pub fn tone(&self, kind: HighlightKind) -> Color {
        self.syntax[kind as usize]
    }

    /// Syntax colors for text on `bg`, a line's background.
    pub fn tones_on(&self, bg: Option<Color>) -> &[Color; HIGHLIGHT_KINDS] {
        match bg {
            Some(bg) if bg == self.add_line => &self.syntax_on_add,
            Some(bg) if bg == self.del_line => &self.syntax_on_del,
            _ => &self.syntax,
        }
    }

    pub fn tint(base: Color, tone: Color) -> Color {
        base.lerp(tone, TINT)
    }

    /// Background of a line of `kind` on `side`, if it has one.
    pub fn line(&self, kind: RowKind, side: Side) -> Option<Color> {
        match (kind, side) {
            (RowKind::Removed, _) | (RowKind::Modified, Side::Old) => Some(self.del_line),
            (RowKind::Added, _) | (RowKind::Modified, Side::New) => Some(self.add_line),
            _ => None,
        }
    }
}

/// Least contrast a syntax color keeps against a changed line's
/// background.
const MIN_TONE_CONTRAST: f32 = 3.0;
const LEGIBLE_STEPS: u8 = 10;

/// `bg` composited over `surface`.
fn opaque(bg: Color, surface: Color) -> Color {
    surface.lerp(bg.with_alpha(255), f32::from(bg.a) / 255.0)
}

/// `tone`, moved toward `text` just far enough to reach
/// [`MIN_TONE_CONTRAST`] on `bg`.
fn legible(tone: Color, bg: Color, text: Color) -> Color {
    (0..=LEGIBLE_STEPS)
        .map(|step| tone.lerp(text, f32::from(step) / f32::from(LEGIBLE_STEPS)))
        .find(|color| contrast_ratio(*color, bg) >= MIN_TONE_CONTRAST)
        .unwrap_or(text)
}

const TINT: f32 = 0.14;
const PICKED_WASH: f32 = 0.09;
const SEPARATOR_LIFT: f32 = 0.035;
const NUMBER_TINT: f32 = 0.45;
const PICKED_GUTTER: f32 = 0.16;
const PICKED_NUMBER: f32 = 0.6;

const ALL_KINDS: [HighlightKind; HIGHLIGHT_KINDS] = [
    HighlightKind::Normal,
    HighlightKind::Keyword,
    HighlightKind::String,
    HighlightKind::Comment,
    HighlightKind::Number,
    HighlightKind::Type,
    HighlightKind::Function,
    HighlightKind::Operator,
    HighlightKind::Punctuation,
    HighlightKind::Variable,
    HighlightKind::Constant,
    HighlightKind::Builtin,
    HighlightKind::Attribute,
    HighlightKind::Tag,
    HighlightKind::Property,
    HighlightKind::Namespace,
    HighlightKind::Label,
    HighlightKind::Preprocessor,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faint_tones_are_lifted_to_stay_legible_on_a_changed_line() {
        let bg = Color::rgba(44, 60, 46, 255);
        let text = Color::rgba(220, 220, 220, 255);
        let faint = Color::rgba(86, 95, 137, 255);
        let lifted = legible(faint, bg, text);
        assert!(contrast_ratio(faint, bg) < MIN_TONE_CONTRAST);
        assert!(contrast_ratio(lifted, bg) >= MIN_TONE_CONTRAST);
        assert_eq!(legible(text, bg, text), text);
    }
}
