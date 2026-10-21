use std::time::Duration;

use diffy_ui::DiffEvent;
use diffy_ui::diff_view::decorator::DiffViewOptions;
use diffy_ui::diff_view::diff_view_custom;
use quark::view;
use quark_app::ViewContext;
use quark_components::{CollectionEnv, TabItem, tab_bar};
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;

use super::state::{FileTab, FilesController, TabState};
use crate::app::commands::keymap::WORKSPACE_CONTEXT;
use crate::app::shell::empty_state;
use crate::app::text_targets::VIEWPORT;
use crate::app::tokens::{self, Sz};
use crate::app::viewport::interaction::keys;
use crate::contracts::shell::Action;
use crate::contracts::viewport::files::FilesAction;
use crate::contracts::viewport::interaction::DiffInput;

const SYNTAX_POLL: Duration = Duration::from_millis(50);

fn input(event: DiffEvent) -> quark_ui::Action {
    Action::from(FilesAction::Input(DiffInput(event))).into()
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn tab_strip(files: &FilesController, width: f32, cx: &ViewContext) -> Option<AnyElement> {
    if files.tabs().is_empty() {
        return None;
    }
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let mut items = vec![
        TabItem::new("Diff", Action::from(FilesAction::ShowDiff))
            .icon(lucide::GIT_COMPARE)
            .id("viewport.diff")
            .active(files.active().is_none()),
    ];
    items.extend(files.tabs().iter().map(|tab| {
        TabItem::new(
            format!("{} @ {}", file_name(&tab.file.path), tab.file.label),
            Action::from(FilesAction::Activate(tab.id)),
        )
        .icon(lucide::FILE)
        .id(format!("viewport.file.{}", tab.id.0))
        .active(files.active() == Some(tab.id))
        .on_close(Action::from(FilesAction::Close(tab.id)))
    }));
    Some(view! {
        <div
            w={width}
            h={tokens::scaled(Sz::SIDEBAR_LIST_OFFSET, scale)}
            class="flex-row items-end shrink-0 overflow-hidden"
            test_id="viewport.tabs"
        >
            {tab_bar(items).label("Open files").full_width()}
        </div>
    })
}

pub fn file_view(
    files: &mut FilesController,
    width: f32,
    height: f32,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let theme = cx.theme;
    let tab: &mut FileTab = files.active_tab_mut()?;
    let path = tab.file.path.clone();
    if let TabState::Ready(state) = &mut tab.state {
        state.set_viewport(width, height);
    }
    tab.apply_reveal();
    let view = match &mut tab.state {
        TabState::Loading => empty_state("Loading file", &path, theme),
        TabState::Failed(message) => empty_state("Couldn't open file", message, theme),
        TabState::Ready(state) => {
            let scale_factor = cx.frame.scale_factor();
            let now_ms = cx.frame.elapsed().as_millis() as u64;
            let text = cx.frame.text();
            state.prepare(&mut text.system, &mut text.layouts, scale_factor, now_ms);
            if state.syntax_pending() {
                cx.frame.request_frame_in(SYNTAX_POLL);
            }
            let env = CollectionEnv {
                focused: cx.is_focused(VIEWPORT),
                accessible: cx.frame.accessibility_active(),
            };
            let options = DiffViewOptions {
                keys: Some(keys::DIFF),
                ..DiffViewOptions::default()
            };
            diff_view_custom(state, cx.theme, env, input, options)
        }
    };
    Some(view! {
        <div
            w={width}
            h={height}
            class="overflow-hidden"
            key_context={WORKSPACE_CONTEXT}
            on:key:escape={Action::from(FilesAction::ShowDiff)}
            aria-label={format!("File {path}")}
            test_id="viewport.file"
        >
            {view}
        </div>
    })
}
