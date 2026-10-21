use std::ops::Range;

use diffy_diff::Side;
use diffy_syntax::{Symbol, SymbolKind};
use quark::view;
use quark_app::ViewContext;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::ViewportController;
use crate::app::shell::empty_state;
use crate::app::tokens::{self, Layout, Sp, TextStyle};
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::Action;
use crate::contracts::viewport::files::head_revision;
use crate::contracts::viewport::symbols::{DefinitionTarget, SymbolsAction};

const MAX_ROWS: usize = 5_000;

pub fn kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "fn",
        SymbolKind::Method => "method",
        SymbolKind::Class => "type",
        SymbolKind::Interface => "trait",
        SymbolKind::Module => "mod",
        SymbolKind::Macro => "macro",
        SymbolKind::Constant => "const",
        SymbolKind::Type => "type",
        SymbolKind::Field => "field",
        SymbolKind::Other => "def",
    }
}

fn depths(symbols: &[Symbol]) -> Vec<usize> {
    let mut open: Vec<u32> = Vec::new();
    symbols
        .iter()
        .map(|symbol| {
            while open.last().is_some_and(|end| *end < symbol.start_line) {
                open.pop();
            }
            let depth = open.len();
            if symbol.end_line > symbol.start_line {
                open.push(symbol.end_line);
            }
            depth
        })
        .collect()
}

fn touched(symbol: &Symbol, changed: &[Range<u32>]) -> bool {
    changed
        .iter()
        .any(|range| range.start <= symbol.end_line && range.end > symbol.start_line)
}

pub fn view(
    viewport: &ViewportController,
    shared: &SharedContext,
    width: f32,
    height: f32,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let Some(document) = shared.document.as_ref() else {
        return empty_state("No file", "Open a file to see its outline", theme);
    };
    let head = head_revision(shared).map(|(_, head)| head);
    if head.as_ref() != Some(&document.revision) {
        return empty_state(
            "Outline unavailable",
            "Outlines cover files at the compared revision",
            theme,
        );
    }
    let Some(index) = shared.symbols.as_ref() else {
        return empty_state("Indexing symbols", &document.path, theme);
    };
    let Some(file) = index.file(&document.path) else {
        return empty_state("No symbols", &document.path, theme);
    };
    let changed = shared
        .manifest
        .as_deref()
        .and_then(|manifest| {
            manifest
                .files
                .iter()
                .find(|file| file.paths.new_path().as_deref() == Some(document.path.as_str()))
        })
        .and_then(|file| {
            viewport
                .interaction
                .diff()?
                .changed_lines(file.key, Side::New)
        })
        .unwrap_or_default();
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let mono = TextStyle::MonoSmall.font_size(theme);
    let pad = tokens::scaled(Sp::SM, scale);
    let indent = tokens::scaled(Sp::MD, scale);
    let dot = tokens::scaled(Layout::STATUS_DOT, scale);
    let symbols = &file.symbols[..file.symbols.len().min(MAX_ROWS)];
    let rows: Vec<AnyElement> = symbols
        .iter()
        .zip(depths(symbols))
        .enumerate()
        .map(|(i, (symbol, depth))| {
            let target = DefinitionTarget {
                path: document.path.clone(),
                revision: document.revision.clone(),
                line: symbol.line,
                column: symbol.column,
                len: symbol.name.len() as u32,
                kind: Some(symbol.kind),
            };
            let kind = kind_label(symbol.kind);
            let changed = touched(symbol, &changed);
            let label = if changed {
                format!("{kind} {}, changed, line {}", symbol.name, symbol.line + 1)
            } else {
                format!("{kind} {}, line {}", symbol.name, symbol.line + 1)
            };
            view! {
                <div
                    class="flex-row items-center w-full shrink-0"
                    h={theme.metrics.ui_row_height.round()}
                    pl={pad + indent * depth as f32}
                    pr={pad}
                    gap={tokens::scaled(Sp::XS, scale)}
                    rounded={theme.metrics.control_radius}
                    hover_bg={tc.sidebar_row_hover}
                    class="cursor-pointer"
                    key={format!("outline:{i}")}
                    accessibility_role={AccessibilityRole::ListBoxOption}
                    aria-label={label}
                    test_id={format!("outline.{}", symbol.name)}
                    on:click={Action::from(SymbolsAction::Jump(target))}
                >
                    <text size={mono} class="font-mono" color={tc.syntax_keyword}>{kind}</text>
                    <div class="flex-1 flex-row items-center overflow-hidden" min_w={0.0}>
                        <text size={mono} class="font-mono truncate" color={tc.text}>{symbol.name.to_string()}</text>
                    </div>
                    if changed {
                        <div class="shrink-0" w={dot} h={dot} rounded={dot} bg={tc.status_warning} />
                    }
                </div>
            }
        })
        .collect();
    view! {
        <div
            w={width}
            h={height}
            class="flex-col overflow-y-scroll scrollbar-auto-hide"
            px={tokens::scaled(Sp::XS, scale)}
            track_scroll={viewport.symbols.outline_scroll()}
            accessibility_role={AccessibilityRole::ListBox}
            aria-label={format!("Outline of {}", document.path)}
            test_id="outline"
        >
            {...rows}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(name: &str, start_line: u32, end_line: u32) -> Symbol {
        Symbol {
            name: Box::from(name),
            kind: SymbolKind::Function,
            line: start_line,
            column: 0,
            start_line,
            end_line,
        }
    }

    #[test]
    fn nesting_follows_line_spans() {
        let symbols = [
            symbol("Store", 0, 10),
            symbol("open", 1, 4),
            symbol("close", 5, 9),
            symbol("main", 12, 14),
        ];
        assert_eq!(depths(&symbols), vec![0, 1, 1, 0]);
        let changed = vec![3..4, 20..22];
        assert!(touched(&symbols[1], &changed));
        assert!(!touched(&symbols[3], &changed));
    }
}
